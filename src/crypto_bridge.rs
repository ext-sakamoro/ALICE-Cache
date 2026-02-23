//! ALICE-Cache × ALICE-Crypto Bridge
//!
//! Signed cache entries for tamper prevention.
//! Each cache entry includes a BLAKE3 integrity hash; optionally,
//! entries can be encrypted with XChaCha20-Poly1305.

use alice_crypto::{hash, open, seal, CipherError, Key};

/// Integrity-verified cache entry wrapper.
///
/// Stores data alongside a BLAKE3 hash for tamper detection.
#[derive(Clone)]
pub struct SignedEntry {
    /// The cached data.
    pub data: Vec<u8>,
    /// BLAKE3 hash of `data` at insertion time.
    pub hash: [u8; 32],
}

impl SignedEntry {
    /// Create a new signed entry from raw data.
    pub fn new(data: Vec<u8>) -> Self {
        let h = hash(&data);
        Self {
            hash: *h.as_bytes(),
            data,
        }
    }

    /// Verify the entry's integrity.
    ///
    /// Returns `true` if the data has not been modified since creation.
    pub fn verify(&self) -> bool {
        let h = hash(&self.data);
        *h.as_bytes() == self.hash
    }
}

/// Encrypted cache entry (authenticated encryption).
///
/// Data is sealed with XChaCha20-Poly1305 so cache contents
/// are confidential even if the cache is compromised.
#[derive(Clone)]
pub struct EncryptedEntry {
    /// Encrypted ciphertext (includes nonce + tag).
    pub ciphertext: Vec<u8>,
    /// BLAKE3 hash of the plaintext (for dedup without decrypting).
    pub plaintext_hash: [u8; 32],
}

/// Encrypted cache manager.
///
/// Wraps cache operations with automatic encryption/decryption.
pub struct CryptoCache {
    /// Encryption key for all entries.
    key: Key,
    /// Entries encrypted.
    pub encrypted_count: u64,
    /// Entries decrypted.
    pub decrypted_count: u64,
    /// Integrity check failures.
    pub integrity_failures: u64,
}

impl CryptoCache {
    /// Create a new encrypted cache with a fresh random key.
    pub fn new() -> Result<Self, CipherError> {
        Ok(Self {
            key: Key::generate()?,
            encrypted_count: 0,
            decrypted_count: 0,
            integrity_failures: 0,
        })
    }

    /// Create from an existing key.
    pub fn with_key(key: Key) -> Self {
        Self {
            key,
            encrypted_count: 0,
            decrypted_count: 0,
            integrity_failures: 0,
        }
    }

    /// Encrypt data for cache storage.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<EncryptedEntry, CipherError> {
        let plaintext_hash = *hash(plaintext).as_bytes();
        let ciphertext = seal(&self.key, plaintext)?;
        self.encrypted_count += 1;
        Ok(EncryptedEntry {
            ciphertext,
            plaintext_hash,
        })
    }

    /// Decrypt a cache entry.
    pub fn decrypt(&mut self, entry: &EncryptedEntry) -> Result<Vec<u8>, CipherError> {
        let plaintext = open(&self.key, &entry.ciphertext)?;

        // Verify plaintext hash
        let h = *hash(&plaintext).as_bytes();
        if h != entry.plaintext_hash {
            self.integrity_failures += 1;
            return Err(CipherError::DecryptionFailed);
        }

        self.decrypted_count += 1;
        Ok(plaintext)
    }

    /// Check if two encrypted entries contain the same plaintext
    /// (without decrypting — hash comparison only).
    pub fn entries_equal(a: &EncryptedEntry, b: &EncryptedEntry) -> bool {
        a.plaintext_hash == b.plaintext_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signed_entry() {
        let entry = SignedEntry::new(b"hello cache".to_vec());
        assert!(entry.verify());

        // Tamper
        let mut tampered = entry.clone();
        tampered.data[0] ^= 0xFF;
        assert!(!tampered.verify());
    }

    #[test]
    fn test_encrypted_roundtrip() {
        let mut cache = CryptoCache::new().unwrap();

        let data = b"secret cached data";
        let entry = cache.encrypt(data).unwrap();
        assert_eq!(cache.encrypted_count, 1);

        let recovered = cache.decrypt(&entry).unwrap();
        assert_eq!(&recovered, data);
        assert_eq!(cache.decrypted_count, 1);
    }

    #[test]
    fn test_entries_equal() {
        let mut cache = CryptoCache::new().unwrap();

        let e1 = cache.encrypt(b"same data").unwrap();
        let e2 = cache.encrypt(b"same data").unwrap();
        let e3 = cache.encrypt(b"different data").unwrap();

        assert!(CryptoCache::entries_equal(&e1, &e2));
        assert!(!CryptoCache::entries_equal(&e1, &e3));
    }

    #[test]
    fn test_wrong_key_fails() {
        let mut cache1 = CryptoCache::new().unwrap();
        let mut cache2 = CryptoCache::new().unwrap();

        let entry = cache1.encrypt(b"test").unwrap();

        // Decrypting with wrong key should fail
        assert!(cache2.decrypt(&entry).is_err());
    }
}
