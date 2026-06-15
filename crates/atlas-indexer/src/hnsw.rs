//! Hierarchical Navigable Small World (HNSW) approximate nearest-neighbour index.
//!
//! Implements Algorithm 1 (INSERT) and Algorithm 5 (K-NN SEARCH) from:
//!   Malkov & Yashunin, 2018. "Efficient and Robust Approximate Nearest Neighbor
//!   Search Using Hierarchical Navigable Small World Graphs." IEEE TPAMI.
//!
//! Parameters
//! ----------
//! - `M = 16`                — number of neighbours per node per layer
//! - `ef_construction = 200` — candidate list size during construction
//! - `ef_search = 50`        — candidate list size during query
//!
//! Distance
//! --------
//! Negative cosine similarity so that *smaller distance = more similar*.
//! All internal comparisons use this convention; callers receive raw cosine
//! similarity (positive) in the returned tuples.

use std::collections::{BinaryHeap, HashSet};

/// HNSW hyper-parameters.
const M: usize = 16;
const M_MAX0: usize = M * 2; // layer-0 cap is 2xM (standard practice)
const EF_CONSTRUCTION: usize = 200;
pub const EF_SEARCH: usize = 50;

/// A node stored in the index.
struct Node {
    /// Pre-normalised embedding (unit vector).
    embedding: Vec<f32>,
    /// Adjacency lists per layer: `neighbours[layer]` = list of neighbour node ids.
    neighbours: Vec<Vec<usize>>,
}

// ---------------------------------------------------------------------------
// Ordered wrappers for BinaryHeap.
// ---------------------------------------------------------------------------

/// Min-heap entry (smallest `dist` pops first).
#[derive(PartialEq)]
struct MinCand {
    dist: f32,
    id: usize,
}

impl Eq for MinCand {}

impl PartialOrd for MinCand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MinCand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Rust's BinaryHeap is a max-heap; invert to get min-heap behaviour.
        other
            .dist
            .partial_cmp(&self.dist)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Max-heap entry (largest `dist` pops first).
#[derive(PartialEq)]
struct MaxCand {
    dist: f32,
    id: usize,
}

impl Eq for MaxCand {}

impl PartialOrd for MaxCand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MaxCand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.dist
            .partial_cmp(&other.dist)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

// ---------------------------------------------------------------------------
// Public index struct.
// ---------------------------------------------------------------------------

/// Self-contained HNSW index (in-memory; rebuilt at load time from flat store).
pub struct HnswIndex {
    nodes: Vec<Node>,
    /// Id of the node with the highest layer (the global entry point).
    entry_point: Option<usize>,
    /// Highest layer currently occupied.
    max_layer: usize,
    /// LCG state for deterministic random layer assignment.
    rng_state: u64,
}

impl HnswIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            entry_point: None,
            max_layer: 0,
            rng_state: 0x12345678_9abcdef0,
        }
    }

    /// Number of nodes in the index.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    // -----------------------------------------------------------------------
    // LCG PRNG (no external dependencies needed).
    // -----------------------------------------------------------------------

    fn next_f64(&mut self) -> f64 {
        // Numerical Recipes LCG -- wrapping arithmetic on u64.
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // Map upper 53 bits to [0, 1).
        (self.rng_state >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Assign a random layer for a new element.
    /// Formula: `floor(-ln(uniform) / ln(M))` -- standard HNSW level assignment.
    fn random_layer(&mut self) -> usize {
        let u = self.next_f64().max(f64::MIN_POSITIVE);
        (-u.ln() / (M as f64).ln()).floor() as usize
    }

    // -----------------------------------------------------------------------
    // Distance & normalisation helpers.
    // -----------------------------------------------------------------------

    /// Negative cosine similarity between two unit vectors (dot product negated).
    #[inline]
    fn dist(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        -dot.clamp(-1.0, 1.0)
    }

    /// Normalise a vector to unit length; returns a zero-vector unchanged.
    fn normalise(v: &[f32]) -> Vec<f32> {
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < f32::EPSILON {
            return v.to_vec();
        }
        v.iter().map(|x| x / norm).collect()
    }

    // -----------------------------------------------------------------------
    // Algorithm 2: SEARCH-LAYER.
    // -----------------------------------------------------------------------

    /// Greedy beam search at a single graph layer.
    ///
    /// Returns up to `ef` candidates sorted by ascending distance (best first).
    fn search_layer(
        &self,
        query: &[f32],
        entry_ids: &[usize],
        ef: usize,
        layer: usize,
    ) -> Vec<(f32, usize)> {
        let mut visited: HashSet<usize> = HashSet::new();
        let mut candidates: BinaryHeap<MinCand> = BinaryHeap::new();
        let mut w: BinaryHeap<MaxCand> = BinaryHeap::new(); // ef-best set

        for &ep in entry_ids {
            if visited.insert(ep) {
                let d = Self::dist(query, &self.nodes[ep].embedding);
                candidates.push(MinCand { dist: d, id: ep });
                w.push(MaxCand { dist: d, id: ep });
            }
        }

        while let Some(MinCand { dist: c_dist, id: c_id }) = candidates.pop() {
            let f_dist = w.peek().map(|x| x.dist).unwrap_or(f32::MAX);
            if c_dist > f_dist && w.len() >= ef {
                break;
            }
            if layer < self.nodes[c_id].neighbours.len() {
                for &e_id in &self.nodes[c_id].neighbours[layer] {
                    if visited.insert(e_id) {
                        let e_dist = Self::dist(query, &self.nodes[e_id].embedding);
                        let f_dist2 = w.peek().map(|x| x.dist).unwrap_or(f32::MAX);
                        if e_dist < f_dist2 || w.len() < ef {
                            candidates.push(MinCand {
                                dist: e_dist,
                                id: e_id,
                            });
                            w.push(MaxCand {
                                dist: e_dist,
                                id: e_id,
                            });
                            if w.len() > ef {
                                w.pop();
                            }
                        }
                    }
                }
            }
        }

        let mut result: Vec<(f32, usize)> = w.into_iter().map(|c| (c.dist, c.id)).collect();
        result.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        result
    }

    // -----------------------------------------------------------------------
    // Algorithm 4: SELECT-NEIGHBORS-HEURISTIC (pure / static).
    // -----------------------------------------------------------------------

    /// Select at most `m` diverse neighbours from a pre-sorted candidate list.
    ///
    /// `candidates` must be sorted ascending by distance (closest first).
    /// `get_embedding` is a closure that returns the embedding for a given node id;
    /// it is used for the inter-neighbour distance check.
    ///
    /// This is a static method so it can be called from `&mut self` contexts
    /// without conflicting borrows.
    fn select_neighbours_heuristic(
        candidates: &[(f32, usize)],
        m: usize,
        get_embedding: impl Fn(usize) -> Vec<f32>,
    ) -> Vec<usize> {
        let mut result: Vec<usize> = Vec::with_capacity(m);
        let mut discarded: Vec<(f32, usize)> = Vec::new();

        'outer: for &(d_cand, id_cand) in candidates {
            if result.len() >= m {
                break;
            }
            let emb_cand = get_embedding(id_cand);
            for &r_id in &result {
                let emb_r = get_embedding(r_id);
                // Inter-neighbour distance (same negative-cosine metric).
                let dot: f32 = emb_cand.iter().zip(&emb_r).map(|(x, y)| x * y).sum();
                let d_r = -dot.clamp(-1.0, 1.0);
                if d_r < d_cand {
                    // Candidate is eclipsed by an existing result.
                    discarded.push((d_cand, id_cand));
                    continue 'outer;
                }
            }
            result.push(id_cand);
        }

        // Fill remaining slots from discarded list (paper allows this).
        if result.len() < m {
            discarded.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            for (_, id) in discarded {
                if result.len() >= m {
                    break;
                }
                result.push(id);
            }
        }

        result
    }

    // -----------------------------------------------------------------------
    // Neighbour-shrink helper.
    // -----------------------------------------------------------------------

    /// Prune `nodes[node_id].neighbours[layer]` to at most `m_max` entries
    /// using the diversity heuristic.
    fn shrink_connections(&mut self, node_id: usize, layer: usize, m_max: usize) {
        // Guard: nothing to do?
        {
            let node = &self.nodes[node_id];
            if layer >= node.neighbours.len() || node.neighbours[layer].len() <= m_max {
                return;
            }
        }

        // Clone what we need before taking mutable access.
        let query: Vec<f32> = self.nodes[node_id].embedding.clone();
        let current: Vec<usize> = self.nodes[node_id].neighbours[layer].clone();

        // Snapshot embeddings of all current neighbours.
        let nb_embs: Vec<(usize, Vec<f32>)> = current
            .iter()
            .map(|&nb| (nb, self.nodes[nb].embedding.clone()))
            .collect();

        // Score candidates against the query.
        let mut candidates: Vec<(f32, usize)> = nb_embs
            .iter()
            .map(|(nb, emb)| {
                let dot: f32 = query.iter().zip(emb).map(|(x, y)| x * y).sum();
                (-dot.clamp(-1.0, 1.0), *nb)
            })
            .collect();
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // The heuristic only needs the cloned embeddings -- no borrow of self.
        let selected = Self::select_neighbours_heuristic(&candidates, m_max, |id| {
            nb_embs
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, e)| e.clone())
                .unwrap_or_default()
        });
        self.nodes[node_id].neighbours[layer] = selected;
    }

    // -----------------------------------------------------------------------
    // Algorithm 1: INSERT.
    // -----------------------------------------------------------------------

    /// Insert a single embedding into the index.
    pub fn insert(&mut self, embedding: &[f32]) {
        let unit = Self::normalise(embedding);
        let new_id = self.nodes.len();
        let new_layer = self.random_layer();

        self.nodes.push(Node {
            embedding: unit.clone(),
            neighbours: vec![Vec::new(); new_layer + 1],
        });

        let ep = match self.entry_point {
            None => {
                self.entry_point = Some(new_id);
                self.max_layer = new_layer;
                return;
            }
            Some(ep) => ep,
        };

        let current_max = self.max_layer;
        let mut entry_ids = vec![ep];

        // Phase 1: descend from current_max to new_layer+1, tracking single best.
        for layer in (new_layer + 1..=current_max).rev() {
            let results = self.search_layer(&unit, &entry_ids, 1, layer);
            if let Some(&(_, best)) = results.first() {
                entry_ids = vec![best];
            }
        }

        // Phase 2: from min(new_layer, current_max) to 0 with ef_construction.
        for layer in (0..=new_layer.min(current_max)).rev() {
            let m_max = if layer == 0 { M_MAX0 } else { M };

            let candidates = self.search_layer(&unit, &entry_ids, EF_CONSTRUCTION, layer);

            // Snapshot embeddings for the heuristic (immutable borrow snapshot).
            let cand_embs: Vec<(usize, Vec<f32>)> = candidates
                .iter()
                .map(|&(_, id)| (id, self.nodes[id].embedding.clone()))
                .collect();

            let selected = Self::select_neighbours_heuristic(&candidates, M, |id| {
                cand_embs
                    .iter()
                    .find(|(i, _)| *i == id)
                    .map(|(_, e)| e.clone())
                    .unwrap_or_default()
            });

            // Install edges: new_id -> selected.
            self.nodes[new_id].neighbours[layer] = selected.clone();

            // Install edges: selected -> new_id (bidirectional).
            for &nb_id in &selected {
                if layer >= self.nodes[nb_id].neighbours.len() {
                    self.nodes[nb_id].neighbours.resize(layer + 1, Vec::new());
                }
                self.nodes[nb_id].neighbours[layer].push(new_id);
                if self.nodes[nb_id].neighbours[layer].len() > m_max {
                    self.shrink_connections(nb_id, layer, m_max);
                }
            }

            // Advance entry points for the next lower layer.
            entry_ids = candidates.iter().map(|&(_, id)| id).collect();
        }

        // Promote global entry point if the new node lives higher.
        if new_layer > current_max {
            self.entry_point = Some(new_id);
            self.max_layer = new_layer;
        }
    }

    // -----------------------------------------------------------------------
    // Algorithm 5: K-NN SEARCH.
    // -----------------------------------------------------------------------

    /// Find the `k` nearest neighbours of `query`.
    ///
    /// Returns `(cosine_similarity, node_id)` pairs sorted by descending
    /// similarity (best match first).
    pub fn knn_search(&self, query: &[f32], k: usize) -> Vec<(f32, usize)> {
        let ep = match self.entry_point {
            None => return vec![],
            Some(ep) => ep,
        };

        let unit = Self::normalise(query);
        let mut entry_ids = vec![ep];

        // Greedy descent to layer 1.
        for layer in (1..=self.max_layer).rev() {
            let results = self.search_layer(&unit, &entry_ids, 1, layer);
            if let Some(&(_, best)) = results.first() {
                entry_ids = vec![best];
            }
        }

        // Search layer 0 with ef_search candidates.
        let ef = EF_SEARCH.max(k);
        let results = self.search_layer(&unit, &entry_ids, ef, 0);

        // Convert internal distance (-cos) back to cosine similarity.
        results
            .into_iter()
            .take(k)
            .map(|(dist, id)| (-dist, id))
            .collect()
    }
}

impl Default for HnswIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vec(v: &[f32]) -> Vec<f32> {
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / norm).collect()
    }

    #[test]
    fn single_node_search() {
        let mut idx = HnswIndex::new();
        idx.insert(&[1.0f32, 0.0, 0.0]);
        let results = idx.knn_search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results.len(), 1);
        assert!(
            (results[0].0 - 1.0).abs() < 1e-5,
            "cosine sim should be ~1.0, got {}",
            results[0].0
        );
    }

    #[test]
    fn two_node_ordering() {
        let mut idx = HnswIndex::new();
        idx.insert(&[1.0f32, 0.0]);
        idx.insert(&[0.0f32, 1.0]);
        let results = idx.knn_search(&[1.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, 0, "nearest should be id 0");
        assert!((results[0].0 - 1.0).abs() < 1e-5);
        assert_eq!(results[1].1, 1, "second should be id 1");
        assert!(results[1].0.abs() < 1e-5);
    }

    #[test]
    fn large_corpus_self_match() {
        let mut idx = HnswIndex::new();
        let dim = 32usize;
        let mut state = 0xdeadbeef_u64;
        let mut vecs: Vec<Vec<f32>> = Vec::new();
        for _ in 0..200 {
            let mut v = Vec::with_capacity(dim);
            for _ in 0..dim {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let f = ((state >> 33) as f32) / (u32::MAX as f32) * 2.0 - 1.0;
                v.push(f);
            }
            let normed = unit_vec(&v);
            vecs.push(normed.clone());
            idx.insert(&normed);
        }

        // The exact nearest to vecs[7] is itself (id=7).
        let results = idx.knn_search(&vecs[7], 5);
        assert!(!results.is_empty());
        assert_eq!(results[0].1, 7, "self-match should be top-1");
    }

    #[test]
    fn k_larger_than_corpus() {
        let mut idx = HnswIndex::new();
        for i in 0..10usize {
            let mut v = vec![0.0f32; 10];
            v[i] = 1.0;
            idx.insert(&v);
        }
        let mut q = vec![0.0f32; 10];
        q[0] = 1.0;
        let results = idx.knn_search(&q, 20);
        assert!(!results.is_empty());
        assert!(results.len() <= 10);
        assert_eq!(results[0].1, 0, "exact match should be top-1");
    }
}
