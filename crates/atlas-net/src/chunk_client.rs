//! [`atlas_chunk::ChunkStore`] over the wire.

use crate::runtime::ClientRuntime;
use atlas_chunk::ChunkStore;
use atlas_core::{Error, Hash, Result};
use atlas_proto::{ChunkRequest, ChunkResponse, Request, Response};
use std::sync::Arc;

pub struct RemoteChunkStore {
    rt: Arc<ClientRuntime>,
}

impl RemoteChunkStore {
    pub fn connect(addr: impl Into<String>) -> Result<Self> {
        Ok(Self {
            rt: ClientRuntime::connect(addr)?,
        })
    }

    pub fn from_runtime(rt: Arc<ClientRuntime>) -> Self {
        Self { rt }
    }

    fn call(&self, req: ChunkRequest) -> Result<ChunkResponse> {
        match self.rt.call(Request::Chunk(Box::new(req)))? {
            Response::Chunk(c) => Ok(*c),
            other => Err(Error::Backend(format!("unexpected response: {other:?}"))),
        }
    }
}

impl ChunkStore for RemoteChunkStore {
    fn put(&self, bytes: &[u8]) -> Result<Hash> {
        match self.call(ChunkRequest::Put {
            bytes: bytes.to_vec(),
        })? {
            ChunkResponse::Put { hash } => Ok(hash),
            other => Err(Error::Backend(format!("expected Put, got {other:?}"))),
        }
    }

    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        match self.call(ChunkRequest::Get { hash: *hash })? {
            ChunkResponse::Get { bytes } => Ok(bytes),
            other => Err(Error::Backend(format!("expected Get, got {other:?}"))),
        }
    }

    fn delete(&self, hash: &Hash) -> Result<()> {
        match self.call(ChunkRequest::Delete { hash: *hash })? {
            ChunkResponse::Delete => Ok(()),
            other => Err(Error::Backend(format!("expected Delete, got {other:?}"))),
        }
    }

    fn has(&self, hash: &Hash) -> Result<bool> {
        match self.call(ChunkRequest::Has { hash: *hash })? {
            ChunkResponse::Has { exists } => Ok(exists),
            other => Err(Error::Backend(format!("expected Has, got {other:?}"))),
        }
    }

    fn verify(&self, hash: &Hash) -> Result<()> {
        match self.call(ChunkRequest::Verify { hash: *hash })? {
            ChunkResponse::Verify => Ok(()),
            other => Err(Error::Backend(format!("expected Verify, got {other:?}"))),
        }
    }

    fn size(&self, hash: &Hash) -> Result<u64> {
        match self.call(ChunkRequest::Size { hash: *hash })? {
            ChunkResponse::Size { bytes } => Ok(bytes),
            other => Err(Error::Backend(format!("expected Size, got {other:?}"))),
        }
    }

    fn iter_hashes(&self) -> Result<Vec<Hash>> {
        match self.call(ChunkRequest::IterHashes)? {
            ChunkResponse::IterHashes { hashes } => Ok(hashes),
            other => Err(Error::Backend(format!(
                "expected IterHashes, got {other:?}"
            ))),
        }
    }
}
