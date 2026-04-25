//! Sled-backed [`MetaStore`] implementation (ADR-0003).

use crate::store::{MetaStore, Transaction, TxOp};
use atlas_core::{Error, Result};
use std::path::Path;

/// A `sled` database wrapped as a [`MetaStore`].
pub struct SledStore {
    db: sled::Db,
}

impl SledStore {
    /// Open (or create) a sled database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = sled::Config::new()
            .path(path)
            .mode(sled::Mode::HighThroughput)
            .open()
            .map_err(|e| Error::Backend(format!("sled: {e}")))?;
        Ok(Self { db })
    }

    /// Flush all pending writes to disk. Useful before process exit.
    pub fn flush(&self) -> Result<()> {
        self.db
            .flush()
            .map(|_| ())
            .map_err(|e| Error::Backend(format!("sled flush: {e}")))
    }
}

impl MetaStore for SledStore {
    fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.db
            .get(key.as_bytes())
            .map(|o| o.map(|ivec| ivec.to_vec()))
            .map_err(|e| Error::Backend(format!("sled get: {e}")))
    }

    fn put_raw(&self, key: &str, value: &[u8]) -> Result<()> {
        self.db
            .insert(key.as_bytes(), value.to_vec())
            .map(|_| ())
            .map_err(|e| Error::Backend(format!("sled put: {e}")))
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.db
            .remove(key.as_bytes())
            .map(|_| ())
            .map_err(|e| Error::Backend(format!("sled delete: {e}")))
    }

    fn scan_prefix(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let mut out = Vec::new();
        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (k, v) = item.map_err(|e| Error::Backend(format!("sled scan: {e}")))?;
            let key = String::from_utf8(k.to_vec())
                .map_err(|e| Error::Backend(format!("non-utf8 key: {e}")))?;
            out.push((key, v.to_vec()));
        }
        Ok(out)
    }

    fn apply(&self, tx: Transaction) -> Result<()> {
        // sled::Batch is atomic across a single tree.
        let mut batch = sled::Batch::default();
        for op in tx.ops {
            match op {
                TxOp::Put { key, value } => batch.insert(key.as_bytes(), value),
                TxOp::Delete { key } => batch.remove(key.as_bytes()),
            }
        }
        self.db
            .apply_batch(batch)
            .map_err(|e| Error::Backend(format!("sled apply: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::Hash;
    use atlas_object::{Branch, BranchProtection, RefRecord};

    fn tmp() -> (tempfile::TempDir, SledStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SledStore::open(dir.path().join("db")).unwrap();
        (dir, store)
    }

    #[test]
    fn put_get_delete_raw() {
        let (_d, s) = tmp();
        s.put_raw("k", b"v").unwrap();
        assert_eq!(s.get_raw("k").unwrap(), Some(b"v".to_vec()));
        s.delete("k").unwrap();
        assert_eq!(s.get_raw("k").unwrap(), None);
    }

    #[test]
    fn refs_and_branches_roundtrip() {
        let (_d, s) = tmp();
        let r = RefRecord {
            path: "/foo".into(),
            target: Hash::of(b"t"),
            updated_at: 1,
        };
        s.put_ref(&r).unwrap();
        assert_eq!(s.get_ref("/foo").unwrap(), Some(r.clone()));

        let b = Branch {
            name: "main".into(),
            head: Hash::of(b"c"),
            protection: BranchProtection::default(),
        };
        s.put_branch(&b).unwrap();
        assert_eq!(s.list_branches().unwrap(), vec![b]);
    }

    #[test]
    fn scan_prefix_is_bounded() {
        let (_d, s) = tmp();
        s.put_raw("ref:/a", b"1").unwrap();
        s.put_raw("ref:/b", b"2").unwrap();
        s.put_raw("commit:x", b"3").unwrap();
        let refs = s.scan_prefix("ref:").unwrap();
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn transaction_is_atomic() {
        let (_d, s) = tmp();
        let mut tx = Transaction::new();
        tx.put_raw("a".into(), b"1".to_vec());
        tx.put_raw("b".into(), b"2".to_vec());
        s.apply(tx).unwrap();
        assert_eq!(s.get_raw("a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(s.get_raw("b").unwrap(), Some(b"2".to_vec()));
    }
}
