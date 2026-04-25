//! Crash-recovery tests for the CRAQ chain.
//!
//! These exercise the *behaviour* the chain primitive promises in the
//! face of partial failures, without standing up a real network. We use
//! a `FlakyChunkStore` wrapper that can be told to fail the next N
//! operations, simulating a downed replica.

use atlas_chunk::{ChunkStore, LocalChunkStore};
use atlas_core::{Error, Hash, Result};
use atlas_replicate::ReplicatedChunkStore;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tempfile::TempDir;

/// Wrapper that fails the next N writes/reads when `arm()` is called.
struct FlakyChunkStore {
    inner: Arc<dyn ChunkStore>,
    fail_writes: AtomicUsize,
    fail_reads: AtomicUsize,
    /// Truncate stored bytes to N when set, simulating a torn write.
    truncate_to: AtomicUsize,
}

impl FlakyChunkStore {
    fn new(inner: Arc<dyn ChunkStore>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            fail_writes: AtomicUsize::new(0),
            fail_reads: AtomicUsize::new(0),
            truncate_to: AtomicUsize::new(usize::MAX),
        })
    }

    fn arm_write_failures(&self, n: usize) {
        self.fail_writes.store(n, Ordering::SeqCst);
    }

    fn arm_read_failures(&self, n: usize) {
        self.fail_reads.store(n, Ordering::SeqCst);
    }

    fn arm_truncation(&self, max_bytes: usize) {
        self.truncate_to.store(max_bytes, Ordering::SeqCst);
    }
}

impl ChunkStore for FlakyChunkStore {
    fn put(&self, bytes: &[u8]) -> Result<Hash> {
        if self.fail_writes.load(Ordering::SeqCst) > 0 {
            self.fail_writes.fetch_sub(1, Ordering::SeqCst);
            return Err(Error::Backend("simulated write failure".into()));
        }
        let cap = self.truncate_to.load(Ordering::SeqCst);
        let to_write = if cap < bytes.len() {
            &bytes[..cap]
        } else {
            bytes
        };
        // Note: hash is computed on the truncated bytes — this models a
        // *successful* short write, which is what we want to test
        // surface against the replicated store's hash-equality check.
        self.inner.put(to_write)
    }

    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        if self.fail_reads.load(Ordering::SeqCst) > 0 {
            self.fail_reads.fetch_sub(1, Ordering::SeqCst);
            return Err(Error::Backend("simulated read failure".into()));
        }
        self.inner.get(hash)
    }

    fn delete(&self, hash: &Hash) -> Result<()> {
        self.inner.delete(hash)
    }

    fn has(&self, hash: &Hash) -> Result<bool> {
        self.inner.has(hash)
    }

    fn verify(&self, hash: &Hash) -> Result<()> {
        self.inner.verify(hash)
    }

    fn size(&self, hash: &Hash) -> Result<u64> {
        self.inner.size(hash)
    }

    fn iter_hashes(&self) -> Result<Vec<Hash>> {
        self.inner.iter_hashes()
    }
}

fn three_flaky_chain() -> (
    Vec<TempDir>,
    Vec<Arc<FlakyChunkStore>>,
    ReplicatedChunkStore,
) {
    let dirs: Vec<TempDir> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let flaky: Vec<Arc<FlakyChunkStore>> = dirs
        .iter()
        .map(|d| {
            let local = Arc::new(LocalChunkStore::open(d.path()).unwrap()) as Arc<dyn ChunkStore>;
            FlakyChunkStore::new(local)
        })
        .collect();
    let chain: Vec<Arc<dyn ChunkStore>> = flaky
        .iter()
        .map(|f| f.clone() as Arc<dyn ChunkStore>)
        .collect();
    let r = ReplicatedChunkStore::new(chain).unwrap();
    (dirs, flaky, r)
}

#[test]
fn put_fails_atomically_when_middle_replica_is_down() {
    let (_dirs, flaky, r) = three_flaky_chain();
    flaky[1].arm_write_failures(1);
    let result = r.put(b"will-not-replicate");
    assert!(result.is_err(), "expected propagation to abort");
    // Head got the chunk; middle and tail did not. Caller's retry will
    // fix it because `put` is content-addressed and idempotent.
    let h = atlas_core::Hash::of(b"will-not-replicate");
    assert!(flaky[0].has(&h).unwrap());
    assert!(!flaky[2].has(&h).unwrap(), "tail should not have written");
}

#[test]
fn put_retry_after_replica_recovers() {
    let (_dirs, flaky, r) = three_flaky_chain();
    flaky[1].arm_write_failures(1);
    assert!(r.put(b"retry-me").is_err());
    // Recovered — retry succeeds and chunk is present everywhere.
    let h = r.put(b"retry-me").unwrap();
    for f in &flaky {
        assert!(f.has(&h).unwrap(), "chunk missing on a replica after retry");
    }
}

#[test]
fn read_falls_through_to_tail_unaffected_by_head_outage() {
    let (_dirs, flaky, r) = three_flaky_chain();
    let h = r.put(b"served-by-tail").unwrap();
    flaky[0].arm_read_failures(10); // disable head reads entirely
                                    // Reads in CRAQ go to tail by design — head being down is fine.
    assert_eq!(r.get(&h).unwrap(), b"served-by-tail");
}

#[test]
fn torn_write_at_head_is_caught_by_hash_mismatch() {
    let (_dirs, flaky, r) = three_flaky_chain();
    // Head will write only the first 4 bytes — the resulting hash
    // differs from middle/tail, so propagation must surface Integrity.
    flaky[0].arm_truncation(4);
    let res = r.put(b"this-string-is-torn");
    assert!(matches!(res, Err(Error::Integrity { .. })), "got {:?}", res);
}

#[test]
fn delete_tolerates_missing_chunks_along_the_chain() {
    let (_dirs, flaky, r) = three_flaky_chain();
    let h = r.put(b"to-be-deleted").unwrap();

    // Manually remove from middle replica only — simulates a stale state
    // after partial-failure replay.
    flaky[1].delete(&h).unwrap();
    assert!(!flaky[1].has(&h).unwrap());

    // delete should still succeed; NotFound on middle is tolerated.
    r.delete(&h).unwrap();
    for f in &flaky {
        assert!(!f.has(&h).unwrap());
    }
}
