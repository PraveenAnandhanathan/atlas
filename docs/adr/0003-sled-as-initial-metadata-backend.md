# ADR-0003: sled as the initial metadata backend; RocksDB and FoundationDB behind a trait

**Date:** 2026-04-25
**Status:** Accepted

## Context

The plan (section 8) picks **RocksDB** for single-node and **FoundationDB** for clusters. Both are the right long-term choices — RocksDB for throughput on one machine, FoundationDB for transactional distributed state (the same rationale 3FS uses).

However, both carry build-system overhead we want to avoid in Phase 0:
- **RocksDB** via `rocksdb` crate requires a C++ toolchain (CMake, LLVM) on every developer machine and every CI runner. On Windows this is particularly fragile.
- **FoundationDB** requires a running cluster even for unit tests; integration testing is heavy.

Phase 0's goal is "single-node FUSE mount, passes pjdfstest subset." We need a metadata KV that installs in `cargo build` with zero native deps, has transactional semantics, and is replaceable later.

## Decision

Phase 0 and Phase 1 use **`sled`** as the metadata backend, accessed through a `MetaStore` trait.

- `sled` is pure Rust, no C++ toolchain needed.
- Provides ordered KV with atomic batches (close enough to our transactional needs for single-node).
- We define `MetaStore` so Phase 2 can add `RocksDbStore` and `FoundationDbStore` without touching call sites.

Phase 2 (T2.3) replaces `sled` with RocksDB for single-node and FoundationDB for cluster. At that point `sled` is kept as an optional cargo feature for dev-loop ergonomics or retired.

## Consequences

- Developer onboarding needs zero native deps until Phase 2.
- On-disk format of the metadata store is *not* `sled`'s format — we own serialization. Values are bincode-encoded per [spec v0.1](../spec/v0.1.md) §10.
- All metadata access goes through `MetaStore` methods. No call site touches `sled::Db` directly.
- `sled` known issue: higher disk usage and slower crash recovery than RocksDB. Acceptable at Phase 0 scale.
- This ADR is **superseded by a future ADR** when RocksDB lands, not silently edited.
