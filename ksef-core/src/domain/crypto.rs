/// Signed `XAdES` auth request ready to submit to `KSeF`.
#[derive(Debug, Clone)]
pub struct SignedAuthRequest {
    signed_xml: Vec<u8>,
}

impl SignedAuthRequest {
    #[must_use]
    pub fn new(signed_xml: Vec<u8>) -> Self {
        Self { signed_xml }
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.signed_xml
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.signed_xml
    }
}

/// `KSeF` RSA public key for invoice encryption.
#[derive(Debug, Clone)]
pub struct KSeFPublicKey {
    key_pem: String,
    key_id: String,
}

impl KSeFPublicKey {
    #[must_use]
    pub fn new(key_pem: String, key_id: String) -> Self {
        Self { key_pem, key_id }
    }

    #[must_use]
    pub fn pem(&self) -> &str {
        &self.key_pem
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.key_id
    }
}

/// An encrypted invoice ready to submit to `KSeF`.
///
/// Binary format: `encrypted_aes_key || iv || encrypted_data`
#[derive(Debug, Clone)]
pub struct EncryptedInvoice {
    encrypted_aes_key: Vec<u8>,
    iv: Vec<u8>,
    encrypted_data: Vec<u8>,
    plaintext_hash_sha256_base64: String,
    plaintext_size_bytes: u64,
    encrypted_hash_sha256_base64: String,
    encrypted_size_bytes: u64,
}

impl EncryptedInvoice {
    #[must_use]
    pub fn new(
        encrypted_aes_key: Vec<u8>,
        iv: Vec<u8>,
        encrypted_data: Vec<u8>,
        plaintext_hash_sha256_base64: String,
        plaintext_size_bytes: u64,
        encrypted_hash_sha256_base64: String,
        encrypted_size_bytes: u64,
    ) -> Self {
        Self {
            encrypted_aes_key,
            iv,
            encrypted_data,
            plaintext_hash_sha256_base64,
            plaintext_size_bytes,
            encrypted_hash_sha256_base64,
            encrypted_size_bytes,
        }
    }

    #[must_use]
    pub fn aes_key(&self) -> &[u8] {
        &self.encrypted_aes_key
    }

    #[must_use]
    pub fn iv(&self) -> &[u8] {
        &self.iv
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.encrypted_data
    }

    #[must_use]
    pub fn plaintext_hash_sha256_base64(&self) -> &str {
        &self.plaintext_hash_sha256_base64
    }

    #[must_use]
    pub fn plaintext_size_bytes(&self) -> u64 {
        self.plaintext_size_bytes
    }

    #[must_use]
    pub fn encrypted_hash_sha256_base64(&self) -> &str {
        &self.encrypted_hash_sha256_base64
    }

    #[must_use]
    pub fn encrypted_size_bytes(&self) -> u64 {
        self.encrypted_size_bytes
    }
}
