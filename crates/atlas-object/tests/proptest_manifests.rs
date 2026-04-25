//! Property tests for the object model (T0.9).
//!
//! Invariants under test:
//!   1. `seal` is deterministic — equal inputs produce equal hashes.
//!   2. `seal` then `verify` always succeeds.
//!   3. Bincode roundtrip is lossless on sealed manifests.
//!   4. Mutating any non-hash field after sealing breaks `verify`.
//!   5. Two manifests differing in any non-hash field hash differently.

use atlas_core::{Hash, ObjectKind};
use atlas_object::codec::{decode, encode, seal, verify};
use atlas_object::manifest::{
    BlobManifest, ChunkRef, DirEntry, DirectoryManifest, FileManifest,
};
use proptest::collection::vec;
use proptest::prelude::*;

fn arb_hash() -> impl Strategy<Value = Hash> {
    any::<[u8; 32]>().prop_map(Hash::from_bytes)
}

fn arb_chunkref() -> impl Strategy<Value = ChunkRef> {
    (arb_hash(), 1u32..=4 * 1024 * 1024)
        .prop_map(|(hash, length)| ChunkRef { hash, length })
}

fn arb_blob() -> impl Strategy<Value = BlobManifest> {
    (
        any::<u64>(),
        prop::option::of("[a-z]{1,8}"),
        vec(arb_chunkref(), 0..6),
    )
        .prop_map(|(total_size, format_hint, chunks)| BlobManifest {
            hash: Hash::ZERO,
            total_size,
            format_hint,
            chunks,
        })
}

fn arb_kind() -> impl Strategy<Value = ObjectKind> {
    prop_oneof![
        Just(ObjectKind::File),
        Just(ObjectKind::Dir),
        Just(ObjectKind::Symlink),
        Just(ObjectKind::Refspec),
    ]
}

fn arb_dir_entry() -> impl Strategy<Value = DirEntry> {
    ("[a-z][a-z0-9_]{0,7}", arb_hash(), arb_kind())
        .prop_map(|(name, object_hash, kind)| DirEntry {
            name,
            object_hash,
            kind,
        })
}

fn arb_dir() -> impl Strategy<Value = DirectoryManifest> {
    vec(arb_dir_entry(), 0..6).prop_map(|mut entries| {
        // dedup by name and sort, matching the on-disk invariant.
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries.dedup_by(|a, b| a.name == b.name);
        DirectoryManifest {
            hash: Hash::ZERO,
            entries,
            xattrs: Vec::new(),
            policy_ref: None,
        }
    })
}

fn arb_file() -> impl Strategy<Value = FileManifest> {
    (arb_hash(), any::<i64>(), any::<u32>()).prop_map(|(blob_hash, created_at, mode)| {
        FileManifest {
            hash: Hash::ZERO,
            blob_hash,
            created_at,
            mode,
            xattrs: Vec::new(),
            embeddings: Vec::new(),
            schema_ref: None,
            lineage_ref: None,
            policy_ref: None,
            signatures: Vec::new(),
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn blob_seal_deterministic(b in arb_blob()) {
        let mut a = b.clone();
        let mut c = b.clone();
        let (ha, _) = seal(&mut a).unwrap();
        let (hc, _) = seal(&mut c).unwrap();
        prop_assert_eq!(ha, hc);
    }

    #[test]
    fn blob_seal_then_verify(b in arb_blob()) {
        let mut m = b;
        seal(&mut m).unwrap();
        verify(&m).unwrap();
    }

    #[test]
    fn blob_bincode_roundtrip(b in arb_blob()) {
        let mut m = b;
        seal(&mut m).unwrap();
        let bytes = encode(&m).unwrap();
        let back: BlobManifest = decode(&bytes).unwrap();
        prop_assert_eq!(m, back);
    }

    #[test]
    fn blob_tamper_breaks_verify(b in arb_blob()) {
        let mut m = b;
        seal(&mut m).unwrap();
        m.total_size = m.total_size.wrapping_add(1);
        prop_assert!(verify(&m).is_err());
    }

    #[test]
    fn dir_seal_deterministic(d in arb_dir()) {
        let mut a = d.clone();
        let mut c = d.clone();
        let (ha, _) = seal(&mut a).unwrap();
        let (hc, _) = seal(&mut c).unwrap();
        prop_assert_eq!(ha, hc);
    }

    #[test]
    fn dir_seal_then_verify(d in arb_dir()) {
        let mut m = d;
        seal(&mut m).unwrap();
        verify(&m).unwrap();
    }

    #[test]
    fn dir_entries_remain_sorted_after_roundtrip(d in arb_dir()) {
        let mut m = d;
        seal(&mut m).unwrap();
        let bytes = encode(&m).unwrap();
        let back: DirectoryManifest = decode(&bytes).unwrap();
        let names: Vec<&str> = back.entries.iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        prop_assert_eq!(names, sorted);
    }

    #[test]
    fn file_seal_then_verify(f in arb_file()) {
        let mut m = f;
        seal(&mut m).unwrap();
        verify(&m).unwrap();
    }

    #[test]
    fn file_mode_change_changes_hash(f in arb_file()) {
        let mut a = f.clone();
        let mut b = f.clone();
        b.mode = b.mode.wrapping_add(1);
        let (ha, _) = seal(&mut a).unwrap();
        let (hb, _) = seal(&mut b).unwrap();
        prop_assert_ne!(ha, hb);
    }
}
