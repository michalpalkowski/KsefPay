use std::sync::Arc;

use crate::domain::nip::Nip;
use crate::error::CryptoError;
use crate::ports::encryption::XadesSigner;
use crate::ports::signer_factory::{SignerCredentials, SignerFactory};

use super::OpenSslXadesSigner;

/// OpenSSL-based signer factory.
pub struct OpenSslSignerFactory;

impl SignerFactory for OpenSslSignerFactory {
    fn create_signer(
        &self,
        nip: &Nip,
        credentials: SignerCredentials<'_>,
    ) -> Result<Arc<dyn XadesSigner>, CryptoError> {
        match credentials {
            SignerCredentials::Pem { cert_pem, key_pem } => Ok(Arc::new(
                OpenSslXadesSigner::from_pem(key_pem.to_vec(), cert_pem.to_vec()),
            )),
            SignerCredentials::AutoGenerate => {
                tracing::info!(nip = %nip, "auto-generating self-signed certificate");
                let signer = OpenSslXadesSigner::generate_self_signed_for_nip(nip)?;
                Ok(Arc::new(signer))
            }
        }
    }
}
