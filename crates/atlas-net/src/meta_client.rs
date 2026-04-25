//! [`atlas_meta::MetaStore`] over the wire.

use crate::runtime::ClientRuntime;
use atlas_core::{Error, Hash, Result};
use atlas_meta::{MetaStore, Transaction};
use atlas_object::{
    BlobManifest, Branch, Commit, DirectoryManifest, FileManifest, HeadState, RefRecord,
    StoreConfig,
};
use atlas_proto::{BatchOp, MetaRequest, MetaResponse, Request, Response};
use std::sync::Arc;

pub struct RemoteMetaStore {
    rt: Arc<ClientRuntime>,
}

impl RemoteMetaStore {
    pub fn connect(addr: impl Into<String>) -> Result<Self> {
        Ok(Self {
            rt: ClientRuntime::connect(addr)?,
        })
    }

    pub fn from_runtime(rt: Arc<ClientRuntime>) -> Self {
        Self { rt }
    }

    fn call(&self, req: MetaRequest) -> Result<MetaResponse> {
        match self.rt.call(Request::Meta(Box::new(req)))? {
            Response::Meta(m) => Ok(*m),
            other => Err(Error::Backend(format!("unexpected response: {other:?}"))),
        }
    }
}

fn unexpected(label: &str, got: MetaResponse) -> Error {
    Error::Backend(format!("expected {label}, got {got:?}"))
}

impl MetaStore for RemoteMetaStore {
    fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.call(MetaRequest::GetRaw { key: key.into() })? {
            MetaResponse::OptBytes { value } => Ok(value),
            r => Err(unexpected("OptBytes", r)),
        }
    }

    fn put_raw(&self, key: &str, value: &[u8]) -> Result<()> {
        match self.call(MetaRequest::PutRaw {
            key: key.into(),
            value: value.to_vec(),
        })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        match self.call(MetaRequest::Delete { key: key.into() })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn scan_prefix(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
        match self.call(MetaRequest::ScanPrefix {
            prefix: prefix.into(),
        })? {
            MetaResponse::Pairs { pairs } => Ok(pairs),
            r => Err(unexpected("Pairs", r)),
        }
    }

    fn apply(&self, tx: Transaction) -> Result<()> {
        let ops = tx
            .into_ops()
            .into_iter()
            .map(|op| match op {
                atlas_meta::TxOpExternal::Put { key, value } => BatchOp::Put { key, value },
                atlas_meta::TxOpExternal::Delete { key } => BatchOp::Delete { key },
            })
            .collect();
        match self.call(MetaRequest::ApplyBatch { ops })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn get_blob_manifest(&self, hash: &Hash) -> Result<Option<BlobManifest>> {
        match self.call(MetaRequest::GetBlobManifest { hash: *hash })? {
            MetaResponse::OptBlobManifest { manifest } => Ok(manifest),
            r => Err(unexpected("OptBlobManifest", r)),
        }
    }

    fn put_blob_manifest(&self, m: &BlobManifest) -> Result<()> {
        match self.call(MetaRequest::PutBlobManifest {
            manifest: m.clone(),
        })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn get_file_manifest(&self, hash: &Hash) -> Result<Option<FileManifest>> {
        match self.call(MetaRequest::GetFileManifest { hash: *hash })? {
            MetaResponse::OptFileManifest { manifest } => Ok(manifest),
            r => Err(unexpected("OptFileManifest", r)),
        }
    }

    fn put_file_manifest(&self, m: &FileManifest) -> Result<()> {
        match self.call(MetaRequest::PutFileManifest {
            manifest: m.clone(),
        })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn get_dir_manifest(&self, hash: &Hash) -> Result<Option<DirectoryManifest>> {
        match self.call(MetaRequest::GetDirManifest { hash: *hash })? {
            MetaResponse::OptDirManifest { manifest } => Ok(manifest),
            r => Err(unexpected("OptDirManifest", r)),
        }
    }

    fn put_dir_manifest(&self, m: &DirectoryManifest) -> Result<()> {
        match self.call(MetaRequest::PutDirManifest {
            manifest: m.clone(),
        })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn get_commit(&self, hash: &Hash) -> Result<Option<Commit>> {
        match self.call(MetaRequest::GetCommit { hash: *hash })? {
            MetaResponse::OptCommit { commit } => Ok(commit),
            r => Err(unexpected("OptCommit", r)),
        }
    }

    fn put_commit(&self, c: &Commit) -> Result<()> {
        match self.call(MetaRequest::PutCommit { commit: c.clone() })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn get_ref(&self, path: &str) -> Result<Option<RefRecord>> {
        match self.call(MetaRequest::GetRef { path: path.into() })? {
            MetaResponse::OptRef { record } => Ok(record),
            r => Err(unexpected("OptRef", r)),
        }
    }

    fn put_ref(&self, r: &RefRecord) -> Result<()> {
        match self.call(MetaRequest::PutRef { record: r.clone() })? {
            MetaResponse::Empty => Ok(()),
            other => Err(unexpected("Empty", other)),
        }
    }

    fn delete_ref(&self, path: &str) -> Result<()> {
        match self.call(MetaRequest::DeleteRef { path: path.into() })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn get_branch(&self, name: &str) -> Result<Option<Branch>> {
        match self.call(MetaRequest::GetBranch { name: name.into() })? {
            MetaResponse::OptBranch { branch } => Ok(branch),
            r => Err(unexpected("OptBranch", r)),
        }
    }

    fn put_branch(&self, b: &Branch) -> Result<()> {
        match self.call(MetaRequest::PutBranch { branch: b.clone() })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        match self.call(MetaRequest::ListBranches)? {
            MetaResponse::Branches { branches } => Ok(branches),
            r => Err(unexpected("Branches", r)),
        }
    }

    fn get_head(&self) -> Result<Option<HeadState>> {
        match self.call(MetaRequest::GetHead)? {
            MetaResponse::OptHead { head } => Ok(head),
            r => Err(unexpected("OptHead", r)),
        }
    }

    fn put_head(&self, h: &HeadState) -> Result<()> {
        match self.call(MetaRequest::PutHead { head: h.clone() })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }

    fn get_config(&self) -> Result<Option<StoreConfig>> {
        match self.call(MetaRequest::GetConfig)? {
            MetaResponse::OptConfig { config } => Ok(config),
            r => Err(unexpected("OptConfig", r)),
        }
    }

    fn put_config(&self, c: &StoreConfig) -> Result<()> {
        match self.call(MetaRequest::PutConfig { config: c.clone() })? {
            MetaResponse::Empty => Ok(()),
            r => Err(unexpected("Empty", r)),
        }
    }
}
