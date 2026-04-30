//! Versioned bincode codec for all persisted manifests (P0-4).
//!
//! Every value stored in the MetaStore is prefixed with two bytes:
//!   [0] = MAGIC  (0xAB — "ATLAS Bincode")
//!   [1] = VERSION (currently 1)
//!
//! On decode, an unknown version returns `Error::UnsupportedVersion` so
//! the caller can run a migration rather than silently deserialising
//! garbage when a struct's schema changes between releases.

use atlas_core::{Error, Result};
use serde::{de::DeserializeOwned, Serialize};

const MAGIC: u8 = 0xAB;
const VERSION: u8 = 1;

/// Encode `value` with the two-byte version prefix.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let payload = bincode::serialize(value).map_err(|e| Error::Serde(e.to_string()))?;
    let mut buf = Vec::with_capacity(2 + payload.len());
    buf.push(MAGIC);
    buf.push(VERSION);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Decode bytes that were written by [`encode`].
/// Returns `Error::Corruption` on bad magic, `Error::UnsupportedVersion` on
/// an unknown version, `Error::Serde` on a malformed payload.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() < 2 {
        return Err(Error::Internal("manifest too short to contain version header".into()));
    }
    if bytes[0] != MAGIC {
        return Err(Error::Internal(format!(
            "bad manifest magic: expected 0x{MAGIC:02X}, got 0x{:02X}",
            bytes[0]
        )));
    }
    match bytes[1] {
        VERSION => {
            bincode::deserialize(&bytes[2..]).map_err(|e| Error::Serde(e.to_string()))
        }
        v => Err(Error::UnsupportedVersion(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Dummy { x: u32, label: String }

    #[test]
    fn round_trip() {
        let val = Dummy { x: 42, label: "hello".into() };
        let bytes = encode(&val).unwrap();
        assert_eq!(bytes[0], MAGIC);
        assert_eq!(bytes[1], VERSION);
        let back: Dummy = decode(&bytes).unwrap();
        assert_eq!(back, val);
    }

    #[test]
    fn bad_magic_errors() {
        let mut bytes = encode(&Dummy { x: 1, label: "a".into() }).unwrap();
        bytes[0] = 0xFF;
        assert!(decode::<Dummy>(&bytes).is_err());
    }

    #[test]
    fn unknown_version_errors() {
        let mut bytes = encode(&Dummy { x: 1, label: "a".into() }).unwrap();
        bytes[1] = 99;
        assert!(matches!(decode::<Dummy>(&bytes), Err(Error::UnsupportedVersion(99))));
    }
}
