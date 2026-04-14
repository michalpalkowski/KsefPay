use std::sync::Arc;

use crate::domain::nip::Nip;
use crate::error::CryptoError;
use crate::ports::encryption::XadesSigner;

/// Credentials for creating a signer.
pub enum SignerCredentials<'a> {
    /// PEM-encoded cert + private key (from DB or env).
    Pem { cert_pem: &'a [u8], key_pem: &'a [u8] },
    /// Auto-generate self-signed cert for the given NIP (test/demo only).
    AutoGenerate,
}

/// Port: create XAdES signers from credentials.
pub trait SignerFactory: Send + Sync {
    fn create_signer(
        &self,
        nip: &Nip,
        credentials: SignerCredentials<'_>,
    ) -> Result<Arc<dyn XadesSigner>, CryptoError>;
}
