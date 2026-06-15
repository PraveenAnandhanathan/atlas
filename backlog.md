# ATLAS Backlog

Findings from the deep audit performed 2026-06-15.  Every item is ground-
truthed against the actual source — not from a first-pass automated sweep.

---

## Phase 1 — Security & Correctness  *(implement immediately)*

| # | Area | Finding | File | Severity |
|---|------|---------|------|----------|
| 1.1 | Auth | **SAML signature-stripping bypass (CVE-class).**  When an IdP cert is configured, `parse_response` only verifies the signature `else if parsed.signed_info_canonical.is_some()`.  An attacker who strips `<ds:SignedInfo>` from the assertion skips verification entirely and impersonates any user. | `atlas-auth/src/saml.rs:111` | 🔴 CRITICAL |
| 1.2 | Auth | **OIDC algorithm confusion.**  Algorithm selection falls through to RS256 silently for any unrecognised header `alg` value.  No explicit rejection of non-RS algorithms or `alg:none`. | `atlas-auth/src/oidc.rs:293-303` | 🟠 HIGH |
| 1.3 | MCP | **Quota / tenant-isolation is dead code.**  `atlas-mcp` has no dependency on `atlas-quota`.  `Enforcer::check_write` and `check_concurrency` are never called on the request path; multi-tenant isolation is unenforced. | `atlas-mcp/Cargo.toml` | 🟠 HIGH |
| 1.4 | Compliance | **Compliance evidence is fabricated.**  `collect_automated()` returns `EvidenceStatus::Collected` for every control, with file paths it never checks exist.  An auditor gets "all controls met" regardless of reality. | `atlas-compliance/src/evidence.rs:54-76` | 🟠 HIGH |
| 1.5 | Semantics | **Embedder fail-fast missing.**  If the embedder service is down, `ingest_file` silently stores empty embeddings and logs only a warning.  Users can ingest millions of documents with no vectors and only discover it when vector search returns nothing. | `atlas-ingest/src/lib.rs:102-111` | 🟠 HIGH |
| 1.6 | Core | **475 `unwrap()` calls in production source.**  Many sit on I/O / lock paths and will `panic!` the process in production.  Worst offenders: atlas-fs (59), atlas-governor (47), atlas-auth (40). | workspace-wide | 🟡 MEDIUM |

---

## Phase 2 — Functional Completeness  *(implement before GA)*

| # | Area | Finding | File | Severity |
|---|------|---------|------|----------|
| 2.1 | Semantics | **Vector index is O(n) brute-force.**  Not DiskANN/HNSW — exhaustive scan over a JSONL file loaded into RAM.  Multi-second latency at 100 k docs; unusable at 1 M.  Explicitly deferred in a comment but not documented for users. | `atlas-indexer/src/vector_store.rs:1-5` | 🟠 HIGH |
| 2.2 | MCP | **6 of 31 MCP tools are hardwired `not_implemented`.**  `fs_read_tensor`, `fs_read_schema`, `semantic_describe`, `semantic_similar`, `semantic_embed`, `policy_set` always return an error.  All six wires (REST, gRPC, S3, A2A, toolwire, MCP) inherit these holes. | `atlas-mcp/src/core.rs:328,402-414,677` | 🟠 HIGH |
| 2.3 | Replication | **Static cluster membership.**  `atlas-replicate` has no dynamic membership, no online reconfiguration, and no node-failure recovery.  Any node loss requires manual intervention. | `atlas-replicate/src/lib.rs:15-16` | 🟠 HIGH |
| 2.4 | Transport | **RDMA transport leaks hardware resources.**  The RDMA feature gate allocates Protection Domains, Queue Pairs, and Memory Regions then silently falls back to TCP.  Every RDMA-enabled deployment leaks kernel resources. | `atlas-transport/src/lib.rs:565-633` | 🟠 HIGH |
| 2.5 | Migration | **Cloud migration sources not implemented.**  S3, GCS, and git-lfs sources return an error ("network clients not yet wired").  Only local ext4 migration works. | `atlas-migrate/src/pipeline.rs:84-89` | 🟡 MEDIUM |
| 2.6 | Chaos | **Chaos framework does not drive a real cluster.**  `ChaosRunner::run()` simulates timing in `dry_run` mode only; the comment "in production this drives a real cluster via gRPC control channels" is aspirational. | `atlas-chaos/src/runner.rs:30` | 🟡 MEDIUM |
| 2.7 | Security | **Redaction uses toy regexes.**  SSN requires hyphen format; unhyphenated SSNs, custom API-key prefixes, and many email forms slip through.  Do not market as DLP. | `atlas-governor/src/redact.rs:42-61` | 🟡 MEDIUM |
| 2.8 | Network | **`atlas-net` has no timeout, retry, or circuit breaker.**  One mutex serialises all RPCs per `ClientRuntime`.  A stalled server blocks all clients indefinitely. | `atlas-net/src/runtime.rs:42-83` | 🟡 MEDIUM |
| 2.9 | Semantics | **Parquet/Arrow/Zarr extraction is metadata-only.**  Only column names are indexed; no row data.  Searching for data semantics fails. | `atlas-ingest/src/formats.rs:301-324` | 🟡 MEDIUM |
| 2.10 | Semantics | **PDF extraction is heuristic.**  Fails on encrypted PDFs, complex layout PDFs, and image-only PDFs.  No clear failure signal; system indexes nothing. | `atlas-ingest/src/formats.rs:177-209` | 🟡 MEDIUM |
| 2.11 | Semantics | **Model switch does not trigger re-embedding.**  Switching models via `/models/switch` does not mark existing embeddings stale, causing mixed-model vector spaces. | `services/embedder/main.py:136-141` | 🟡 MEDIUM |

---

## Phase 3 — Desktop, Scale & Polish  *(complete before "worldwide" claim)*

| # | Area | Finding | File | Severity |
|---|------|---------|------|----------|
| 3.1 | Desktop | **macOS FileProvider is Rust-only scaffolding.**  The `NSFileProviderExtension` Swift `.appex` wrapper that macOS actually invokes does not exist in the repo.  Nothing mounts on a real Mac. | `atlas-fileprovider-mac/` | 🔴 BLOCKS CLAIM |
| 3.2 | Desktop | **GVFS integration is scaffold-only.**  `GVfsBackend` C code lives in `desktop/gvfs-backend/` and KIO worker in `desktop/kio-worker/` — neither directory exists.  Linux desktops never see `atlas://` URIs. | `atlas-gvfs/` | 🔴 BLOCKS CLAIM |
| 3.3 | Desktop | **Windows shell extension has no COM shim.**  The C++ shim that dispatches `QueryContextMenu`/`InvokeCommand` is referenced in comments but does not exist.  Explorer clicks go nowhere. | `atlas-shellext-win/` | 🔴 BLOCKS CLAIM |
| 3.4 | Desktop | **WinFsp directory listing is stubbed.**  `cb_read_directory` writes only an entry count; it does not call `FspFileSystemAddDirInfo`.  Explorer cannot enumerate files inside the mount. | `atlas-wfsp/src/driver.rs:443-455` | 🔴 BLOCKS CLAIM |
| 3.5 | Semantics | **DiskANN/HNSW deferred.**  Replace brute-force linear scan with a real ANN index before advertising semantic search at scale. | `atlas-indexer/src/vector_store.rs` | 🟠 HIGH |
| 3.6 | Network | **`atlas-net` lacks connection pooling and circuit breaker.**  Needed for production distributed operation. | `atlas-net/src/runtime.rs` | 🟠 HIGH |
| 3.7 | Auth | **Session token minting has no enforced entropy.**  `AuthSession::new()` accepts any caller-supplied string.  Need an explicit factory using `OsRng` that enforces 256-bit tokens. | `atlas-auth/src/session.rs:79` | 🟡 MEDIUM |
| 3.8 | Compliance | **No WORM/retention/legal-hold enforcement.**  Compliance reports WORM controls as met but there is no immutable-write enforcement anywhere in the write path. | `atlas-compliance/` | 🟡 MEDIUM |
| 3.9 | MCP | **`fs_read_tensor`, `semantic_similar`, `semantic_embed` not implemented.**  Complete format-aware tensor reads and embedding-based retrieval. | `atlas-mcp/src/core.rs` | 🟡 MEDIUM |
| 3.10 | README | **README phase status overstates completion.**  Phases 6–8 are marked "✅ done" despite desktop being scaffolding, compliance being fake, and chaos not driving real clusters.  Update before public launch. | `README.md` | 🟡 MEDIUM |

---

## Confirmed solid (do not regress)

- Substrate: chunk store, manifests, COW tree, sled meta — real and well-tested.
- Capability tokens: real Ed25519 sign/verify, enforced revocation, strong RNG.
- Audit log: SHA-256 hash chain, persisted, tamper-evident.
- Policy engine: default-deny, wired into every read/write/delete/list path.
- Chunk placement & GC: correct algorithms, well-tested.
- P8 encryption-at-rest: AES-256-GCM + HKDF correctly implemented.
- Protocol wire architecture: one-core-N-wires + conformance harness is genuinely sound.
- Quota logic: `Enforcer::check_write/check_concurrency` correct — just not wired (fixed in P1).

---

*Last updated: 2026-06-15 after deep audit of all 44 crates.*
