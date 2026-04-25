//! BLAKE3-256 hash wrapper (see ADR-0002).
//!
//! A `Hash` is a 32-byte digest. It is serialized as raw bytes in
//! bincode canonical form (used for content-addressing) and as lowercase
//! hex in human-facing APIs (CLI, REST, MCP).

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Byte length of a BLAKE3-256 digest.
pub const HASH_LEN: usize = 32;

/// A BLAKE3-256 digest.
///
/// Zero-valued hashes (`Hash::ZERO`) are reserved for the "hash-of-manifest
/// with its own hash slot zeroed" construction described in spec v0.1 §3.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hash(#[serde(with = "serde_bytes_array")] pub [u8; HASH_LEN]);

impl Hash {
    pub const ZERO: Self = Self([0u8; HASH_LEN]);

    /// Compute a BLAKE3-256 digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Build a `Hash` from raw bytes.
    pub const fn from_bytes(bytes: [u8; HASH_LEN]) -> Self {
        Self(bytes)
    }

    /// Parse a lowercase hex string. Returns `Error::BadHash` on failure.
    pub fn from_hex(s: &str) -> Result<Self> {
        let mut out = [0u8; HASH_LEN];
        hex::decode_to_slice(s, &mut out).map_err(|_| Error::BadHash(s.to_string()))?;
        Ok(Self(out))
    }

    /// Lowercase hex representation (64 chars).
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Raw bytes.
    pub fn as_bytes(&self) -> &[u8; HASH_LEN] {
        &self.0
    }

    /// Short form for human-readable log lines: the first 12 hex chars.
    pub fn short(&self) -> String {
        hex::encode(&self.0[..6])
    }

    /// True if this is the all-zeroes sentinel.
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; HASH_LEN]
    }
}

impl core::fmt::Debug for Hash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Hash({})", self.to_hex())
    }
}

impl core::fmt::Display for Hash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl core::str::FromStr for Hash {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::from_hex(s)
    }
}

/// Streaming BLAKE3 hasher for large inputs.
pub struct Hasher(blake3::Hasher);

impl Hasher {
    pub fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    pub fn update(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update(bytes);
        self
    }

    pub fn finalize(&self) -> Hash {
        Hash(*self.0.finalize().as_bytes())
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// serde helper: serialize [u8; 32] as a fixed-length byte array.
///
/// bincode already encodes this efficiently (32 raw bytes, no length
/// prefix) because the length is known at compile time.
mod serde_bytes_array {
    use super::HASH_LEN;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; HASH_LEN], s: S) -> Result<S::Ok, S::Error> {
        bytes.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; HASH_LEN], D::Error> {
        <[u8; HASH_LEN]>::deserialize(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_vector() {
        // Known BLAKE3 test vector for the empty string.
        let h = Hash::of(b"");
        assert_eq!(
            h.to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn hex_roundtrip() {
        let h = Hash::of(b"atlas");
        let s = h.to_hex();
        let h2 = Hash::from_hex(&s).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn zero_sentinel() {
        assert!(Hash::ZERO.is_zero());
        assert!(!Hash::of(b"x").is_zero());
    }

    #[test]
    fn short_is_12_chars() {
        assert_eq!(Hash::of(b"x").short().len(), 12);
    }

    #[test]
    fn bad_hex_errors() {
        assert!(Hash::from_hex("not-a-hash").is_err());
        assert!(Hash::from_hex("deadbeef").is_err()); // too short
    }
}
