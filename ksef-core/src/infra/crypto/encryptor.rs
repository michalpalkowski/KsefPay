use async_trait::async_trait;
use openssl::base64::encode_block;
use openssl::encrypt::Encrypter;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::rsa::{Padding, Rsa};
use openssl::sha::sha256;
use openssl::symm::{Cipher, encrypt as aes_encrypt};
use rand::RngCore;

use crate::domain::crypto::{EncryptedInvoice, KSeFPublicKey};
use crate::domain::xml::InvoiceXml;
use crate::error::CryptoError;
use crate::ports::encryption::InvoiceEncryptor;
use crate::ports::invoice_decryptor::InvoiceDecryptor;

/// AES-256-CBC + RSA-OAEP encryptor for `KSeF` invoice submission.
pub struct AesCbcEncryptor;

#[async_trait]
impl InvoiceEncryptor for AesCbcEncryptor {
    async fn encrypt(
        &self,
        xml: &InvoiceXml,
        public_key: &KSeFPublicKey,
    ) -> Result<EncryptedInvoice, CryptoError> {
        encrypt_invoice(xml.as_bytes(), public_key.pem())
    }
}

impl InvoiceDecryptor for AesCbcEncryptor {
    fn decrypt(&self, ciphertext: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError> {
        aes_256_cbc_decrypt(ciphertext, key, iv)
    }
}

fn encrypt_invoice(plaintext: &[u8], pem: &str) -> Result<EncryptedInvoice, CryptoError> {
    // Generate random AES-256 key and IV
    let mut aes_key = [0u8; 32];
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut aes_key);
    rand::thread_rng().fill_bytes(&mut iv);

    // Encrypt invoice XML with AES-256-CBC
    let encrypted_data = aes_encrypt(Cipher::aes_256_cbc(), &aes_key, Some(&iv), plaintext)
        .map_err(|e| CryptoError::AesEncryptionFailed(e.to_string()))?;

    // Encrypt AES key with RSA-OAEP using KSeF public key
    let rsa = Rsa::public_key_from_pem(pem.as_bytes())
        .map_err(|e| CryptoError::InvalidPublicKey(e.to_string()))?;
    let pkey = PKey::from_rsa(rsa).map_err(|e| CryptoError::RsaEncryptionFailed(e.to_string()))?;
    let mut encrypter =
        Encrypter::new(&pkey).map_err(|e| CryptoError::RsaEncryptionFailed(e.to_string()))?;
    encrypter
        .set_rsa_padding(Padding::PKCS1_OAEP)
        .map_err(|e| CryptoError::RsaEncryptionFailed(e.to_string()))?;
    encrypter
        .set_rsa_oaep_md(MessageDigest::sha256())
        .map_err(|e| CryptoError::RsaEncryptionFailed(e.to_string()))?;
    encrypter
        .set_rsa_mgf1_md(MessageDigest::sha256())
        .map_err(|e| CryptoError::RsaEncryptionFailed(e.to_string()))?;
    let mut encrypted_aes_key = vec![
        0u8;
        encrypter.encrypt_len(&aes_key).map_err(|e| {
            CryptoError::RsaEncryptionFailed(format!("compute encrypted key length: {e}"))
        })?
    ];
    let len = encrypter
        .encrypt(&aes_key, &mut encrypted_aes_key)
        .map_err(|e| CryptoError::RsaEncryptionFailed(e.to_string()))?;
    encrypted_aes_key.truncate(len);

    let plaintext_hash_sha256_base64 = encode_block(&sha256(plaintext));
    let plaintext_size_bytes = u64::try_from(plaintext.len())
        .map_err(|_| CryptoError::AesEncryptionFailed("plaintext too large".to_string()))?;
    let encrypted_hash_sha256_base64 = encode_block(&sha256(&encrypted_data));
    let encrypted_size_bytes = u64::try_from(encrypted_data.len())
        .map_err(|_| CryptoError::AesEncryptionFailed("ciphertext too large".to_string()))?;

    Ok(EncryptedInvoice::new(
        encrypted_aes_key,
        iv.to_vec(),
        encrypted_data,
        plaintext_hash_sha256_base64,
        plaintext_size_bytes,
        encrypted_hash_sha256_base64,
        encrypted_size_bytes,
    ))
}

/// Decrypt AES-256-CBC data using the provided raw key and IV.
pub fn aes_256_cbc_decrypt(
    ciphertext: &[u8],
    key: &[u8],
    iv: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    openssl::symm::decrypt(Cipher::aes_256_cbc(), key, Some(iv), ciphertext).map_err(|e| {
        CryptoError::AesEncryptionFailed(format!("AES-256-CBC decryption failed: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::encrypt::Decrypter;
    use openssl::rsa::Rsa;

    fn generate_test_keypair() -> (String, String) {
        let rsa = Rsa::generate(2048).unwrap();
        let pub_pem = String::from_utf8(rsa.public_key_to_pem().unwrap()).unwrap();
        let priv_pem = String::from_utf8(rsa.private_key_to_pem().unwrap()).unwrap();
        (pub_pem, priv_pem)
    }

    fn decrypt_invoice(encrypted: &EncryptedInvoice, private_pem: &str) -> Vec<u8> {
        // Decrypt AES key with RSA-OAEP (SHA-256 + MGF1-SHA256)
        let pkey = PKey::private_key_from_pem(private_pem.as_bytes()).unwrap();
        let mut decrypter = Decrypter::new(&pkey).unwrap();
        decrypter.set_rsa_padding(Padding::PKCS1_OAEP).unwrap();
        decrypter.set_rsa_oaep_md(MessageDigest::sha256()).unwrap();
        decrypter.set_rsa_mgf1_md(MessageDigest::sha256()).unwrap();
        let mut aes_key = vec![0u8; decrypter.decrypt_len(encrypted.aes_key()).unwrap()];
        let len = decrypter
            .decrypt(encrypted.aes_key(), &mut aes_key)
            .unwrap();
        let aes_key = &aes_key[..len];

        // Decrypt data with AES-256-CBC
        openssl::symm::decrypt(
            Cipher::aes_256_cbc(),
            aes_key,
            Some(encrypted.iv()),
            encrypted.data(),
        )
        .unwrap()
    }

    #[test]
    fn encrypt_and_decrypt_round_trip() {
        let (pub_pem, priv_pem) = generate_test_keypair();
        let plaintext = b"<Faktura>test invoice XML content</Faktura>";

        let encrypted = encrypt_invoice(plaintext, &pub_pem).unwrap();

        // Verify structure
        assert!(!encrypted.aes_key().is_empty());
        assert_eq!(encrypted.iv().len(), 16);
        assert!(!encrypted.data().is_empty());

        // Verify encrypted data differs from plaintext
        assert_ne!(encrypted.data(), plaintext);

        // Verify round-trip
        let decrypted = decrypt_invoice(&encrypted, &priv_pem);
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn each_encryption_produces_different_output() {
        let (pub_pem, _) = generate_test_keypair();
        let plaintext = b"<Faktura>same content</Faktura>";

        let a = encrypt_invoice(plaintext, &pub_pem).unwrap();
        let b = encrypt_invoice(plaintext, &pub_pem).unwrap();

        // Different random AES key + IV → different ciphertext
        assert_ne!(a.iv(), b.iv());
        assert_ne!(a.data(), b.data());
    }

    #[test]
    fn invalid_public_key_returns_error() {
        let result = encrypt_invoice(b"test", "not a valid PEM");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CryptoError::InvalidPublicKey(_)
        ));
    }

    #[tokio::test]
    async fn encryptor_trait_works() {
        let (pub_pem, priv_pem) = generate_test_keypair();
        let xml = InvoiceXml::new("<Faktura>trait test</Faktura>".to_string());
        let key = KSeFPublicKey::new(pub_pem, "test-key-id".to_string());

        let encryptor = AesCbcEncryptor;
        let encrypted = encryptor.encrypt(&xml, &key).await.unwrap();

        let decrypted = decrypt_invoice(&encrypted, &priv_pem);
        assert_eq!(decrypted, xml.as_bytes());
    }

    #[test]
    fn aes_256_cbc_encrypt_decrypt_round_trip() {
        let plaintext = "<Faktura>export test with Polish chars</Faktura>".as_bytes();
        let mut key = [0u8; 32];
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut key);
        rand::thread_rng().fill_bytes(&mut iv);

        let ciphertext = aes_encrypt(Cipher::aes_256_cbc(), &key, Some(&iv), plaintext).unwrap();
        assert_ne!(&ciphertext, plaintext);

        let decrypted = aes_256_cbc_decrypt(&ciphertext, &key, &iv).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn aes_256_cbc_decrypt_wrong_key_fails() {
        let plaintext = b"<Faktura>test</Faktura>";
        let mut key = [0u8; 32];
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut key);
        rand::thread_rng().fill_bytes(&mut iv);

        let ciphertext = aes_encrypt(Cipher::aes_256_cbc(), &key, Some(&iv), plaintext).unwrap();

        let mut wrong_key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut wrong_key);
        let result = aes_256_cbc_decrypt(&ciphertext, &wrong_key, &iv);
        assert!(result.is_err());
    }

    #[test]
    fn aes_256_cbc_decrypt_empty_ciphertext_fails() {
        let key = [0u8; 32];
        let iv = [0u8; 16];
        let result = aes_256_cbc_decrypt(&[], &key, &iv);
        assert!(result.is_err());
    }
}
