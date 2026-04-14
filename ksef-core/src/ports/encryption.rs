use async_trait::async_trait;

use crate::domain::auth::AuthChallenge;
use crate::domain::crypto::{EncryptedInvoice, KSeFPublicKey, SignedAuthRequest};
use crate::domain::nip::Nip;
use crate::domain::xml::InvoiceXml;
use crate::error::CryptoError;

/// Port: invoice encryption (AES-256-CBC + RSA-OAEP).
#[async_trait]
pub trait InvoiceEncryptor: Send + Sync {
    async fn encrypt(
        &self,
        xml: &InvoiceXml,
        public_key: &KSeFPublicKey,
    ) -> Result<EncryptedInvoice, CryptoError>;
}

/// Port: `XAdES` signing for `KSeF` auth.
#[async_trait]
pub trait XadesSigner: Send + Sync {
    async fn sign_auth_request(
        &self,
        challenge: &AuthChallenge,
        nip: &Nip,
    ) -> Result<SignedAuthRequest, CryptoError>;
}
