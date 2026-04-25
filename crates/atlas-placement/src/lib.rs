//! Chunk placement strategies.
//!
//! Given a chunk hash and a set of candidate chains, decide which chain
//! is responsible for storing the chunk. Three strategies ship today:
//!
//! - [`RoundRobin`] — pure hash-modulo, ignores capacity.
//! - [`CapacityAware`] — weight chains by free bytes; falls back to
//!   round-robin within equal-weight groups.
//! - [`RackAware`] — keep replicas on distinct racks. Used together with
//!   one of the above as the *primary* selector.
//!
//! The trait is intentionally small. It returns chain *indices* into
//! whatever list of chains the caller maintains, so this crate doesn't
//! need to know about `ChunkStore`, network endpoints, or anything else
//! beyond the bookkeeping needed to make a placement decision.

use atlas_core::Hash;
use serde::{Deserialize, Serialize};

/// Metadata about a single replication chain (or storage group).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainInfo {
    pub id: String,
    /// Free capacity in bytes. `u64::MAX` for "unknown / treat as full".
    pub free_bytes: u64,
    /// Rack / failure domain identifier. Two chains with the same rack
    /// share a fault boundary.
    pub rack: String,
}

/// Strategy that maps a chunk hash → an index into a `&[ChainInfo]`.
pub trait Placement: Send + Sync {
    /// Pick the chain index for a primary placement.
    fn primary(&self, chains: &[ChainInfo], hash: &Hash) -> Option<usize>;

    /// Pick `n` distinct chain indices, optionally honouring rack
    /// diversity. Default impl falls back to deterministic order
    /// starting at `primary(...)`.
    fn replicas(&self, chains: &[ChainInfo], hash: &Hash, n: usize) -> Vec<usize> {
        let mut out = Vec::with_capacity(n.min(chains.len()));
        if chains.is_empty() {
            return out;
        }
        let Some(start) = self.primary(chains, hash) else {
            return out;
        };
        for k in 0..chains.len() {
            if out.len() == n {
                break;
            }
            out.push((start + k) % chains.len());
        }
        out
    }
}

/// Hash-modulo selection. Stable across runs as long as the chain list
/// order is stable.
#[derive(Debug, Default, Clone, Copy)]
pub struct RoundRobin;

impl Placement for RoundRobin {
    fn primary(&self, chains: &[ChainInfo], hash: &Hash) -> Option<usize> {
        if chains.is_empty() {
            return None;
        }
        // Use the first 8 bytes of the hash as a u64 mixer.
        let bytes = hash.as_bytes();
        let mut acc: u64 = 0;
        for &b in &bytes[..8] {
            acc = acc.wrapping_mul(131).wrapping_add(b as u64);
        }
        Some((acc as usize) % chains.len())
    }
}

/// Weight chains by free capacity; ties broken by hash-modulo.
#[derive(Debug, Default, Clone, Copy)]
pub struct CapacityAware;

impl Placement for CapacityAware {
    fn primary(&self, chains: &[ChainInfo], hash: &Hash) -> Option<usize> {
        if chains.is_empty() {
            return None;
        }
        let total: u128 = chains.iter().map(|c| c.free_bytes as u128).sum();
        if total == 0 {
            return RoundRobin.primary(chains, hash);
        }
        let bytes = hash.as_bytes();
        let mut mix: u128 = 0;
        for &b in &bytes[..16] {
            mix = mix.wrapping_mul(131).wrapping_add(b as u128);
        }
        let target = mix % total;
        let mut acc: u128 = 0;
        for (i, c) in chains.iter().enumerate() {
            acc += c.free_bytes as u128;
            if target < acc {
                return Some(i);
            }
        }
        Some(chains.len() - 1)
    }
}

/// Wraps another strategy, then expands to `n` replicas while preferring
/// distinct racks.
#[derive(Debug, Clone)]
pub struct RackAware<P: Placement> {
    pub inner: P,
}

impl<P: Placement> RackAware<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

impl<P: Placement> Placement for RackAware<P> {
    fn primary(&self, chains: &[ChainInfo], hash: &Hash) -> Option<usize> {
        self.inner.primary(chains, hash)
    }

    fn replicas(&self, chains: &[ChainInfo], hash: &Hash, n: usize) -> Vec<usize> {
        let mut out = Vec::with_capacity(n.min(chains.len()));
        let Some(primary) = self.inner.primary(chains, hash) else {
            return out;
        };
        out.push(primary);

        let mut used_racks = vec![chains[primary].rack.clone()];

        // First pass: distinct racks.
        for k in 1..chains.len() {
            if out.len() == n {
                break;
            }
            let i = (primary + k) % chains.len();
            if !used_racks.contains(&chains[i].rack) {
                out.push(i);
                used_racks.push(chains[i].rack.clone());
            }
        }
        // Second pass: fill remainder ignoring rack diversity.
        for k in 1..chains.len() {
            if out.len() == n {
                break;
            }
            let i = (primary + k) % chains.len();
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(id: &str, free: u64, rack: &str) -> ChainInfo {
        ChainInfo {
            id: id.into(),
            free_bytes: free,
            rack: rack.into(),
        }
    }

    fn h(seed: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        Hash::from_bytes(bytes)
    }

    #[test]
    fn round_robin_distributes() {
        let cs = vec![
            chain("a", 0, "r1"),
            chain("b", 0, "r2"),
            chain("c", 0, "r3"),
        ];
        let p = RoundRobin;
        let mut counts = [0usize; 3];
        for s in 0..=255u8 {
            counts[p.primary(&cs, &h(s)).unwrap()] += 1;
        }
        for c in counts {
            assert!(c > 0, "round-robin starved a chain");
        }
    }

    #[test]
    fn capacity_aware_prefers_larger() {
        let cs = vec![chain("a", 1, "r1"), chain("b", 1_000_000, "r2")];
        let p = CapacityAware;
        let mut to_b = 0;
        for s in 0..=255u8 {
            if p.primary(&cs, &h(s)).unwrap() == 1 {
                to_b += 1;
            }
        }
        assert!(
            to_b > 200,
            "expected most placements on the bigger chain, got {to_b}"
        );
    }

    #[test]
    fn rack_aware_picks_distinct_racks_first() {
        let cs = vec![
            chain("a", 1, "r1"),
            chain("b", 1, "r1"),
            chain("c", 1, "r2"),
            chain("d", 1, "r3"),
        ];
        let p = RackAware::new(RoundRobin);
        let picks = p.replicas(&cs, &h(0), 3);
        assert_eq!(picks.len(), 3);
        let racks: std::collections::HashSet<_> = picks.iter().map(|&i| &cs[i].rack).collect();
        assert_eq!(racks.len(), 3, "expected 3 distinct racks");
    }

    #[test]
    fn empty_chains_returns_none() {
        assert!(RoundRobin.primary(&[], &h(0)).is_none());
        assert!(CapacityAware.primary(&[], &h(0)).is_none());
    }
}
