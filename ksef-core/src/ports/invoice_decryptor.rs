use crate::error::CryptoError;

/// Port: symmetric decryption for exported invoice archives.
pub trait InvoiceDecryptor: Send + Sync {
    fn decrypt(&self, ciphertext: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError>;
}
