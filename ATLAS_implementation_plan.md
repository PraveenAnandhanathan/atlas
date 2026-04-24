# ATLAS — Implementation Action Plan

**Companion to:** [`ATLAS_design_report.md`](./ATLAS_design_report.md)
**Status:** Execution blueprint, v1
**Horizon:** 0–36 months (v1 usable) / 36–48+ months (v2 production)
**Assumed team:** 8–15 engineers at steady state; 3–5 for the first 6 months

---

## Table of contents

1. Goals, non-goals, and success criteria
2. Critical path and strategy
3. Team shape and hiring sequence
4. Workstream decomposition
5. Milestone calendar (quarter by quarter)
6. Detailed task backlog — Phase 0 → Phase 7
7. Dependency graph between components
8. Tech stack decisions (frozen in v1)
9. Repository and monorepo layout
10. Engineering practices and quality bars
11. Testing strategy by layer
12. Benchmarking and performance targets
13. Security, compliance, and threat model
14. Release process and versioning
15. Documentation plan
16. Risk register and mitigations
17. Go/no-go gates between phases
18. Day-1 checklist — what to start Monday
19. Open decisions that block execution

---

## 1. Goals, non-goals, and success criteria

### 1.1 Goals for v1 (month 24)

- Mountable ATLAS volume on Linux, macOS, Windows (FUSE / FileProvider / WinFsp).
- Content-addressed storage with dedup + integrity + CoW versioning.
- Git-like branching, commits, time-travel across arbitrary subtrees.
- Semantic search (hybrid: vector + BM25 + structured) over mounted volumes.
- Lineage recording (implicit process-level + explicit SDK calls).
- Policy engine with capability tokens and read-time redaction.
- Native MCP server exposing the full tool catalog from section 7.1 of the design.
- Python SDK covering the public surface in section 10.4.
- `atlasctl` CLI with every subcommand in section 6.4.
- Single-node (Solo) and small-cluster (Team) deployment modes.

### 1.2 Non-goals for v1

- Native kernel miniport on Windows (FUSE/WinFsp only).
- Native Linux kernel module.
- Multi-region replication.
- SOC 2 certification (readiness yes, audit no).
- A2A adapter beyond a reference implementation.
- GPU embedding cluster autoscaling.
- WebDAV and NFS adapters.

### 1.3 Success criteria (measurable)

| Metric | Target |
|---|---|
| Single-node sequential read | ≥ 80% of raw NVMe throughput |
| Single-node random read (4 KiB) | ≥ 60% of raw NVMe IOPS |
| Small-cluster aggregate throughput | ≥ 50 GB/s on 8-node reference rig |
| Metadata ops/sec (single node) | ≥ 50k stat, ≥ 10k create |
| Branch creation on 10 GB tree | < 100 ms |
| Dedup ratio on synthetic ML dataset | ≥ 95% on checkpoint deltas |
| Semantic query p95 latency (10M docs) | < 500 ms |
| pjdfstest pass rate | ≥ 95% |
| External alpha users | ≥ 5 research/ML teams |

---

## 2. Critical path and strategy

The **critical path** is the shortest sequence from nothing to a usable product:

```
spec freeze → chunk daemon → metadata KV → object model →
FUSE client → CLI → versioning → Python SDK → semantic plane MVP
```

Everything else (governance, MCP, desktop GUI, multi-protocol) is scaffolded *around* the critical path and joins later.

### Strategic principles

1. **Walk the critical path with a single team first.** Resist parallelizing before Phase 1 lands — the spec is still moving.
2. **Every phase ships something end-to-end usable.** No vaporware layers.
3. **POSIX compatibility is the permanent gate.** If a change breaks `cat`, it doesn't merge.
4. **Rust for systems code, Python for ML glue, Swift/C++/C only where the OS forces it.** No language zoo.
5. **One on-disk format, versioned from day 1.** Format migrations ship in every release from v0.2 forward.
6. **Dogfood from month 4.** Team uses ATLAS Solo for its own notes, code, datasets as soon as mount works.

---

## 3. Team shape and hiring sequence

### 3.1 Roles

| Role | Count at steady state | Hire priority |
|---|---|---|
| Storage / distributed-systems engineer (Rust) | 4 | 1 |
| Filesystem / FUSE / kernel engineer | 2 | 1 |
| ML infra engineer (embedders, indexers) | 2 | 2 |
| Platform engineer (CI, release, ops) | 1 | 2 |
| Security / governance engineer | 1 | 3 |
| Desktop / UX engineer (Swift + C++ + TS) | 2 | 3 |
| Protocol / API engineer (MCP, gRPC, REST) | 1 | 3 |
| Tech writer | 1 | 2 |
| Eng manager / architect | 1 | 1 |

### 3.2 Hiring sequence

- **Months 0–3 (founding 3–5):** architect, 2 Rust storage engineers, 1 FS engineer, 1 platform engineer.
- **Months 3–9 (to ~8):** +1 Rust storage, +1 ML infra, +1 tech writer.
- **Months 9–18 (to ~12):** +1 security, +1 desktop, +1 protocol, +1 ML infra.
- **Months 18–24 (to ~15):** +1 desktop, +1 FS/kernel, +1 Rust storage.

---

## 4. Workstream decomposition

Each workstream is owned by one lead and maps to a slice of [section 15](./ATLAS_design_report.md) of the design report.

| # | Workstream | Lead role | Components owned |
|---|---|---|---|
| WS-1 | Substrate & chunks | Storage lead | `atlas-storage`, `atlas-gc`, `atlas-tiering` |
| WS-2 | Metadata & object model | Storage lead | `atlas-meta`, object spec, commit graph |
| WS-3 | Filesystem clients | FS lead | `atlas-fuse`, `atlas-wfsp`, `atlas-fileprovider-mac`, `atlas-gvfs`, `atlas-kio` |
| WS-4 | Intelligence | ML infra lead | `atlas-embedder`, `atlas-indexer`, `atlas-lineage`, format plugins |
| WS-5 | Governance | Security lead | `atlas-governor`, audit, capability tokens, signing |
| WS-6 | Protocols | Protocol lead | `atlas-mcp`, `atlas-a2a`, `atlas-rest`, `atlas-grpc`, `atlas-s3` |
| WS-7 | SDKs & CLI | Platform lead | `atlasctl`, `atlas-sdk-py`, `atlas-sdk-rs`, others later |
| WS-8 | User surfaces | Desktop lead | `atlas-explorer`, shell extensions, web console |
| WS-9 | Infra & ops | Platform lead | CI, release, `atlas-deploy`, `atlas-ops`, `atlas-bench`, `atlas-chaos` |

---

## 5. Milestone calendar (quarter by quarter)

| Quarter | Phase | Shipping milestone |
|---|---|---|
| Q1 (M0–3) | Phase 0 start | Spec freeze v0.1, chunk daemon alpha, CI & repo scaffolding |
| Q2 (M3–6) | Phase 0 end | **Milestone A** — single-node FUSE mount, read/write via CLI |
| Q3 (M6–9) | Phase 1 | Commits, branches, `log`/`diff`/`checkout` |
| Q4 (M9–12) | Phase 1 end + Phase 2 start | **Milestone B** — versioning GA on single node; multi-node chunk replication alpha |
| Q5 (M12–15) | Phase 2 | CRAQ chains, FoundationDB backend, RDMA prototype |
| Q6 (M15–18) | Phase 2 end + Phase 3 | **Milestone C** — 3FS-class throughput on 4-node rig; Semantic plane alpha (text-only) |
| Q7 (M18–21) | Phase 3 + Phase 4 start | Hybrid queries across text+vector+structured; lineage journal |
| Q8 (M21–24) | Phase 4 + Phase 5 start | **Milestone D** — governance GA; MCP server v1; Python SDK v1 |
| Q9 (M24–27) | Phase 5 + Phase 6 start | A2A, REST, gRPC, S3 gateway; WinFsp driver alpha |
| Q10 (M27–30) | Phase 6 | macOS FileProvider; Finder/Explorer extensions; GUI v0.5 |
| Q11 (M30–33) | Phase 6 end | **Milestone E — v1 GA** — desktop-ready ATLAS on all three OSes |
| Q12+ (M33+) | Phase 7 | Production hardening, enterprise auth, chaos, DR, compliance |

---

## 6. Detailed task backlog — Phase 0 → Phase 7

### Phase 0 — Foundation (M0–6)

**Exit criteria:** single-node FUSE mount on Linux, POSIX basics pass, chunks dedup across files.

**Tasks**

- [ ] T0.1 Freeze v0.1 on-disk spec (chunks, blob/file/directory manifests)
- [ ] T0.2 Choose hash function (BLAKE3) and chunk size (4 MiB default) — written ADR
- [ ] T0.3 Scaffolding: monorepo, Rust workspace, CI, lint, test matrix
- [ ] T0.4 `atlas-storage` single-node: put/get/delete/verify chunks over gRPC
- [ ] T0.5 `atlas-meta` v0 on RocksDB: inodes, xattrs, refs
- [ ] T0.6 Object-model library: compose chunks+metadata into file/dir manifests
- [ ] T0.7 `atlas-fuse` minimum ops: getattr, readdir, open, read, write, create, unlink, rename, setxattr, getxattr
- [ ] T0.8 `atlasctl` MVP: mount, ls, stat, cat, cp, mv, rm
- [ ] T0.9 Property tests on object model; fuzz on on-disk spec
- [ ] T0.10 pjdfstest subset CI job

### Phase 1 — Versioning (M6–10)

- [ ] T1.1 Commit record + commit graph in metadata
- [ ] T1.2 Branch creation / deletion / listing (O(1) branch create)
- [ ] T1.3 CoW write path with manifest chaining
- [ ] T1.4 `atlasctl commit | checkout | log | diff | branch`
- [ ] T1.5 Time-travel mount (`atlas://vol/@commit/…` read-only)
- [ ] T1.6 Python SDK v0.1: `atlas.open`, `atlas.branch`, `atlas.commit`
- [ ] T1.7 Dedup benchmark on 10 GB / 1% mutation tree
- [ ] T1.8 Format plugin `atlas-fmt-safetensors` (tensor-slice reads)

### Phase 2 — Distributed (M10–16)

- [ ] T2.1 Multi-node `atlas-storage` with CRAQ chain replication
- [ ] T2.2 Chunk placement policy (capacity-aware, rack-aware stub)
- [ ] T2.3 FoundationDB-backed metadata adapter; dual-backend behind trait
- [ ] T2.4 RDMA transport (RoCE v2 first, IB later); TCP fallback
- [ ] T2.5 `atlas-gc` background chunk GC with refcount journal
- [ ] T2.6 `atlas-bench` suite vs NFS, ext4, 3FS where possible
- [ ] T2.7 Crash-recovery tests for torn writes and replica drops
- [ ] T2.8 `atlas-deploy` installer for single-node and 3-node cluster

### Phase 3 — Semantic (M14–20, overlaps P2)

- [ ] T3.1 `atlas-embedder` service (Python, GPU-aware), model registry
- [ ] T3.2 `atlas-indexer` with DiskANN + Tantivy + structured KV index
- [ ] T3.3 Ingest pipeline: format detect → extract → chunk → embed → index
- [ ] T3.4 Format plugins: parquet, pdf, docx, jsonl, image, audio, zarr, arrow
- [ ] T3.5 Hybrid query API (`vector AND keyword AND xattr filter`)
- [ ] T3.6 `atlas.semantic.*` SDK and `atlasctl find`
- [ ] T3.7 Re-embedding job framework (model-version tagging)
- [ ] T3.8 Policy-aware query filter (don't surface unreadable results)

### Phase 4 — Lineage and governance (M18–24)

- [ ] T4.1 `atlas-lineage` edge journal + graph query service
- [ ] T4.2 Implicit tracking at FUSE layer; explicit `atlas.lineage.record`
- [ ] T4.3 Sampling controls + lineage rollups
- [ ] T4.4 `atlas-governor` policy engine with evaluate-at-open
- [ ] T4.5 Capability-token issuance, verification, revocation
- [ ] T4.6 Read-time redaction (names/emails/SSNs/API keys detectors)
- [ ] T4.7 Lineage-constraint enforcement on write
- [ ] T4.8 Merkle-tree audit log, export tooling
- [ ] T4.9 Commit and policy signing (local key + KMS)

### Phase 5 — MCP and protocols (M22–28)

- [ ] T5.1 `atlas-mcp` server with full tool catalog from design §7.1
- [ ] T5.2 Subtree-scoped MCP serve (`atlasctl mcp serve <path>`)
- [ ] T5.3 `atlas-a2a` agent adapter — reference integration
- [ ] T5.4 `atlas-rest` + OpenAPI spec
- [ ] T5.5 `atlas-grpc` service definitions + reflection
- [ ] T5.6 `atlas-s3` gateway (S3 v4 signing, bucket=volume, key=path)
- [ ] T5.7 Anthropic tool-use / OpenAI function-calling JSON adapters on top of MCP
- [ ] T5.8 Adapter conformance test harness (one capability, N wire formats)

### Phase 6 — Desktop integration (M26–34)

- [ ] T6.1 Windows WinFsp-based driver (`atlas-wfsp`) — drive letter + mount point
- [ ] T6.2 `atlas-shellext-win` — columns + context menu
- [ ] T6.3 macOS FileProvider extension (`atlas-fileprovider-mac`)
- [ ] T6.4 Finder Sync + Quick Look generators for safetensors/parquet/embeddings
- [ ] T6.5 `atlas-gvfs` + `atlas-kio` Linux file-manager integrations
- [ ] T6.6 `atlas-explorer` GUI (Tauri + TypeScript) — browser, search, lineage, version, policy tabs
- [ ] T6.7 Onboarding: installer wizards, first-mount flow, sample data
- [ ] T6.8 `atlas-web` admin console (for Team/Scale mode)

### Phase 7 — Production hardening (M32–48+)

- [ ] T7.1 `atlas-chaos` fault-injection framework + nightly runs
- [ ] T7.2 Backup, snapshot export, cross-region replication
- [ ] T7.3 Enterprise auth: OIDC, SAML, SCIM
- [ ] T7.4 SOC 2 / ISO 27001 control readiness
- [ ] T7.5 Disaster recovery runbooks + game days
- [ ] T7.6 Per-workload performance tuning (training, inference, build)
- [ ] T7.7 Multi-tenant quotas, isolation, noisy-neighbor controls
- [ ] T7.8 Long-form migration tools from S3 / GCS / ext4 / git-LFS

---

## 7. Dependency graph between components

```
atlas-storage ──┬──> atlas-meta ──> object-model ──┬──> atlas-fuse ──> atlasctl
                │                                  │
                │                                  ├──> atlas-sdk-py ──> semantic SDK
                │                                  │
                └──> atlas-gc                      └──> atlas-lineage ──┐
                                                                       │
atlas-embedder ──> atlas-indexer ──> semantic queries ──────────────── │ ──> atlas-governor
                                                                       │
atlas-mcp ──> protocol adapters (a2a, rest, grpc, s3) <────────────────┘
                                                  │
atlas-explorer ──> shell extensions ──────────────┘
```

Hard ordering:
1. spec → storage → meta → object model → fuse → cli
2. object model → versioning → python sdk
3. object model → embedder/indexer → semantic
4. meta + object model → lineage → governor
5. governor → mcp (tokens) → other protocol adapters
6. fuse/wfsp/fileprovider → explorer → shell extensions

---

## 8. Tech stack decisions (frozen in v1)

| Area | Choice | Rationale |
|---|---|---|
| Systems language | Rust | Memory safety at FS-daemon scale |
| ML glue | Python 3.12+ | Ecosystem fit |
| Metadata KV (cluster) | FoundationDB | Proven transactional scale (same as 3FS) |
| Metadata KV (single) | RocksDB | Embedded, same schema |
| Hash | BLAKE3 | Speed + tree-hash properties |
| Chunk size default | 4 MiB | Balance dedup vs metadata overhead |
| Replication | CRAQ chains | Strong consistency + parallel reads |
| RPC | gRPC over HTTP/2 + Protobuf | Toolchain, streaming, reflection |
| Vector index | DiskANN (large) / HNSW (small) | Disk-resident scale + in-mem speed |
| Text index | Tantivy | Rust-native, BM25F |
| Windows FS | WinFsp v1 → native miniport v2 | Ship now, optimize later |
| macOS FS | FileProvider extension | Modern, supported, sparse-capable |
| Linux FS | FUSE v1 → optional kmod | Portability first |
| GUI | Tauri + TypeScript | Small footprint, cross-platform |
| Signing | Sigstore-compatible (cosign lineage) | Industry standard |
| Policy language | YAML with JSON Schema validation | Human-editable, machine-checkable |
| CI | GitHub Actions + self-hosted runners for cluster tests | Meets repo choice |

ADRs (architecture decision records) land in `docs/adr/` the week each choice is made.

---

## 9. Repository and monorepo layout

Single monorepo. Trunk-based development on `main`, short-lived feature branches.

```
atlas/
├── README.md
├── ATLAS_design_report.md          # design whitepaper (this companion)
├── ATLAS_implementation_plan.md    # this file
├── LICENSE
├── Cargo.toml                      # Rust workspace root
├── pyproject.toml                  # Python SDK root
├── docs/
│   ├── adr/                        # architecture decision records
│   ├── spec/                       # on-disk format versions
│   └── runbooks/
├── crates/
│   ├── atlas-storage/
│   ├── atlas-meta/
│   ├── atlas-object/
│   ├── atlas-fuse/
│   ├── atlas-indexer/
│   ├── atlas-lineage/
│   ├── atlas-governor/
│   ├── atlas-mcp/
│   ├── atlas-rest/
│   ├── atlas-grpc/
│   ├── atlas-s3/
│   ├── atlas-gc/
│   ├── atlas-tiering/
│   ├── atlas-bench/
│   ├── atlas-chaos/
│   └── atlasctl/
├── clients/
│   ├── py/                         # atlas-sdk-py
│   ├── rs/                         # atlas-sdk-rs
│   ├── c/                          # atlas-sdk-c
│   ├── js/                         # atlas-sdk-js
│   └── go/                         # atlas-sdk-go
├── desktop/
│   ├── explorer/                   # Tauri + TS
│   ├── win-shellext/               # C++
│   ├── mac-fileprovider/           # Swift
│   ├── nautilus-ext/               # Python
│   └── dolphin-ext/                # C++
├── services/
│   ├── embedder/                   # Python, GPU
│   └── web/                        # admin console
├── formats/
│   ├── parquet/
│   ├── safetensors/
│   ├── pdf/
│   ├── docx/
│   ├── jsonl/
│   ├── arrow/
│   ├── zarr/
│   ├── image/
│   └── audio/
├── deploy/
│   ├── solo/
│   ├── team/
│   └── scale/
├── tests/
│   ├── e2e/
│   ├── pjdfstest/
│   ├── fuzz/
│   └── chaos/
└── .github/workflows/
```

---

## 10. Engineering practices and quality bars

- **Trunk-based development.** Short-lived branches, required reviews, squash-merge.
- **Conventional commits** enforced in CI (`feat:`, `fix:`, `spec:` …).
- **Two-person review** for anything touching on-disk format, security, or signing.
- **ADR for every non-trivial decision**, stored in `docs/adr/NNNN-title.md`.
- **Every PR passes:** lint, unit, integration, POSIX subset, fuzz smoke, docs build.
- **Format versioning:** `spec: vMAJOR.MINOR`; migrations required before merging breaking changes.
- **Audit-logged everything** in the governor from the first release that ships it.
- **Feature flags** for user-visible surfaces — off by default until the success criterion is met.
- **Deprecations announced one minor version ahead** and tracked in `docs/deprecations.md`.

---

## 11. Testing strategy by layer

| Layer | Test types |
|---|---|
| Chunk layer | Unit, property (round-trip hash), fuzz (byte-level), chaos (disk kill) |
| Metadata | Unit, property (commutative ops), Jepsen-style linearizability |
| Object model | Property tests on CoW invariants, snapshot fuzz |
| FUSE client | pjdfstest, xfstests subset, stress (fio, ior) |
| Versioning | Dedup ratio, branch creation latency, time-travel correctness |
| Semantic | Recall@k on labeled corpora; latency p50/p95/p99 |
| Lineage | Completeness under sampled vs full modes |
| Governance | Policy eval correctness, redaction leak tests, capability-scope escape tests |
| Protocols | Conformance per spec + adapter-cross tests |
| Desktop | Manual QA matrix on Win/macOS/Linux versions per release |

Fuzz and chaos run **nightly** starting Phase 2. Benchmarks run **on every RC tag**.

---

## 12. Benchmarking and performance targets

Reference hardware for each target:

- **Solo bench rig:** 16-core laptop, 64 GB RAM, 2 TB NVMe.
- **Team bench rig:** 4 nodes, each 32-core / 256 GB / 2×NVMe, 25 GbE.
- **Scale bench rig:** 8 nodes, each 64-core / 512 GB / 4×NVMe, 100 GbE RDMA.

Workloads:

1. Sequential read/write (large files, fio).
2. Random 4 KiB read (training-style).
3. Checkpoint write (striped, strong consistency).
4. Metadata storm (10M file creates).
5. Dedup scenario (checkpoint deltas).
6. Semantic query mix at 1M / 10M / 100M documents.
7. Small-file storm (WebDataset-style) — known weak point, track specifically.

Targets live in [section 1.3](#13-success-criteria-measurable) and are re-baselined every minor release.

---

## 13. Security, compliance, and threat model

### 13.1 Threat model

- **Untrusted agents** with time-boxed capability tokens: must not escape scope.
- **Compromised storage node:** cannot forge chunks (content-addressed) or commits (signed).
- **Compromised client:** cannot exfiltrate policy-protected content beyond its capabilities; redaction applies in-kernel path.
- **Insider admin:** can disable enforcement locally, but every disable is audit-logged and tamper-evident.
- **Network attacker:** mTLS on all RPC, RDMA over authenticated transport.

### 13.2 Compliance

- **SOC 2 Type I readiness** by end of Phase 6 (controls, not certification).
- **GDPR** handled via retention + redaction policies; delete-after enforced.
- **SPDX license metadata** preserved end-to-end for downstream reproducibility.

### 13.3 Key management

- Solo: OS keyring.
- Team: HashiCorp Vault or cloud KMS.
- Scale: HSM-backed KMS + key rotation runbook.

---

## 14. Release process and versioning

- **On-disk format** uses `vMAJOR.MINOR`. New minor versions are forward-compatible; new majors require migration.
- **Software** uses SemVer per crate/SDK; public APIs stable at v1.0 (Milestone D onward).
- **Release cadence:** monthly pre-1.0; quarterly after v1 GA.
- **Release channels:** `dev` (nightly), `beta` (RC), `stable`, `lts`.
- **Every release:** changelog, migration note (if any), benchmark delta, security advisory list.

---

## 15. Documentation plan

- `docs/spec/` — on-disk format, versioned.
- `docs/adr/` — architecture decision records.
- `docs/guide/` — user guide, per-OS install, CLI manual.
- `docs/sdk/` — generated API docs for Python, Rust, C, JS, Go.
- `docs/agents/` — MCP tool reference, A2A capability card.
- `docs/runbooks/` — operations, DR, upgrade.
- `docs/benchmarks/` — latest numbers + methodology.
- Sample notebooks live in `examples/` and are exercised by CI.

---

## 16. Risk register and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Metadata plane doesn't scale to billions of small files | High | High | Packed-object primitive on roadmap; FoundationDB already 3FS-proven; bench at 100M early |
| Embedding cost blows up | Med | High | Lazy/on-first-query + subtree opt-in + tiered embedders |
| FUSE perf ceiling blocks training use-cases | Med | High | Benchmark early; fast path via SDK bypasses VFS for tensor slicing |
| Desktop-OS integration slips (esp. Windows miniport) | High | Med | WinFsp in v1; miniport is v2 and optional |
| Protocol sprawl (MCP, A2A, next) | High | Med | Thin adapters, single capability model, conformance tests |
| Governance perceived as DRM | Med | Med | Local admin override with audit marker; public policy of transparency |
| Key management complexity | Med | High | Pick one KMS per deployment mode; rotation runbook before v1 GA |
| On-disk format churn | Med | High | Spec freeze gate at each milestone; migrations required before breaking |
| Talent scarcity (Rust+FS+RDMA+ML) | High | High | Stagger hiring; pair senior with mid; open-source visibility as recruiting lever |
| Dogfooding exposes unacceptable perf | Med | Med | Dogfood from M4; dedicated perf fix budget every quarter |

---

## 17. Go/no-go gates between phases

| Gate | From → To | Must be true |
|---|---|---|
| G1 | Phase 0 → 1 | FUSE mount passes pjdfstest subset on Linux; dedup demonstrated |
| G2 | Phase 1 → 2 | Branch on 10 GB tree < 100 ms; time-travel correctness tests green |
| G3 | Phase 2 → 3 | ≥ 50 GB/s aggregate on 8-node rig; Jepsen-class tests pass |
| G4 | Phase 3 → 4 | Hybrid query p95 < 500 ms at 10M docs; recall@10 within 5% of Faiss baseline |
| G5 | Phase 4 → 5 | Capability-scope escape tests clean; audit log tamper-evident |
| G6 | Phase 5 → 6 | MCP tool catalog covers design §7.1; adapter conformance green |
| G7 | Phase 6 → 7 (v1 GA) | Desktop mount on Win/macOS/Linux; 5 external alpha users retained ≥ 30 days |

A gate failure triggers a focused remediation sprint, not a phase skip.

---

## 18. Day-1 checklist — what to start Monday

1. Create GitHub org protections on `PraveenAnandhanathan/atlas` (branch protection, required reviews, signed commits).
2. Commit this plan and the design report; open `docs/adr/0001-record-architecture-decisions.md`.
3. File a tracking issue for each Phase 0 task (T0.1–T0.10); tag `phase-0`.
4. Publish a 1-page charter (scope, non-goals, success criteria) at `docs/CHARTER.md`.
5. Stand up CI: Rust workspace, rustfmt+clippy, cargo-deny, cargo-fuzz smoke.
6. Start the on-disk spec doc at `docs/spec/v0.1-draft.md`.
7. Open a hiring rec for storage engineer #1 and #2.
8. Schedule weekly architecture review (1 hr) and monthly spec-change review.
9. Pick the first dogfood target (personal notes / small dataset) — stub where it will live.
10. Write `CONTRIBUTING.md` with ADR template, commit convention, review rules.

---

## 19. Open decisions that block execution

These must be decided before Phase 0 exits. Each becomes an ADR.

- **Erasure coding vs replication** default for the chunk layer.
- **Chunk packing** strategy for small-file scenarios (inline? sealed packs?).
- **KV schema** in the metadata plane — exact key encoding and index layout.
- **Snapshot semantics** — subtree commits vs volume commits vs both.
- **Redaction engine** — regex-first vs model-first PII detection.
- **MCP transport** — local Unix socket + TLS TCP + optional WS: confirm auth handshake details.
- **License choice for the project** (Apache 2.0 proposed; confirm with legal).
- **Telemetry** — opt-in anonymous usage beacons yes/no for Solo mode.

---

**End of plan.** This is the operational companion to the design report. Updated per phase; every milestone revises its successor.
