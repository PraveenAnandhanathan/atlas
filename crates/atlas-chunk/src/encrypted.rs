//! AES-256-GCM at-rest encryption wrapper for [`LocalChunkStore`] (P8).
//!
//! # Design
//!
//! Content-addressing is preserved on the **plaintext**: the key stored on disk
//! is always `BLAKE3(plaintext)`, while the file *content* is the AES-256-GCM
//! ciphertext.  This means:
//!
//! - Deduplication still works — identical plaintexts produce identical hashes.
//! - Integrity checks still work — `verify()` decrypts and re-hashes the result.
//! - The only change vs `LocalChunkStore` is that the bytes on disk are opaque
//!   without the master key.
//!
//! # Key hierarchy
//!
//! ```text
//! master_key (32 bytes, random, stored in <store>/encryption.key)
//!     └── per-chunk key (32 bytes) = HKDF-SHA256(master_key, info=chunk_hash_hex)
//!             └── nonce (12 bytes) = first 12 bytes of per-chunk key
//! ```
//!
//! HKDF with the chunk hash as the `info` parameter produces a unique (key, nonce)
//! pair per chunk, so nonce reuse is structurally impossible.
//!
//! # Enterprise vs personal use
//!
//! For personal single-node deployments the master key lives in a file next to
//! the chunk store — the OS filesystem ACLs are the security boundary.  For
//! enterprise deployments the operator should set `ATLAS_ENCRYPTION_KEY` to a
//! hex-encoded 32-byte key fetched from a KMS (AWS KMS, HashiCorp Vault, etc.)
//! and ensure the file-based key is absent.

use crate::{ChunkStore, LocalChunkStore};
use aes_gcm::{aead::Aead, Aes256Gcm, Key, KeyInit, Nonce};
use atlas_core::{Error, Hash, Result};
use hkdf::Hkdf;
use sha2::Sha256;
use std::path::Path;

/// AES-256-GCM encrypted wrapper around [`LocalChunkStore`].
pub struct EncryptedChunkStore {
    inner: LocalChunkStore,
    master_key: [u8; 32],
}

impl EncryptedChunkStore {
    /// Open or create an encrypted store at `root`.
    ///
    /// The master key is resolved in priority order:
    /// 1. `ATLAS_ENCRYPTION_KEY` env var (hex-encoded 32 bytes) — for KMS integration.
    /// 2. `<root>/encryption.key` file — created with a fresh random key if absent.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;
        let master_key = resolve_master_key(root)?;
        let inner = LocalChunkStore::open(root.join("chunks"))?;
        Ok(Self { inner, master_key })
    }

    /// Wrap an existing `LocalChunkStore` with a caller-supplied key.
    /// Useful for tests and KMS-key-injection patterns.
    pub fn with_key(inner: LocalChunkStore, master_key: [u8; 32]) -> Self {
        Self { inner, master_key }
    }

    /// Re-encrypt every chunk in the store with `new_key`.
    ///
    /// Each chunk is decrypted with the current master key, then re-encrypted
    /// with `new_key` and written back in-place.  On success the in-memory
    /// master key is updated to `new_key`; the caller is responsible for
    /// persisting `new_key` to the key file **after** this method returns `Ok`.
    ///
    /// Returns the number of chunks re-encrypted.
    ///
    /// # Atomicity
    ///
    /// Each individual chunk write is atomic at the filesystem level (write to
    /// temp path, then rename).  If the process is interrupted mid-rotation the
    /// store will be left in a mixed state where some chunks use the old key and
    /// some use the new key.  Recovery requires re-running `key rotate` with
    /// whichever key was active at the time of interruption.
    pub fn rekey(&mut self, new_key: [u8; 32]) -> Result<usize> {
        let hashes: Vec<Hash> = self.inner
            .iter_hashes()
            .collect::<Result<_>>()?;
        let n = hashes.len();
        tracing::info!(chunks = n, "key rotation: starting");
        for hash in &hashes {
            // Read raw ciphertext from the inner store.
            let ciphertext = self.inner.get(hash)?;
            // Decrypt with the current (old) key.
            let (old_key_bytes, old_nonce_bytes) = derive_chunk_keymaterial(&self.master_key, hash);
            let old_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&old_key_bytes));
            let plaintext = old_cipher
                .decrypt(Nonce::from_slice(&old_nonce_bytes), ciphertext.as_ref())
                .map_err(|e| Error::Internal(format!("rekey: decrypt {}: {e}", hash.short())))?;
            // Re-encrypt with the new key.
            let (new_key_bytes, new_nonce_bytes) = derive_chunk_keymaterial(&new_key, hash);
            let new_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&new_key_bytes));
            let new_ciphertext = new_cipher
                .encrypt(Nonce::from_slice(&new_nonce_bytes), plaintext.as_ref())
                .map_err(|e| Error::Internal(format!("rekey: encrypt {}: {e}", hash.short())))?;
            // Write back, overwriting the old ciphertext.
            self.inner.put_raw_force(*hash, &new_ciphertext)?;
        }
        self.master_key = new_key;
        tracing::info!(chunks = n, "key rotation: complete");
        Ok(n)
    }

    /// Derive a unique (key, nonce) pair for `chunk_hash` using HKDF-SHA256.
    fn derive_key_nonce(&self, chunk_hash: &Hash) -> ([u8; 32], [u8; 12]) {
        derive_chunk_keymaterial(&self.master_key, chunk_hash)
    }

    fn encrypt(&self, plaintext: &[u8], hash: &Hash) -> Result<Vec<u8>> {
        let (key_bytes, nonce_bytes) = self.derive_key_nonce(hash);
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_bytes);
        cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| Error::Internal(format!("encrypt chunk {}: {e}", hash.short())))
    }

    fn decrypt(&self, ciphertext: &[u8], hash: &Hash) -> Result<Vec<u8>> {
        let (key_bytes, nonce_bytes) = self.derive_key_nonce(hash);
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::Internal(format!("decrypt chunk {}: {e}", hash.short())))
    }
}

/// Derive per-chunk (key, nonce) from `master_key` using HKDF-SHA256 with
/// the chunk hash as the `info` parameter.  Producing a unique (key, nonce)
/// pair per chunk prevents nonce reuse.
fn derive_chunk_keymaterial(master_key: &[u8; 32], chunk_hash: &Hash) -> ([u8; 32], [u8; 12]) {
    let hk = Hkdf::<Sha256>::new(None, master_key);
    let info = chunk_hash.to_hex();
    let mut okm = [0u8; 44]; // 32-byte key + 12-byte nonce
    hk.expand(info.as_bytes(), &mut okm)
        .expect("HKDF output length is always valid");
    let mut key = [0u8; 32];
    let mut nonce = [0u8; 12];
    key.copy_from_slice(&okm[..32]);
    nonce.copy_from_slice(&okm[32..]);
    (key, nonce)
}

impl crate::ChunkStore for EncryptedChunkStore {
    fn put(&self, bytes: &[u8]) -> Result<Hash> {
        let hash = Hash::of(bytes);
        if self.inner.has(&hash)? {
            return Ok(hash);
        }
        let ciphertext = self.encrypt(bytes, &hash)?;
        self.inner.put_raw(hash, &ciphertext)?;
        tracing::trace!(hash = %hash.short(), plain_len = bytes.len(), "encrypted chunk put");
        Ok(hash)
    }

    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        let ciphertext = self.inner.get(hash)?;
        let plaintext = self.decrypt(&ciphertext, hash)?;
        // Integrity: plaintext must match its hash.
        let actual = Hash::of(&plaintext);
        if &actual != hash {
            return Err(Error::Integrity {
                expected: hash.to_hex(),
                actual: actual.to_hex(),
            });
        }
        Ok(plaintext)
    }

    fn delete(&self, hash: &Hash) -> Result<()> {
        self.inner.delete(hash)
    }

    fn has(&self, hash: &Hash) -> Result<bool> {
        self.inner.has(hash)
    }

    fn verify(&self, hash: &Hash) -> Result<()> {
        // Decrypt and check — decrypt() already verifies the integrity.
        self.get(hash).map(|_| ())
    }

    fn size(&self, hash: &Hash) -> Result<u64> {
        // Ciphertext is plaintext_len + 16 bytes GCM tag; return ciphertext size
        // as a conservative upper bound (callers use this for capacity planning).
        self.inner.size(hash)
    }

    fn iter_hashes(&self) -> Box<dyn Iterator<Item = Result<Hash>> + '_> {
        self.inner.iter_hashes()
    }
}

// ── Key management ────────────────────────────────────────────────────────────

fn resolve_master_key(store_root: &Path) -> Result<[u8; 32]> {
    // Priority 1: environment variable (KMS / secrets manager injection).
    if let Ok(hex) = std::env::var("ATLAS_ENCRYPTION_KEY") {
        return parse_hex_key(hex.trim());
    }
    // Priority 2: file on disk.
    let key_path = store_root.join("encryption.key");
    if key_path.exists() {
        let hex = std::fs::read_to_string(&key_path)?;
        return parse_hex_key(hex.trim());
    }
    // No key found — generate a fresh one and persist it.
    generate_and_save_key(&key_path)
}

fn parse_hex_key(hex: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex)
        .map_err(|e| Error::Internal(format!("ATLAS_ENCRYPTION_KEY decode: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| Error::Internal("ATLAS_ENCRYPTION_KEY must be 32 bytes (64 hex chars)".into()))
}

fn generate_and_save_key(path: &Path) -> Result<[u8; 32]> {
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    // Atomic write so a crash mid-write never leaves a partial key.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, hex::encode(key))?;
    std::fs::rename(&tmp, path)?;
    tracing::info!(path = %path.display(), "generated new encryption master key");
    Ok(key)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChunkStore;
    use tempfile::TempDir;

    fn enc_store() -> (TempDir, EncryptedChunkStore) {
        let dir = tempfile::tempdir().unwrap();
        let inner = LocalChunkStore::open(dir.path().join("chunks")).unwrap();
        let key = [0xABu8; 32];
        (dir, EncryptedChunkStore::with_key(inner, key))
    }

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let (_d, s) = enc_store();
        let data = b"the quick brown fox jumps over the lazy dog";
        let h = s.put(data).unwrap();
        assert_eq!(s.get(&h).unwrap(), data.to_vec());
    }

    #[test]
    fn hash_is_plaintext_hash() {
        let (_d, s) = enc_store();
        let data = b"content";
        let h = s.put(data).unwrap();
        assert_eq!(h, Hash::of(data));
    }

    #[test]
    fn ciphertext_on_disk_differs_from_plaintext() {
        let (dir, s) = enc_store();
        let data = b"secret bytes";
        let h = s.put(data).unwrap();
        // Read the raw file from the inner LocalChunkStore path.
        let hex = h.to_hex();
        let raw_path = dir.path().join("chunks").join(&hex[..2]).join(&hex[2..4]).join(&hex);
        let raw = std::fs::read(raw_path).unwrap();
        assert_ne!(raw, data, "plaintext must not be stored on disk");
    }

    #[test]
    fn wrong_key_fails_decrypt() {
        let (dir, s) = enc_store();
        let data = b"private";
        let h = s.put(data).unwrap();

        // Open same path with a different key.
        let inner2 = LocalChunkStore::open(dir.path().join("chunks")).unwrap();
        let s2 = EncryptedChunkStore::with_key(inner2, [0xCCu8; 32]);
        assert!(s2.get(&h).is_err(), "wrong key must fail to decrypt");
    }

    #[test]
    fn idempotent_put() {
        let (_d, s) = enc_store();
        let h1 = s.put(b"same").unwrap();
        let h2 = s.put(b"same").unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn rekey_re_encrypts_all_chunks() {
        let (_d, mut s) = enc_store();
        let data1 = b"first secret payload";
        let data2 = b"second secret payload";
        let h1 = s.put(data1).unwrap();
        let h2 = s.put(data2).unwrap();

        let new_key = [0xDEu8; 32];
        let count = s.rekey(new_key).unwrap();
        assert_eq!(count, 2, "should have re-encrypted 2 chunks");

        // After rekey, both chunks must still be readable.
        assert_eq!(s.get(&h1).unwrap(), data1.to_vec());
        assert_eq!(s.get(&h2).unwrap(), data2.to_vec());
    }

    #[test]
    fn rekey_old_key_cannot_decrypt_after_rotation() {
        let (dir, mut s) = enc_store();
        let h = s.put(b"secret").unwrap();
        let new_key = [0xBBu8; 32];
        s.rekey(new_key).unwrap();

        // Open with the OLD key ([0xAB; 32]) — must fail to decrypt.
        let inner2 = LocalChunkStore::open(dir.path().join("chunks")).unwrap();
        let s2 = EncryptedChunkStore::with_key(inner2, [0xABu8; 32]);
        assert!(s2.get(&h).is_err(), "old key must not decrypt after rekey");
    }

    #[test]
    fn key_generated_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        // No ATLAS_ENCRYPTION_KEY env var, no file → auto-generate.
        let s1 = EncryptedChunkStore::open(dir.path()).unwrap();
        let h = s1.put(b"hello").unwrap();
        drop(s1);

        // Re-open — must load the same key from file.
        let s2 = EncryptedChunkStore::open(dir.path()).unwrap();
        assert_eq!(s2.get(&h).unwrap(), b"hello".to_vec());
    }
}
