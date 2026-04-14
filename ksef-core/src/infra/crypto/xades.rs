use std::fmt::Write;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use openssl::sign::Signer;
use openssl::x509::X509;

use crate::domain::auth::AuthChallenge;
use crate::domain::certificate::CertificateKind;
use crate::domain::crypto::SignedAuthRequest;
use crate::domain::nip::Nip;
use crate::error::CryptoError;
use crate::ports::encryption::XadesSigner;

const AUTH_NS: &str = "http://ksef.mf.gov.pl/auth/token/2.0";
const DSIG_NS: &str = "http://www.w3.org/2000/09/xmldsig#";
const XADES_NS: &str = "http://uri.etsi.org/01903/v1.3.2#";

/// `XAdES`-BES signer using OpenSSL for crypto and `bergshamra-c14n` for
/// Exclusive XML Canonicalization.
pub struct OpenSslXadesSigner {
    private_key_pem: Vec<u8>,
    certificate_pem: Vec<u8>,
}

impl OpenSslXadesSigner {
    /// Create from existing PEM key + certificate.
    #[must_use]
    pub fn from_pem(private_key_pem: Vec<u8>, certificate_pem: Vec<u8>) -> Self {
        Self {
            private_key_pem,
            certificate_pem,
        }
    }

    /// Generate a self-signed test certificate for the `KSeF` sandbox.
    ///
    /// The `CertificateKind` determines which X.509 field carries the NIP:
    /// - `Seal` → `organizationIdentifier` (OID 2.5.4.97) = `VATPL-{NIP}`
    /// - `Personal` → `serialNumber` (OID 2.5.4.5) = `TINPL-{NIP}`
    ///
    /// See `domain::certificate::CertificateKind` for protocol documentation.
    pub fn generate_for_nip(nip: &Nip, kind: CertificateKind) -> Result<Self, CryptoError> {
        let nip_value = kind.format_nip(nip);
        match kind {
            CertificateKind::Seal => Self::generate_cert_with_dn(|name| {
                name.append_entry_by_nid(Nid::ORGANIZATIONNAME, "ksef-paymoney")?;
                name.append_entry_by_text(kind.field_name(), &nip_value)?;
                name.append_entry_by_nid(Nid::COMMONNAME, "ksef-paymoney seal")?;
                name.append_entry_by_nid(Nid::COUNTRYNAME, "PL")?;
                Ok(())
            }),
            CertificateKind::Personal => Self::generate_cert_with_dn(|name| {
                name.append_entry_by_nid(Nid::GIVENNAME, "Test")?;
                name.append_entry_by_nid(Nid::SURNAME, "User")?;
                name.append_entry_by_nid(Nid::SERIALNUMBER, &nip_value)?;
                name.append_entry_by_nid(Nid::COMMONNAME, "Test User")?;
                name.append_entry_by_nid(Nid::COUNTRYNAME, "PL")?;
                Ok(())
            }),
        }
    }

    /// Shortcut: Seal certificate — the default for automated systems.
    pub fn generate_self_signed_for_nip(nip: &Nip) -> Result<Self, CryptoError> {
        Self::generate_for_nip(nip, CertificateKind::Seal)
    }

    /// Shortcut: Seal certificate with default test NIP.
    pub fn generate_self_signed() -> Result<Self, CryptoError> {
        let nip = Nip::parse("5260250274").expect("hardcoded test NIP");
        Self::generate_for_nip(&nip, CertificateKind::Seal)
    }

    fn generate_cert_with_dn(
        build_name: impl FnOnce(
            &mut openssl::x509::X509NameBuilder,
        ) -> Result<(), openssl::error::ErrorStack>,
    ) -> Result<Self, CryptoError> {
        let rsa = Rsa::generate(2048)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        let pkey = PKey::from_rsa(rsa)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;

        let mut builder =
            X509::builder().map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        builder
            .set_version(2)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;

        let mut name_builder = openssl::x509::X509NameBuilder::new()
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        build_name(&mut name_builder)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        let name = name_builder.build();

        builder
            .set_subject_name(&name)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        builder
            .set_issuer_name(&name)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        builder
            .set_pubkey(&pkey)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;

        let not_before =
            openssl::asn1::Asn1Time::from_unix((Utc::now() - Duration::minutes(61)).timestamp())
                .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        let not_after =
            openssl::asn1::Asn1Time::from_unix((Utc::now() + Duration::days(730)).timestamp())
                .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        builder
            .set_not_before(&not_before)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        builder
            .set_not_after(&not_after)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;

        let serial = openssl::bn::BigNum::from_u32(1)
            .and_then(|bn| bn.to_asn1_integer())
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;
        builder
            .set_serial_number(&serial)
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;

        builder
            .sign(&pkey, MessageDigest::sha256())
            .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?;

        let cert = builder.build();

        Ok(Self {
            private_key_pem: pkey
                .private_key_to_pem_pkcs8()
                .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?,
            certificate_pem: cert
                .to_pem()
                .map_err(|e| CryptoError::CertificateGenerationFailed(e.to_string()))?,
        })
    }
}

#[async_trait]
impl XadesSigner for OpenSslXadesSigner {
    async fn sign_auth_request(
        &self,
        challenge: &AuthChallenge,
        nip: &Nip,
    ) -> Result<SignedAuthRequest, CryptoError> {
        sign_auth_request_sync(&self.private_key_pem, &self.certificate_pem, challenge, nip)
    }
}

// =========================================================================
// XML Exclusive Canonicalization
// =========================================================================

/// Canonicalize XML using Exclusive C14N (no comments).
fn exc_c14n(xml: &str) -> Result<Vec<u8>, CryptoError> {
    let doc = uppsala::parse(xml)
        .map_err(|e| CryptoError::XadesSigningFailed(format!("parse XML for c14n: {e}")))?;
    let empty: &[String] = &[];
    bergshamra_c14n::exclusive::canonicalize(&doc, false, None, empty)
        .map_err(|e| CryptoError::XadesSigningFailed(format!("exc-c14n: {e}")))
}

// =========================================================================
// Signing logic
// =========================================================================

fn sign_auth_request_sync(
    private_key_pem: &[u8],
    certificate_pem: &[u8],
    challenge: &AuthChallenge,
    nip: &Nip,
) -> Result<SignedAuthRequest, CryptoError> {
    let pkey = PKey::private_key_from_pem(private_key_pem)
        .map_err(|e| CryptoError::XadesSigningFailed(format!("load private key: {e}")))?;
    let cert = X509::from_pem(certificate_pem)
        .map_err(|e| CryptoError::XadesSigningFailed(format!("load certificate: {e}")))?;

    let cert_der = cert
        .to_der()
        .map_err(|e| CryptoError::XadesSigningFailed(e.to_string()))?;
    let cert_digest = sha256_base64(&cert_der);
    let cert_b64 = base64_encode(&cert_der);
    let signing_time = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Step 1: Build the complete document with a PLACEHOLDER signature.
    // This ensures the non-Signature content is byte-identical in both
    // the "document for digest" and the final output.
    let placeholder = "PLACEHOLDER_SIGNATURE_ELEMENT";
    let doc_with_placeholder = build_document_with_marker(nip, &challenge.challenge, placeholder);

    // Step 2: Compute document digest.
    // For enveloped-signature, the digest is of the document with Signature removed.
    // Since our placeholder is a text node (not an element), we extract the part
    // before and after the placeholder — that's the document sans Signature.
    let doc_without_sig = doc_with_placeholder.replace(placeholder, "");
    let c14n_doc = exc_c14n(&doc_without_sig)?;
    let doc_digest = sha256_base64(&c14n_doc);

    // Step 3: Build SignedProperties.
    // For the digest, we must C14N SignedProperties with its own namespace
    // declarations included (as it would appear in the final document).
    // In exclusive C14N, each element carries its own visibly-utilized namespaces,
    // so canonicalizing the fragment standalone IS correct — but we must ensure
    // the namespace declarations are present ON the element itself.
    let signed_properties = build_signed_properties(&signing_time, &cert_digest);
    // SignedProperties already has xmlns:xades and xmlns:ds on the element,
    // so exc-c14n standalone is correct.
    let canonicalized_signed_properties =
        exc_c14n(&format!(r#"<?xml version="1.0"?>{signed_properties}"#))?;
    let sp_digest = sha256_base64(&canonicalized_signed_properties);

    // Step 4: Build and canonicalize SignedInfo, sign it.
    // Same logic: SignedInfo has xmlns:ds on itself.
    let signed_info = build_signed_info(&doc_digest, &sp_digest);
    let canonicalized_signed_info = exc_c14n(&format!(r#"<?xml version="1.0"?>{signed_info}"#))?;

    let mut signer = Signer::new(MessageDigest::sha256(), &pkey)
        .map_err(|e| CryptoError::XadesSigningFailed(e.to_string()))?;
    signer
        .update(&canonicalized_signed_info)
        .map_err(|e| CryptoError::XadesSigningFailed(e.to_string()))?;
    let signature_bytes = signer
        .sign_to_vec()
        .map_err(|e| CryptoError::XadesSigningFailed(e.to_string()))?;
    let signature_b64 = base64_encode(&signature_bytes);

    // Step 5: Build the complete Signature element and insert it into the document.
    let signature_element =
        build_signature_element(&signed_info, &signature_b64, &cert_b64, &signed_properties);
    let final_xml = doc_with_placeholder.replace(placeholder, &signature_element);

    Ok(SignedAuthRequest::new(final_xml.into_bytes()))
}

// =========================================================================
// XML builders
// =========================================================================

/// Build the document with a text marker where the Signature element will go.
fn build_document_with_marker(nip: &Nip, challenge: &str, marker: &str) -> String {
    let mut xml = String::new();
    write!(
        xml,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#
    )
    .unwrap();
    write!(xml, r#"<AuthTokenRequest xmlns="{AUTH_NS}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">"#).unwrap();
    write!(xml, "<Challenge>{challenge}</Challenge>").unwrap();
    write!(xml, "<ContextIdentifier>").unwrap();
    write!(xml, "<Nip>{nip}</Nip>").unwrap();
    write!(xml, "</ContextIdentifier>").unwrap();
    write!(
        xml,
        "<SubjectIdentifierType>certificateSubject</SubjectIdentifierType>"
    )
    .unwrap();
    write!(xml, "{marker}").unwrap();
    write!(xml, "</AuthTokenRequest>").unwrap();
    xml
}

#[cfg(test)]
fn build_auth_token_request(nip: &Nip, challenge: &str) -> String {
    build_document_with_marker(nip, challenge, "")
}

fn build_signed_properties(signing_time: &str, cert_digest: &str) -> String {
    let mut sp = String::new();
    write!(sp, r#"<xades:SignedProperties xmlns:xades="{XADES_NS}" xmlns:ds="{DSIG_NS}" Id="SignedProperties-1">"#).unwrap();
    write!(sp, "<xades:SignedSignatureProperties>").unwrap();
    write!(sp, "<xades:SigningTime>{signing_time}</xades:SigningTime>").unwrap();
    write!(sp, "<xades:SigningCertificateV2>").unwrap();
    write!(sp, "<xades:Cert>").unwrap();
    write!(sp, "<xades:CertDigest>").unwrap();
    write!(
        sp,
        r#"<ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>"#
    )
    .unwrap();
    write!(sp, "<ds:DigestValue>{cert_digest}</ds:DigestValue>").unwrap();
    write!(sp, "</xades:CertDigest>").unwrap();
    write!(sp, "</xades:Cert>").unwrap();
    write!(sp, "</xades:SigningCertificateV2>").unwrap();
    write!(sp, "</xades:SignedSignatureProperties>").unwrap();
    write!(sp, "</xades:SignedProperties>").unwrap();
    sp
}

fn build_signed_info(doc_digest: &str, sp_digest: &str) -> String {
    let mut si = String::new();
    write!(si, r#"<ds:SignedInfo xmlns:ds="{DSIG_NS}">"#).unwrap();
    write!(
        si,
        r#"<ds:CanonicalizationMethod Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/>"#
    )
    .unwrap();
    write!(
        si,
        r#"<ds:SignatureMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/>"#
    )
    .unwrap();
    write!(si, r#"<ds:Reference URI="">"#).unwrap();
    write!(si, "<ds:Transforms>").unwrap();
    write!(
        si,
        r#"<ds:Transform Algorithm="http://www.w3.org/2000/09/xmldsig#enveloped-signature"/>"#
    )
    .unwrap();
    write!(
        si,
        r#"<ds:Transform Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/>"#
    )
    .unwrap();
    write!(si, "</ds:Transforms>").unwrap();
    write!(
        si,
        r#"<ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>"#
    )
    .unwrap();
    write!(si, "<ds:DigestValue>{doc_digest}</ds:DigestValue>").unwrap();
    write!(si, "</ds:Reference>").unwrap();
    write!(si, "<ds:Reference URI=\"#SignedProperties-1\" Type=\"http://uri.etsi.org/01903#SignedProperties\">").unwrap();
    write!(si, "<ds:Transforms>").unwrap();
    write!(
        si,
        r#"<ds:Transform Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/>"#
    )
    .unwrap();
    write!(si, "</ds:Transforms>").unwrap();
    write!(
        si,
        r#"<ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>"#
    )
    .unwrap();
    write!(si, "<ds:DigestValue>{sp_digest}</ds:DigestValue>").unwrap();
    write!(si, "</ds:Reference>").unwrap();
    write!(si, "</ds:SignedInfo>").unwrap();
    si
}

/// Build the `<ds:Signature>` element to be inserted into the document.
fn build_signature_element(
    signed_info: &str,
    signature_value: &str,
    cert_b64: &str,
    signed_properties: &str,
) -> String {
    let mut sig = String::new();
    write!(
        sig,
        r#"<ds:Signature xmlns:ds="{DSIG_NS}" Id="Signature-1">"#
    )
    .unwrap();
    write!(sig, "{signed_info}").unwrap();
    write!(
        sig,
        "<ds:SignatureValue>{signature_value}</ds:SignatureValue>"
    )
    .unwrap();
    write!(sig, "<ds:KeyInfo>").unwrap();
    write!(sig, "<ds:X509Data>").unwrap();
    write!(sig, "<ds:X509Certificate>{cert_b64}</ds:X509Certificate>").unwrap();
    write!(sig, "</ds:X509Data>").unwrap();
    write!(sig, "</ds:KeyInfo>").unwrap();
    write!(sig, "<ds:Object>").unwrap();
    write!(
        sig,
        "<xades:QualifyingProperties xmlns:xades=\"{XADES_NS}\" Target=\"#Signature-1\">"
    )
    .unwrap();
    write!(sig, "{signed_properties}").unwrap();
    write!(sig, "</xades:QualifyingProperties>").unwrap();
    write!(sig, "</ds:Object>").unwrap();
    write!(sig, "</ds:Signature>").unwrap();
    sig
}

// =========================================================================
// Helpers
// =========================================================================

fn sha256_base64(data: &[u8]) -> String {
    let digest = openssl::hash::hash(MessageDigest::sha256(), data).unwrap();
    base64_encode(&digest)
}

fn base64_encode(data: &[u8]) -> String {
    openssl::base64::encode_block(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_challenge() -> AuthChallenge {
        AuthChallenge {
            timestamp: "2026-04-13T10:00:00Z".to_string(),
            challenge: "20260413-CR-XXXXXXXXXX-YYYYYYYY-ZZ".to_string(),
        }
    }

    fn test_nip() -> Nip {
        Nip::parse("5260250274").unwrap()
    }

    #[test]
    fn generate_self_signed_succeeds() {
        let signer = OpenSslXadesSigner::generate_self_signed().unwrap();
        assert!(!signer.private_key_pem.is_empty());
        assert!(!signer.certificate_pem.is_empty());
    }

    #[test]
    fn build_auth_token_request_has_correct_structure() {
        let xml = build_auth_token_request(&test_nip(), "test-challenge");
        assert!(xml.contains(&format!(r#"xmlns="{AUTH_NS}""#)));
        assert!(xml.contains("<Nip>5260250274</Nip>"));
        assert!(xml.contains("<Challenge>test-challenge</Challenge>"));
        assert!(xml.contains("<SubjectIdentifierType>certificateSubject</SubjectIdentifierType>"));
    }

    #[test]
    fn exc_c14n_produces_deterministic_output() {
        let xml = build_auth_token_request(&test_nip(), "test-challenge");
        let a = exc_c14n(&xml).unwrap();
        let b = exc_c14n(&xml).unwrap();
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn exc_c14n_removes_xml_declaration() {
        let xml = build_auth_token_request(&test_nip(), "test-challenge");
        let c14n = exc_c14n(&xml).unwrap();
        let s = std::str::from_utf8(&c14n).unwrap();
        // C14N spec: XML declaration is not output
        assert!(!s.contains("<?xml"));
        // Content is preserved
        assert!(s.contains("<Challenge>test-challenge</Challenge>"));
    }

    #[tokio::test]
    async fn sign_auth_request_produces_valid_xades() {
        let signer = OpenSslXadesSigner::generate_self_signed().unwrap();
        let challenge = test_challenge();
        let nip = test_nip();

        let signed = signer.sign_auth_request(&challenge, &nip).await.unwrap();
        let xml = std::str::from_utf8(signed.as_bytes()).unwrap();

        assert!(xml.contains("<AuthTokenRequest"));
        assert!(xml.contains("<ds:Signature"));
        assert!(xml.contains("<ds:SignedInfo"));
        assert!(xml.contains("<ds:SignatureValue>"));
        assert!(xml.contains("<ds:X509Certificate>"));
        assert!(xml.contains("<xades:SignedProperties"));
        assert!(xml.contains("<xades:SigningTime>"));
        assert!(xml.contains("<xades:SigningCertificateV2>"));
        assert!(xml.contains("<Nip>5260250274</Nip>"));
        assert!(xml.contains("rsa-sha256"));
        assert!(xml.contains("xml-exc-c14n#"));
        assert!(xml.contains("enveloped-signature"));
    }

    #[tokio::test]
    async fn sign_with_different_challenges_produces_different_output() {
        let signer = OpenSslXadesSigner::generate_self_signed().unwrap();
        let nip = test_nip();

        let a = signer
            .sign_auth_request(
                &AuthChallenge {
                    timestamp: "2026-04-13T10:00:00Z".to_string(),
                    challenge: "challenge-AAA".to_string(),
                },
                &nip,
            )
            .await
            .unwrap();

        let b = signer
            .sign_auth_request(
                &AuthChallenge {
                    timestamp: "2026-04-13T10:00:00Z".to_string(),
                    challenge: "challenge-BBB".to_string(),
                },
                &nip,
            )
            .await
            .unwrap();

        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn sha256_base64_is_deterministic() {
        let a = sha256_base64(b"test data");
        let b = sha256_base64(b"test data");
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }
}
