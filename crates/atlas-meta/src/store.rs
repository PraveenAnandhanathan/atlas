//! [`MetaStore`] trait — the single dependency the rest of ATLAS has
//! on a metadata backend.

use atlas_core::{Hash, Result};
use atlas_object::{
    BlobManifest, Branch, Commit, DirectoryManifest, FileManifest, HeadState, RefRecord,
    StoreConfig,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::versioned::{decode as decode_versioned, encode as encode_versioned};

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

/// Public view of [`TxOp`] for wire-layer adapters that need to ship a
/// transaction over a network.
#[derive(Debug, Clone)]
pub enum TxOpExternal {
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
        let bytes = encode_versioned(value)?;
        self.put_raw(key, bytes);
        Ok(())
    }

    /// Consume the transaction and return its operations in a public form.
    pub fn into_ops(self) -> Vec<TxOpExternal> {
        self.ops
            .into_iter()
            .map(|op| match op {
                TxOp::Put { key, value } => TxOpExternal::Put { key, value },
                TxOp::Delete { key } => TxOpExternal::Delete { key },
            })
            .collect()
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

    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>
    where
        Self: Sized,
    {
        match self.get_raw(key)? {
            Some(bytes) => Ok(Some(decode_versioned(&bytes)?)),
            None => Ok(None),
        }
    }

    fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()>
    where
        Self: Sized,
    {
        let bytes = encode_versioned(value)?;
        self.put_raw(key, &bytes)
    }

    // -- Domain helpers ----------------------------------------------
    //
    // These take concrete types so they remain dyn-callable. Each one
    // hits get_raw/put_raw with versioned bincode encoding.

    fn get_blob_manifest(&self, hash: &Hash) -> Result<Option<BlobManifest>> {
        decode_opt(self.get_raw(&crate::keys::object(hash))?)
    }

    fn put_blob_manifest(&self, m: &BlobManifest) -> Result<()> {
        let bytes = encode(m)?;
        self.put_raw(&crate::keys::object(&m.hash), &bytes)
    }

    fn get_file_manifest(&self, hash: &Hash) -> Result<Option<FileManifest>> {
        decode_opt(self.get_raw(&crate::keys::object(hash))?)
    }

    fn put_file_manifest(&self, m: &FileManifest) -> Result<()> {
        let bytes = encode(m)?;
        self.put_raw(&crate::keys::object(&m.hash), &bytes)
    }

    fn get_dir_manifest(&self, hash: &Hash) -> Result<Option<DirectoryManifest>> {
        decode_opt(self.get_raw(&crate::keys::object(hash))?)
    }

    fn put_dir_manifest(&self, m: &DirectoryManifest) -> Result<()> {
        let bytes = encode(m)?;
        self.put_raw(&crate::keys::object(&m.hash), &bytes)
    }

    fn get_commit(&self, hash: &Hash) -> Result<Option<Commit>> {
        decode_opt(self.get_raw(&crate::keys::commit(hash))?)
    }

    fn put_commit(&self, c: &Commit) -> Result<()> {
        let bytes = encode(c)?;
        self.put_raw(&crate::keys::commit(&c.hash), &bytes)
    }

    fn get_ref(&self, path: &str) -> Result<Option<RefRecord>> {
        decode_opt(self.get_raw(&crate::keys::refkey(path))?)
    }

    fn put_ref(&self, r: &RefRecord) -> Result<()> {
        let bytes = encode(r)?;
        self.put_raw(&crate::keys::refkey(&r.path), &bytes)
    }

    fn delete_ref(&self, path: &str) -> Result<()> {
        self.delete(&crate::keys::refkey(path))
    }

    fn get_branch(&self, name: &str) -> Result<Option<Branch>> {
        decode_opt(self.get_raw(&crate::keys::branch(name))?)
    }

    fn put_branch(&self, b: &Branch) -> Result<()> {
        let bytes = encode(b)?;
        self.put_raw(&crate::keys::branch(&b.name), &bytes)
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        let entries = self.scan_prefix(crate::keys::branch_prefix())?;
        let mut out = Vec::with_capacity(entries.len());
        for (_, bytes) in entries {
            let b: Branch = decode_versioned(&bytes)?;
            out.push(b);
        }
        Ok(out)
    }

    fn get_head(&self) -> Result<Option<HeadState>> {
        decode_opt(self.get_raw(crate::keys::head())?)
    }

    fn put_head(&self, h: &HeadState) -> Result<()> {
        let bytes = encode(h)?;
        self.put_raw(crate::keys::head(), &bytes)
    }

    fn get_config(&self) -> Result<Option<StoreConfig>> {
        decode_opt(self.get_raw(crate::keys::config())?)
    }

    fn put_config(&self, c: &StoreConfig) -> Result<()> {
        let bytes = encode(c)?;
        self.put_raw(crate::keys::config(), &bytes)
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    encode_versioned(value)
}

fn decode_opt<T: DeserializeOwned>(bytes: Option<Vec<u8>>) -> Result<Option<T>> {
    match bytes {
        Some(b) => Ok(Some(decode_versioned(&b)?)),
        None => Ok(None),
    }
}
