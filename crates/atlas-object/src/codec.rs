//! Canonical encoding and self-hashing for manifests.
//!
//! See spec v0.1 §3. All manifests are serialized with bincode v1 legacy
//! (fixed-int, little-endian). The manifest's `hash` field is computed
//! by serializing the manifest with its `hash` slot replaced by
//! `Hash::ZERO`, then BLAKE3-hashing those bytes.

use atlas_core::{Error, Hash, Result};
use serde::{de::DeserializeOwned, Serialize};

/// Encode any serde-serializable value in canonical bincode form.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    bincode::serialize(value).map_err(|e| Error::Serde(e.to_string()))
}

/// Decode a canonical bincode buffer.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    bincode::deserialize(bytes).map_err(|e| Error::Serde(e.to_string()))
}

/// Trait implemented for every manifest type that carries a self-hash.
///
/// We keep this trait minimal: it exposes the `hash` field for read and
/// write so the codec can zero it for canonical hashing without caring
/// about the manifest's other fields.
pub trait SelfHashed: Serialize + DeserializeOwned + Clone {
    fn hash(&self) -> Hash;
    fn set_hash(&mut self, h: Hash);
}

macro_rules! impl_self_hashed {
    ($t:ty) => {
        impl $crate::codec::SelfHashed for $t {
            fn hash(&self) -> ::atlas_core::Hash {
                self.hash
            }
            fn set_hash(&mut self, h: ::atlas_core::Hash) {
                self.hash = h;
            }
        }
    };
}

impl_self_hashed!(crate::manifest::BlobManifest);
impl_self_hashed!(crate::manifest::FileManifest);
impl_self_hashed!(crate::manifest::DirectoryManifest);
impl_self_hashed!(crate::manifest::Commit);

/// Serialize `m` with its hash slot zeroed.
///
/// The resulting bytes are stable across any two agents that follow the
/// spec, and hashing them gives the manifest's content hash.
pub fn encode_with_zero_hash<T: SelfHashed>(m: &T) -> Result<Vec<u8>> {
    let mut copy = m.clone();
    copy.set_hash(Hash::ZERO);
    encode(&copy)
}

/// Compute the canonical content hash of a manifest.
pub fn hash_manifest<T: SelfHashed>(m: &T) -> Result<Hash> {
    let bytes = encode_with_zero_hash(m)?;
    Ok(Hash::of(&bytes))
}

/// Encode a manifest to bytes and set its hash to the canonical value.
/// Returns `(hash, bytes_of_the_hashed_manifest)`.
pub fn seal<T: SelfHashed>(m: &mut T) -> Result<(Hash, Vec<u8>)> {
    let h = hash_manifest(m)?;
    m.set_hash(h);
    let bytes = encode(m)?;
    Ok((h, bytes))
}

/// Verify the self-hash of a manifest.
pub fn verify<T: SelfHashed>(m: &T) -> Result<()> {
    let actual = hash_manifest(m)?;
    if actual != m.hash() {
        return Err(Error::Integrity {
            expected: m.hash().to_hex(),
            actual: actual.to_hex(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::*;
    use atlas_core::ObjectKind;

    fn sample_blob() -> BlobManifest {
        BlobManifest {
            hash: Hash::ZERO,
            total_size: 8,
            format_hint: None,
            chunks: vec![
                ChunkRef {
                    hash: Hash::of(b"aaaa"),
                    length: 4,
                },
                ChunkRef {
                    hash: Hash::of(b"bbbb"),
                    length: 4,
                },
            ],
        }
    }

    fn sample_dir() -> DirectoryManifest {
        DirectoryManifest {
            hash: Hash::ZERO,
            entries: vec![
                DirEntry {
                    name: "alpha.txt".into(),
                    object_hash: Hash::of(b"a"),
                    kind: ObjectKind::File,
                },
                DirEntry {
                    name: "sub".into(),
                    object_hash: Hash::of(b"s"),
                    kind: ObjectKind::Dir,
                },
            ],
            xattrs: Vec::new(),
            policy_ref: None,
        }
    }

    #[test]
    fn seal_then_verify() {
        let mut m = sample_blob();
        let (h, _bytes) = seal(&mut m).unwrap();
        assert_eq!(m.hash, h);
        verify(&m).unwrap();
    }

    #[test]
    fn tampering_breaks_verify() {
        let mut m = sample_blob();
        let (_, _) = seal(&mut m).unwrap();
        m.total_size += 1;
        assert!(verify(&m).is_err());
    }

    #[test]
    fn distinct_manifests_have_distinct_hashes() {
        let mut a = sample_blob();
        let mut b = sample_blob();
        b.total_size = 16;
        let (ha, _) = seal(&mut a).unwrap();
        let (hb, _) = seal(&mut b).unwrap();
        assert_ne!(ha, hb);
    }

    #[test]
    fn bincode_roundtrip_blob() {
        let mut m = sample_blob();
        seal(&mut m).unwrap();
        let bytes = encode(&m).unwrap();
        let back: BlobManifest = decode(&bytes).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn dir_hash_is_deterministic() {
        let mut m1 = sample_dir();
        let mut m2 = sample_dir();
        let (h1, _) = seal(&mut m1).unwrap();
        let (h2, _) = seal(&mut m2).unwrap();
        assert_eq!(h1, h2);
    }
}
