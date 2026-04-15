use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;

use crate::domain::auth::{
    AccessToken, AuthChallenge, AuthReference, AuthStatus, ContextIdentifier, RefreshToken,
    TokenPair,
};
use crate::domain::crypto::{EncryptedInvoice, KSeFPublicKey, SignedAuthRequest};
use crate::domain::nip::Nip;
use crate::domain::session::{InvoiceMetadata, InvoiceQuery, KSeFNumber, SessionReference, Upo};
use crate::domain::xml::{InvoiceXml, UntrustedInvoiceXml};
use crate::error::{CryptoError, KSeFError};
use crate::ports::encryption::{InvoiceEncryptor, XadesSigner};
use crate::ports::ksef_auth::KSeFAuth;
use crate::ports::ksef_client::KSeFClient;

/// Mock `KSeFAuth` that always succeeds.
pub struct MockKSeFAuth {
    pub challenge_count: Mutex<u32>,
    pub redeem_count: Mutex<u32>,
    pub token_auth_count: Mutex<u32>,
    poll_statuses: Mutex<Vec<AuthStatus>>,
}

impl MockKSeFAuth {
    #[must_use]
    pub fn new() -> Self {
        Self {
            challenge_count: Mutex::new(0),
            redeem_count: Mutex::new(0),
            token_auth_count: Mutex::new(0),
            poll_statuses: Mutex::new(vec![AuthStatus::Completed]),
        }
    }

    pub fn set_poll_statuses(&self, statuses: Vec<AuthStatus>) {
        *self.poll_statuses.lock().unwrap() = statuses;
    }
}

impl Default for MockKSeFAuth {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl KSeFAuth for MockKSeFAuth {
    async fn request_challenge(&self, _nip: &Nip) -> Result<AuthChallenge, KSeFError> {
        *self.challenge_count.lock().unwrap() += 1;
        Ok(AuthChallenge {
            timestamp: Utc::now().to_rfc3339(),
            challenge: "mock-challenge-value".to_string(),
        })
    }

    async fn authenticate_xades(
        &self,
        _signed_request: &SignedAuthRequest,
    ) -> Result<AuthReference, KSeFError> {
        Ok(AuthReference::new(
            "mock-auth-ref".to_string(),
            "mock-auth-op-token".to_string(),
        ))
    }

    async fn authenticate_token(
        &self,
        _context: &ContextIdentifier,
        _token: &str,
    ) -> Result<AuthReference, KSeFError> {
        *self.token_auth_count.lock().unwrap() += 1;
        Ok(AuthReference::new(
            "mock-token-auth-ref".to_string(),
            "mock-token-auth-op-token".to_string(),
        ))
    }

    async fn poll_auth_status(&self, _reference: &AuthReference) -> Result<AuthStatus, KSeFError> {
        let mut statuses = self.poll_statuses.lock().unwrap();
        if statuses.is_empty() {
            return Ok(AuthStatus::Completed);
        }
        Ok(statuses.remove(0))
    }

    async fn redeem_token(&self, _reference: &AuthReference) -> Result<TokenPair, KSeFError> {
        *self.redeem_count.lock().unwrap() += 1;
        Ok(TokenPair {
            access_token: AccessToken::new("mock-access-token".to_string()),
            refresh_token: RefreshToken::new("mock-refresh-token".to_string()),
            access_token_expires_at: Utc::now() + chrono::Duration::minutes(15),
            refresh_token_expires_at: Utc::now() + chrono::Duration::days(7),
        })
    }

    async fn refresh_token(&self, _refresh_token: &RefreshToken) -> Result<TokenPair, KSeFError> {
        Ok(TokenPair {
            access_token: AccessToken::new("mock-refreshed-token".to_string()),
            refresh_token: RefreshToken::new("mock-refresh-token-2".to_string()),
            access_token_expires_at: Utc::now() + chrono::Duration::minutes(15),
            refresh_token_expires_at: Utc::now() + chrono::Duration::days(7),
        })
    }
}

/// Mock `XadesSigner` that returns dummy signed data.
pub struct MockXadesSigner;

#[async_trait]
impl XadesSigner for MockXadesSigner {
    async fn sign_auth_request(
        &self,
        _challenge: &AuthChallenge,
        _nip: &Nip,
    ) -> Result<SignedAuthRequest, CryptoError> {
        Ok(SignedAuthRequest::new(b"<mock-signed-xml/>".to_vec()))
    }
}

/// Mock `KSeFClient` for session/invoice operations.
pub struct MockKSeFClient {
    pub invoices_sent: Mutex<u32>,
    pub query_count: Mutex<u32>,
    query_results: Mutex<Vec<InvoiceMetadata>>,
    query_errors: Mutex<Vec<KSeFError>>,
    send_errors: Mutex<Vec<KSeFError>>,
    fetch_xml: Mutex<Option<UntrustedInvoiceXml>>,
}

impl MockKSeFClient {
    #[must_use]
    pub fn new() -> Self {
        Self {
            invoices_sent: Mutex::new(0),
            query_count: Mutex::new(0),
            query_results: Mutex::new(Vec::new()),
            query_errors: Mutex::new(Vec::new()),
            send_errors: Mutex::new(Vec::new()),
            fetch_xml: Mutex::new(None),
        }
    }

    /// Set the results that `query_invoices` will return.
    pub fn set_query_results(&self, results: Vec<InvoiceMetadata>) {
        *self.query_results.lock().unwrap() = results;
    }

    /// Set a queue of errors returned by `query_invoices` before successful results.
    pub fn set_query_errors(&self, errors: Vec<KSeFError>) {
        *self.query_errors.lock().unwrap() = errors;
    }

    /// Set a queue of errors returned by `send_invoice` before successful responses.
    pub fn set_send_errors(&self, errors: Vec<KSeFError>) {
        *self.send_errors.lock().unwrap() = errors;
    }

    /// Set the XML that `fetch_invoice` will return for any invoice.
    pub fn set_fetch_xml(&self, xml: InvoiceXml) {
        *self.fetch_xml.lock().unwrap() = Some(xml.into());
    }

    /// Set raw untrusted XML that bypasses trusted constructors.
    pub fn set_fetch_xml_untrusted(&self, xml: String) {
        *self.fetch_xml.lock().unwrap() = Some(UntrustedInvoiceXml::new(xml));
    }
}

impl Default for MockKSeFClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl KSeFClient for MockKSeFClient {
    async fn open_session(
        &self,
        _access_token: &AccessToken,
        _session_encryption: &EncryptedInvoice,
    ) -> Result<SessionReference, KSeFError> {
        Ok(SessionReference::new("mock-session-ref".to_string()))
    }

    async fn send_invoice(
        &self,
        _access_token: &AccessToken,
        _session: &SessionReference,
        _encrypted_invoice: &EncryptedInvoice,
    ) -> Result<KSeFNumber, KSeFError> {
        let mut send_errors = self.send_errors.lock().unwrap();
        if !send_errors.is_empty() {
            return Err(send_errors.remove(0));
        }

        let mut count = self.invoices_sent.lock().unwrap();
        *count += 1;
        Ok(KSeFNumber::new(format!("KSeF-MOCK-{count}")))
    }

    async fn close_session(
        &self,
        _access_token: &AccessToken,
        _session: &SessionReference,
    ) -> Result<Upo, KSeFError> {
        Ok(Upo {
            reference: "mock-upo-ref".to_string(),
            content: b"<UPO>mock</UPO>".to_vec(),
        })
    }

    async fn get_upo(
        &self,
        _access_token: &AccessToken,
        _session: &SessionReference,
    ) -> Result<Upo, KSeFError> {
        Ok(Upo {
            reference: "mock-upo-ref".to_string(),
            content: b"<UPO>mock</UPO>".to_vec(),
        })
    }

    async fn fetch_invoice(
        &self,
        _access_token: &AccessToken,
        ksef_number: &KSeFNumber,
    ) -> Result<UntrustedInvoiceXml, KSeFError> {
        let guard = self.fetch_xml.lock().unwrap();
        if let Some(ref xml) = *guard {
            return Ok(xml.clone());
        }
        Ok(UntrustedInvoiceXml::new(format!(
            "<Faktura>{ksef_number}</Faktura>"
        )))
    }

    async fn query_invoices(
        &self,
        _access_token: &AccessToken,
        _criteria: &InvoiceQuery,
    ) -> Result<Vec<InvoiceMetadata>, KSeFError> {
        *self.query_count.lock().unwrap() += 1;

        let mut errors = self.query_errors.lock().unwrap();
        if !errors.is_empty() {
            return Err(errors.remove(0));
        }

        let guard = self.query_results.lock().unwrap();
        Ok(guard.clone())
    }

    async fn fetch_public_keys(&self) -> Result<Vec<KSeFPublicKey>, KSeFError> {
        Ok(vec![KSeFPublicKey::new(
            "mock-pem-key".to_string(),
            "mock-key-id".to_string(),
        )])
    }
}

/// Mock `InvoiceEncryptor`.
pub struct MockEncryptor;

#[async_trait]
impl InvoiceEncryptor for MockEncryptor {
    async fn encrypt(
        &self,
        _xml: &InvoiceXml,
        _public_key: &KSeFPublicKey,
    ) -> Result<EncryptedInvoice, CryptoError> {
        Ok(EncryptedInvoice::new(
            b"mock-encrypted-key".to_vec(),
            b"mock-iv-16bytes!".to_vec(),
            b"mock-encrypted-data".to_vec(),
            "mock-plaintext-hash".to_string(),
            123,
            "mock-encrypted-hash".to_string(),
            456,
        ))
    }
}
