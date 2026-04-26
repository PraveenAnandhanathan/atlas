# ATLAS

**A**ddressable, **T**ype-aware, **L**ineage-tracked, **A**uditable **S**torage —
a content-addressed, versioned filesystem for AI-era data.

> **Status:** Phases 0 – 5 complete. Single-node substrate, distributed plane, semantic plane, governance plane, and the full protocol surface (MCP / A2A / REST / gRPC / S3 / Anthropic & OpenAI tool-use) are wired and tested. Desktop integration (Phase 6) and production hardening (Phase 7) are next.

The full vision is in
[ATLAS_design_report.md](ATLAS_design_report.md); the build plan is in
[ATLAS_implementation_plan.md](ATLAS_implementation_plan.md). The
authoritative on-disk format is [docs/spec/v0.1.md](docs/spec/v0.1.md);
key decisions live in [docs/adr/](docs/adr/).

## What works today

### Phase 0 – Substrate
- **Content-addressed chunk store** with BLAKE3-256 + 4 MiB default chunks
  ([ADR-0002](docs/adr/0002-blake3-and-4mib-chunks.md)).
- **Self-hashing manifests** for blobs, files, directories, and commits,
  serialized canonically via bincode v1 legacy.
- **Sled-backed metadata** behind a `MetaStore` trait so RocksDB or
  FoundationDB can drop in later
  ([ADR-0003](docs/adr/0003-sled-as-initial-metadata-backend.md)).
- **Copy-on-write tree mutation** — untouched subtrees share storage.
- **`atlasctl` CLI** for every primitive.

### Phase 1 – Versioning
- `commit`, `branch create/list/delete`, `checkout`, `log`, `diff`
  (between commits or trees).
- **Python SDK** (`atlas-sdk`).
- **Safetensors header parser** (`atlas-fmt-safetensors`) for tensor-aware reads.

### Phase 2 – Distributed
- Wire protocol, **CRAQ-style replication**, capacity/rack-aware placement,
  background **chunk GC** with refcount journal.
- Multi-node `atlas-storage`, `atlas-deploy` installer for single-node and
  3-node clusters, crash-recovery tests, FUSE adapter, **PyO3 native binding**.

### Phase 3 – Semantic plane
- `atlas-embedder` service + model registry, `atlas-indexer` (DiskANN +
  Tantivy + structured KV), ingest pipeline (detect → extract → chunk →
  embed → index).
- Format plugins (parquet, pdf, docx, jsonl, image, audio, zarr, arrow),
  hybrid query (`vector AND keyword AND xattr`), `atlas.semantic.*` SDK,
  `atlasctl find`, re-embedding jobs, policy-aware filtering.

### Phase 4 – Lineage and governance
- `atlas-lineage` edge journal + graph query, implicit FUSE-layer tracking
  plus explicit `atlas.lineage.record`, sampling and rollups.
- `atlas-governor` policy engine (evaluate-at-open), capability tokens
  with revocation, **read-time redaction** (PII detectors),
  lineage-constraint enforcement on write.
- **Merkle-tree audit log**, commit and policy signing (local key + KMS).

### Phase 5 – MCP and protocol surface
- **`atlas-mcp`** — capability catalog (30 tools from design §7.1) and
  `CapabilityCore` dispatcher with policy/audit/redaction/scope, JSON-RPC
  2.0 over stdio.
- **`atlasctl mcp serve [--path]`** — subtree-scoped stdio MCP server.
- **`atlas-a2a`** — agent card + `tasks/send`.
- **`atlas-rest`** — HTTP handler + OpenAPI 3.1 spec generator.
- **`atlas-grpc`** — invoke RPC + reflection descriptor with canonical
  status codes.
- **`atlas-s3`** — bucket = volume / key = path gateway with SigV4
  verify/sign and ListObjectsV2 XML.
- **`atlas-toolwire`** — Anthropic tool-use and OpenAI function-call
  adapters (dot↔underscore name normalisation).
- **`atlas-conformance`** — drives every probe through all six wires
  and asserts they agree.

## Not yet wired

- Windows WinFsp driver, macOS FileProvider, Finder/Explorer extensions,
  Tauri GUI (Phase 6).
- Chaos framework, enterprise auth, DR, compliance work (Phase 7).

## Repo layout

```
crates/
  atlas-core/             shared types — Hash, errors, time, format version
  atlas-chunk/            content-addressed chunk store + LocalChunkStore
  atlas-object/           manifests + canonical serialization + self-hash
  atlas-meta/             MetaStore trait + sled backend
  atlas-fs/               filesystem engine — put/get/list/rename/delete
  atlas-version/          commits, branches, log, diff, checkout
  atlas-fuse/             FUSE adapter
  atlas-fmt-safetensors/  safetensors header parser

  atlas-storage/          distributed chunk service (CRAQ replication)
  atlas-placement/        rack/capacity-aware chunk placement
  atlas-gc/               background chunk GC + refcount journal
  atlas-deploy/           single-node + 3-node deployment

  atlas-embedder/         model registry + embedding service
  atlas-indexer/          DiskANN + Tantivy + structured KV index
  atlas-ingest/           detect → extract → chunk → embed → index pipeline

  atlas-lineage/          edge journal + graph query
  atlas-governor/         policy engine, capability tokens, redaction, audit

  atlas-mcp/              capability catalog + JSON-RPC wire (Phase 5)
  atlas-a2a/              agent-to-agent adapter
  atlas-rest/             REST + OpenAPI 3.1
  atlas-grpc/             gRPC service + reflection
  atlas-s3/               S3 v4 gateway
  atlas-toolwire/         Anthropic / OpenAI tool-use adapters
  atlas-conformance/      one capability, N wire formats — assertion harness

  atlasctl/               CLI binary (now ships `mcp serve`)
clients/
  py/                     atlas-sdk — Python SDK (PyO3 native binding)
docs/
  spec/v0.1.md            on-disk format spec
  adr/                    architecture decisions
```

## Build

```bash
cargo build --workspace --release
cargo test  --workspace
```

A stable Rust toolchain (>= 1.75) is enough on Linux, macOS, and Windows.

## Try it

```bash
# create a store
./target/release/atlasctl --store ./demo init

# write some files
echo "hello"   | ./target/release/atlasctl --store ./demo put /greeting.txt
echo "weights" | ./target/release/atlasctl --store ./demo put /models/v1/w.bin

# inspect
./target/release/atlasctl --store ./demo ls /
./target/release/atlasctl --store ./demo cat /greeting.txt

# version
./target/release/atlasctl --store ./demo commit \
    --message "initial commit" \
    --author-name "you" --author-email "you@example.com"

./target/release/atlasctl --store ./demo branch create experiment-1
./target/release/atlasctl --store ./demo checkout experiment-1
echo "world" | ./target/release/atlasctl --store ./demo put /greeting.txt
./target/release/atlasctl --store ./demo commit -m "swap greeting"

./target/release/atlasctl --store ./demo log --limit 5
./target/release/atlasctl --store ./demo diff --from main --to experiment-1
./target/release/atlasctl --store ./demo verify

# expose the store as an MCP server (subtree-scoped)
./target/release/atlasctl --store ./demo mcp serve --path /models
```

## Talk to ATLAS over a protocol

The same capability catalog is reachable over six wire formats — pick
whichever your agent or client already speaks:

| Adapter            | Crate                | Shape                                    |
|--------------------|----------------------|------------------------------------------|
| MCP (JSON-RPC 2.0) | `atlas-mcp`          | `tools/list`, `tools/call`               |
| A2A                | `atlas-a2a`          | agent card + `tasks/send`                |
| REST + OpenAPI     | `atlas-rest`         | `POST /v1/tools/{capability}`            |
| gRPC               | `atlas-grpc`         | `Invoke(capability, principal, args)`    |
| S3 v4              | `atlas-s3`           | bucket = volume, key = path              |
| Anthropic / OpenAI | `atlas-toolwire`     | tool-use / function-calling JSON         |

`atlas-conformance` is the executable form of "one capability, N wire
formats" — every release runs the same probes through all six adapters
and asserts they return identical results.

## Python SDK

```bash
pip install -e clients/py
```

```python
import atlas

store = atlas.Store("./demo")
store.write("/data/sample.txt", b"hello")
print(store.read("/data/sample.txt"))

with store.branch("experiment-1") as b:
    store.write("/data/sample.txt", b"world")
    b.commit("retrain on new sample")

for entry in store.list("/data"):
    print(entry.path, entry.hash, entry.size)
```

## Roadmap snapshot

| Phase | Theme | Status |
|-------|-------|--------|
| 0 | Substrate — chunk store, manifests, FUSE plumbing, `atlasctl` | ✅ done |
| 1 | Versioning — commits, branches, diff, Python SDK, safetensors | ✅ done |
| 2 | Distribution — gRPC, replication, placement, GC, PyO3 binding | ✅ done |
| 3 | Semantics — embeddings, indexer, ingest, hybrid query | ✅ done |
| 4 | Lineage + governance — journal, policy, redaction, audit, signing | ✅ done |
| 5 | MCP and protocol surface — MCP/A2A/REST/gRPC/S3/tool-use + conformance | ✅ done |
| 6 | Desktop integration — WinFsp, FileProvider, Finder/Explorer, Tauri GUI | next |
| 7 | Production hardening — chaos, enterprise auth, DR, compliance | future |

See [ATLAS_implementation_plan.md](ATLAS_implementation_plan.md) for the
full task breakdown and dependency graph.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). The short version: keep the
public surface stable across phase boundaries, and don't merge changes
that the spec doesn't cover.

## License

Apache-2.0 — see [LICENSE](LICENSE).
