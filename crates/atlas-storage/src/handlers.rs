//! Request handlers — translate one wire request into one local call.

use atlas_chunk::ChunkStore;
use atlas_core::Result;
use atlas_meta::{MetaStore, Transaction};
use atlas_proto::{BatchOp, ChunkRequest, ChunkResponse, MetaRequest, MetaResponse};

pub fn handle_chunk(store: &dyn ChunkStore, req: ChunkRequest) -> Result<ChunkResponse> {
    Ok(match req {
        ChunkRequest::Put { bytes } => ChunkResponse::Put {
            hash: store.put(&bytes)?,
        },
        ChunkRequest::Get { hash } => ChunkResponse::Get {
            bytes: store.get(&hash)?,
        },
        ChunkRequest::Delete { hash } => {
            store.delete(&hash)?;
            ChunkResponse::Delete
        }
        ChunkRequest::Has { hash } => ChunkResponse::Has {
            exists: store.has(&hash)?,
        },
        ChunkRequest::Verify { hash } => {
            store.verify(&hash)?;
            ChunkResponse::Verify
        }
        ChunkRequest::Size { hash } => ChunkResponse::Size {
            bytes: store.size(&hash)?,
        },
        ChunkRequest::IterHashes => ChunkResponse::IterHashes {
            hashes: store.iter_hashes().collect::<atlas_core::Result<Vec<_>>>()?,
        },
    })
}

pub fn handle_meta(store: &dyn MetaStore, req: MetaRequest) -> Result<MetaResponse> {
    Ok(match req {
        MetaRequest::GetRaw { key } => MetaResponse::OptBytes {
            value: store.get_raw(&key)?,
        },
        MetaRequest::PutRaw { key, value } => {
            store.put_raw(&key, &value)?;
            MetaResponse::Empty
        }
        MetaRequest::Delete { key } => {
            store.delete(&key)?;
            MetaResponse::Empty
        }
        MetaRequest::ScanPrefix { prefix } => MetaResponse::Pairs {
            pairs: store.scan_prefix(&prefix)?,
        },
        MetaRequest::ApplyBatch { ops } => {
            let mut tx = Transaction::new();
            for op in ops {
                match op {
                    BatchOp::Put { key, value } => tx.put_raw(key, value),
                    BatchOp::Delete { key } => tx.delete(key),
                }
            }
            store.apply(tx)?;
            MetaResponse::Empty
        }
        MetaRequest::GetBlobManifest { hash } => MetaResponse::OptBlobManifest {
            manifest: store.get_blob_manifest(&hash)?,
        },
        MetaRequest::PutBlobManifest { manifest } => {
            store.put_blob_manifest(&manifest)?;
            MetaResponse::Empty
        }
        MetaRequest::GetFileManifest { hash } => MetaResponse::OptFileManifest {
            manifest: store.get_file_manifest(&hash)?,
        },
        MetaRequest::PutFileManifest { manifest } => {
            store.put_file_manifest(&manifest)?;
            MetaResponse::Empty
        }
        MetaRequest::GetDirManifest { hash } => MetaResponse::OptDirManifest {
            manifest: store.get_dir_manifest(&hash)?,
        },
        MetaRequest::PutDirManifest { manifest } => {
            store.put_dir_manifest(&manifest)?;
            MetaResponse::Empty
        }
        MetaRequest::GetCommit { hash } => MetaResponse::OptCommit {
            commit: store.get_commit(&hash)?,
        },
        MetaRequest::PutCommit { commit } => {
            store.put_commit(&commit)?;
            MetaResponse::Empty
        }
        MetaRequest::GetRef { path } => MetaResponse::OptRef {
            record: store.get_ref(&path)?,
        },
        MetaRequest::PutRef { record } => {
            store.put_ref(&record)?;
            MetaResponse::Empty
        }
        MetaRequest::DeleteRef { path } => {
            store.delete_ref(&path)?;
            MetaResponse::Empty
        }
        MetaRequest::GetBranch { name } => MetaResponse::OptBranch {
            branch: store.get_branch(&name)?,
        },
        MetaRequest::PutBranch { branch } => {
            store.put_branch(&branch)?;
            MetaResponse::Empty
        }
        MetaRequest::ListBranches => MetaResponse::Branches {
            branches: store.list_branches()?,
        },
        MetaRequest::GetHead => MetaResponse::OptHead {
            head: store.get_head()?,
        },
        MetaRequest::PutHead { head } => {
            store.put_head(&head)?;
            MetaResponse::Empty
        }
        MetaRequest::GetConfig => MetaResponse::OptConfig {
            config: store.get_config()?,
        },
        MetaRequest::PutConfig { config } => {
            store.put_config(&config)?;
            MetaResponse::Empty
        }
    })
}
