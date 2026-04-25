//! [`MetaStore`] trait — the single dependency the rest of ATLAS has
//! on a metadata backend.

use atlas_core::{Error, Hash, Result};
use atlas_object::{
    Branch, Commit, DirectoryManifest, FileManifest, HeadState, RefRecord, StoreConfig,
};
use serde::{de::DeserializeOwned, Serialize};

/// A small batch of writes that commit atomically.
///
/// Backends may implement this as a real transaction (FoundationDB) or
/// as an atomic batch (sled, RocksDB).
pub struct Transaction {
    pub(crate) ops: Vec<TxOp>,
}

pub(crate) enum TxOp {
    Put { key: String, value: Vec<u8> },
    Delete { key: String },
}

impl Transaction {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub fn put_raw(&mut self, key: String, value: Vec<u8>) {
        self.ops.push(TxOp::Put { key, value });
    }

    pub fn delete(&mut self, key: String) {
        self.ops.push(TxOp::Delete { key });
    }

    pub fn put<T: Serialize>(&mut self, key: String, value: &T) -> Result<()> {
        let bytes = bincode::serialize(value).map_err(|e| Error::Serde(e.to_string()))?;
        self.put_raw(key, bytes);
        Ok(())
    }
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

/// Pluggable metadata backend.
pub trait MetaStore: Send + Sync {
    fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn put_raw(&self, key: &str, value: &[u8]) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;

    /// Iterate keys with the given ASCII prefix.
    fn scan_prefix(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>>;

    /// Apply a batch atomically.
    fn apply(&self, tx: Transaction) -> Result<()>;

    // -- Typed convenience helpers ------------------------------------

    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.get_raw(key)? {
            Some(bytes) => {
                let v = bincode::deserialize(&bytes).map_err(|e| Error::Serde(e.to_string()))?;
                Ok(Some(v))
            }
            None => Ok(None),
        }
    }

    fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let bytes = bincode::serialize(value).map_err(|e| Error::Serde(e.to_string()))?;
        self.put_raw(key, &bytes)
    }

    // -- Domain helpers ----------------------------------------------

    fn get_file_manifest(&self, hash: &Hash) -> Result<Option<FileManifest>> {
        self.get(&crate::keys::object(hash))
    }

    fn put_file_manifest(&self, m: &FileManifest) -> Result<()> {
        self.put(&crate::keys::object(&m.hash), m)
    }

    fn get_dir_manifest(&self, hash: &Hash) -> Result<Option<DirectoryManifest>> {
        self.get(&crate::keys::object(hash))
    }

    fn put_dir_manifest(&self, m: &DirectoryManifest) -> Result<()> {
        self.put(&crate::keys::object(&m.hash), m)
    }

    fn get_commit(&self, hash: &Hash) -> Result<Option<Commit>> {
        self.get(&crate::keys::commit(hash))
    }

    fn put_commit(&self, c: &Commit) -> Result<()> {
        self.put(&crate::keys::commit(&c.hash), c)
    }

    fn get_ref(&self, path: &str) -> Result<Option<RefRecord>> {
        self.get(&crate::keys::refkey(path))
    }

    fn put_ref(&self, r: &RefRecord) -> Result<()> {
        self.put(&crate::keys::refkey(&r.path), r)
    }

    fn delete_ref(&self, path: &str) -> Result<()> {
        self.delete(&crate::keys::refkey(path))
    }

    fn get_branch(&self, name: &str) -> Result<Option<Branch>> {
        self.get(&crate::keys::branch(name))
    }

    fn put_branch(&self, b: &Branch) -> Result<()> {
        self.put(&crate::keys::branch(&b.name), b)
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        let entries = self.scan_prefix(crate::keys::branch_prefix())?;
        let mut out = Vec::with_capacity(entries.len());
        for (_, bytes) in entries {
            let b: Branch =
                bincode::deserialize(&bytes).map_err(|e| Error::Serde(e.to_string()))?;
            out.push(b);
        }
        Ok(out)
    }

    fn get_head(&self) -> Result<Option<HeadState>> {
        self.get(crate::keys::head())
    }

    fn put_head(&self, h: &HeadState) -> Result<()> {
        self.put(crate::keys::head(), h)
    }

    fn get_config(&self) -> Result<Option<StoreConfig>> {
        self.get(crate::keys::config())
    }

    fn put_config(&self, c: &StoreConfig) -> Result<()> {
        self.put(crate::keys::config(), c)
    }
}
