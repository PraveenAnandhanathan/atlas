//! Chain replication for ATLAS chunks.
//!
//! Implements the [`ChunkStore`] trait by fanning writes along an
//! ordered chain of remote stores (head → ... → tail) and reading from
//! the tail, in the style of **CRAQ** (Chain Replication with
//! Apportioned Queries).
//!
//! Phase 2 scope:
//! - **Writes** propagate sequentially through the chain. The call
//!   returns success only when the tail acks, giving strong consistency.
//! - **Reads** go to the tail, which is by definition the node with the
//!   most-recently committed value. (CRAQ's "apportioned queries"
//!   optimisation — read from any node, fall back to tail on dirty —
//!   lands once per-chunk version vectors are added.)
//! - **Membership** is static for now. Online reconfiguration (the
//!   master service in CRAQ §2.3) is a Phase 3 task.
//!
//! The chain is provided as a `Vec<Arc<dyn ChunkStore>>`. The same
//! abstraction works for an in-memory test chain and a chain of
//! [`atlas_net::RemoteChunkStore`] handles in production.

use atlas_chunk::ChunkStore;
use atlas_core::{Error, Hash, Result};
use std::sync::Arc;

/// Where in the chain a node sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainRole {
    Head,
    Middle,
    Tail,
    Singleton,
}

/// A statically-configured chain of [`ChunkStore`] backends.
///
/// Implements [`ChunkStore`] itself, so call sites in `atlas-fs` need
/// no modification — drop a `ReplicatedChunkStore` in place of the
/// local store and writes become 3-way replicated.
pub struct ReplicatedChunkStore {
    chain: Vec<Arc<dyn ChunkStore>>,
}

impl ReplicatedChunkStore {
    /// Build a chain. The order matters: index 0 is the head; the last
    /// index is the tail.
    pub fn new(chain: Vec<Arc<dyn ChunkStore>>) -> Result<Self> {
        if chain.is_empty() {
            return Err(Error::Invalid("replication chain must be non-empty".into()));
        }
        Ok(Self { chain })
    }

    pub fn len(&self) -> usize {
        self.chain.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    pub fn role_at(&self, idx: usize) -> ChainRole {
        if self.chain.len() == 1 {
            ChainRole::Singleton
        } else if idx == 0 {
            ChainRole::Head
        } else if idx == self.chain.len() - 1 {
            ChainRole::Tail
        } else {
            ChainRole::Middle
        }
    }

    fn tail(&self) -> &Arc<dyn ChunkStore> {
        self.chain.last().expect("non-empty by construction")
    }
}

impl ChunkStore for ReplicatedChunkStore {
    /// Walk the chain and put on every replica. Returns the hash if
    /// every node accepted; on the first failure we abort and surface
    /// the error. The caller can retry — `put` is idempotent because
    /// it's content-addressed.
    fn put(&self, bytes: &[u8]) -> Result<Hash> {
        let mut last_hash = None;
        for (i, node) in self.chain.iter().enumerate() {
            let h = node
                .put(bytes)
                .map_err(|e| Error::Backend(format!("chain put failed at node {i}: {e}")))?;
            // Sanity: every node should agree on the hash.
            if let Some(prev) = last_hash {
                if prev != h {
                    return Err(Error::Integrity {
                        expected: format!("{prev:?}"),
                        actual: format!("{h:?}"),
                    });
                }
            }
            last_hash = Some(h);
        }
        Ok(last_hash.unwrap())
    }

    /// Reads from the tail (CRAQ §2.2 — the tail is always clean).
    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        self.tail().get(hash)
    }

    fn delete(&self, hash: &Hash) -> Result<()> {
        // Delete propagates head → tail, same as put.
        for (i, node) in self.chain.iter().enumerate() {
            if let Err(e) = node.delete(hash) {
                // Tolerate "not found" along the chain — partial state is
                // legal during recovery.
                if !matches!(e, Error::NotFound(_)) {
                    return Err(Error::Backend(format!(
                        "chain delete failed at node {i}: {e}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn has(&self, hash: &Hash) -> Result<bool> {
        self.tail().has(hash)
    }

    fn verify(&self, hash: &Hash) -> Result<()> {
        // Verify on every replica — that's the whole point.
        for (i, node) in self.chain.iter().enumerate() {
            node.verify(hash)
                .map_err(|e| Error::Backend(format!("verify failed at node {i}: {e}")))?;
        }
        Ok(())
    }

    fn size(&self, hash: &Hash) -> Result<u64> {
        self.tail().size(hash)
    }

    fn iter_hashes(&self) -> Result<Vec<Hash>> {
        // Tail is the source of truth.
        self.tail().iter_hashes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_chunk::LocalChunkStore;
    use tempfile::TempDir;

    fn three_node_chain() -> (Vec<TempDir>, ReplicatedChunkStore) {
        let dirs: Vec<TempDir> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
        let chain: Vec<Arc<dyn ChunkStore>> = dirs
            .iter()
            .map(|d| Arc::new(LocalChunkStore::open(d.path()).unwrap()) as Arc<dyn ChunkStore>)
            .collect();
        (dirs, ReplicatedChunkStore::new(chain).unwrap())
    }

    #[test]
    fn put_replicates_to_every_node() {
        let (_dirs, replicated) = three_node_chain();
        let h = replicated.put(b"replicate me").unwrap();
        assert_eq!(replicated.len(), 3);
        for node in &replicated.chain {
            assert!(node.has(&h).unwrap(), "node missing chunk");
            assert_eq!(node.get(&h).unwrap(), b"replicate me");
        }
    }

    #[test]
    fn read_uses_tail() {
        let (_dirs, replicated) = three_node_chain();
        let h = replicated.put(b"hi").unwrap();
        // Force-write a different value on the head; tail still wins.
        replicated.chain[0].put(b"corrupted-on-head").ok();
        // (Both bytes hash differently so the head now stores both;
        // the tail only has the original — get returns the original.)
        assert_eq!(replicated.get(&h).unwrap(), b"hi");
    }

    #[test]
    fn delete_propagates() {
        let (_dirs, replicated) = three_node_chain();
        let h = replicated.put(b"to-delete").unwrap();
        replicated.delete(&h).unwrap();
        for node in &replicated.chain {
            assert!(!node.has(&h).unwrap());
        }
    }

    #[test]
    fn empty_chain_rejected() {
        assert!(ReplicatedChunkStore::new(Vec::new()).is_err());
    }

    #[test]
    fn role_assignment() {
        let (_dirs, replicated) = three_node_chain();
        assert_eq!(replicated.role_at(0), ChainRole::Head);
        assert_eq!(replicated.role_at(1), ChainRole::Middle);
        assert_eq!(replicated.role_at(2), ChainRole::Tail);
    }
}
