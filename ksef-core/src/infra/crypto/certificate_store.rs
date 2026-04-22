use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use openssl::symm::{Cipher, decrypt_aead, encrypt_aead};
use rand::RngCore;

const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const DEV_ONLY_KEY: &[u8; 32] = b"0123456789abcdef0123456789abcdef";

/// AES-256-GCM wrapper for at-rest certificate storage.
#[derive(Debug, Clone)]
pub struct CertificateSecretBox {
    key: [u8; 32],
}

impl CertificateSecretBox {
    #[must_use]
    pub fn insecure_dev() -> Self {
        Self { key: *DEV_ONLY_KEY }
    }

    pub fn from_key_material(key: &[u8]) -> Result<Self, String> {
        if key.len() != 32 {
            return Err(format!(
                "certificate storage key must be exactly 32 bytes, got {}",
                key.len()
            ));
        }

        let mut normalized = [0u8; 32];
        normalized.copy_from_slice(key);
        Ok(Self { key: normalized })
    }

    pub fn from_base64(key_b64: &str) -> Result<Self, String> {
        let decoded = BASE64
            .decode(key_b64.trim())
            .map_err(|e| format!("invalid CERT_STORAGE_KEY base64: {e}"))?;
        Self::from_key_material(&decoded)
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<String, String> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);

        let mut tag = [0u8; TAG_LEN];
        let ciphertext = encrypt_aead(
            Cipher::aes_256_gcm(),
            &self.key,
            Some(&nonce),
            &[],
            plaintext,
            &mut tag,
        )
        .map_err(|e| format!("encrypt certificate secret: {e}"))?;

        Ok(format!(
            "{PREFIX}{}:{}:{}",
            BASE64.encode(nonce),
            BASE64.encode(ciphertext),
            BASE64.encode(tag)
        ))
    }

    pub fn decrypt_or_plaintext(&self, stored: &str) -> Result<Vec<u8>, String> {
        if !stored.starts_with(PREFIX) {
            return Ok(stored.as_bytes().to_vec());
        }

        let raw = &stored[PREFIX.len()..];
        let mut parts = raw.split(':');
        let Some(nonce_b64) = parts.next() else {
            return Err("encrypted certificate secret missing nonce".to_string());
        };
        let Some(ciphertext_b64) = parts.next() else {
            return Err("encrypted certificate secret missing ciphertext".to_string());
        };
        let Some(tag_b64) = parts.next() else {
            return Err("encrypted certificate secret missing tag".to_string());
        };
        if parts.next().is_some() {
            return Err("encrypted certificate secret has unexpected extra fields".to_string());
        }

        let nonce = BASE64
            .decode(nonce_b64)
            .map_err(|e| format!("invalid encrypted nonce: {e}"))?;
        let ciphertext = BASE64
            .decode(ciphertext_b64)
            .map_err(|e| format!("invalid encrypted ciphertext: {e}"))?;
        let tag = BASE64
            .decode(tag_b64)
            .map_err(|e| format!("invalid encrypted tag: {e}"))?;

        decrypt_aead(
            Cipher::aes_256_gcm(),
            &self.key,
            Some(&nonce),
            &[],
            &ciphertext,
            &tag,
        )
        .map_err(|e| format!("decrypt certificate secret: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::CertificateSecretBox;

    #[test]
    fn round_trips_encrypted_secret() {
        let crypto = CertificateSecretBox::insecure_dev();
        let encrypted = crypto.encrypt(b"top-secret-pem").unwrap();

        assert!(encrypted.starts_with("enc:v1:"));
        assert_eq!(
            crypto.decrypt_or_plaintext(&encrypted).unwrap(),
            b"top-secret-pem"
        );
    }

    #[test]
    fn accepts_legacy_plaintext_values() {
        let crypto = CertificateSecretBox::insecure_dev();
        assert_eq!(
            crypto
                .decrypt_or_plaintext("-----BEGIN CERTIFICATE-----")
                .unwrap(),
            b"-----BEGIN CERTIFICATE-----"
        );
    }

    #[test]
    fn rejects_wrong_key() {
        let encrypted = CertificateSecretBox::insecure_dev()
            .encrypt(b"top-secret-pem")
            .unwrap();
        let other =
            CertificateSecretBox::from_key_material(b"01234567890123456789012345678901").unwrap();

        assert!(other.decrypt_or_plaintext(&encrypted).is_err());
    }
}
