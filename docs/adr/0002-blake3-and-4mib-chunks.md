# ADR-0002: BLAKE3-256 as the content hash; 4 MiB default chunk size

**Date:** 2026-04-25
**Status:** Accepted

## Context

The substrate is content-addressed. Every chunk, manifest, and commit is named by a cryptographic digest. We must pick:

1. A hash function.
2. A default chunk size.

Both decisions shape on-disk format and throughput; both are hard to change later.

## Decision

### Hash function: **BLAKE3-256** (32-byte output).

Reasons:
- ~1 GB/s on a single core with SIMD, 5–10× faster than SHA-256 on modern x86/ARM.
- Tree-structured — parallelizable and supports incremental verification of sub-ranges.
- Cryptographically strong (based on Bao / BLAKE2); 128-bit security against collisions.
- Active maintenance, well-audited reference implementation.

Rejected alternatives:
- **SHA-256**: universal but ~3× slower; no inherent parallelism.
- **xxHash / Highway**: non-cryptographic; a compromised client could produce collisions.
- **SHA-3**: slower than SHA-256 without operational advantages for our use case.

### Chunk size: **4 MiB default**, configurable per volume.

Reasons:
- Balances dedup granularity against metadata overhead. At 4 MiB, 1 TiB ≈ 262,144 chunks — manageable in the metadata plane.
- Aligns comfortably with NVMe block erase boundaries and RDMA message sizes.
- Matches observed sweet-spot in IPFS, restic, and S3 multipart upload defaults.

Smaller chunks (64 KiB–1 MiB) improve dedup but explode metadata; larger (16–64 MiB) reduce dedup hit-rate on small edits. A per-volume setting in `StoreConfig` lets specialized workloads (e.g. small-file-heavy) tune this.

## Consequences

- The on-disk spec (v0.1) hardcodes BLAKE3 and a 32-byte hash. Changing it is a MAJOR format bump.
- Chunk size is configurable but tools assume the default for benchmarks and test vectors.
- Every implementation (including future Go/Rust/C clients) must use the same BLAKE3 parameters — no keyed mode, no derived-key mode, no XOF extension.
