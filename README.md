# ATLAS

**A**ddressable, **T**ype-aware, **L**ineage-tracked, **A**uditable **S**torage —
a content-addressed, versioned filesystem for AI-era data.

> **Status:** Phase 0 + Phase 1 — single-node implementation. Not yet production-ready.

The full vision is in
[ATLAS_design_report.md](ATLAS_design_report.md); the build plan is in
[ATLAS_implementation_plan.md](ATLAS_implementation_plan.md). The
authoritative on-disk format is [docs/spec/v0.1.md](docs/spec/v0.1.md);
key decisions live in [docs/adr/](docs/adr/).

## What works today

- **Content-addressed chunk store** with BLAKE3-256 + 4 MiB default chunks
  ([ADR-0002](docs/adr/0002-blake3-and-4mib-chunks.md)).
- **Self-hashing manifests** for blobs, files, directories, and commits,
  serialized canonically via bincode v1 legacy.
- **Sled-backed metadata** behind a `MetaStore` trait so RocksDB or
  FoundationDB can drop in later
  ([ADR-0003](docs/adr/0003-sled-as-initial-metadata-backend.md)).
- **Copy-on-write tree mutation** — untouched subtrees share storage.
- **Versioning:** `commit`, `branch create/list/delete`, `checkout`,
  `log`, `diff` (between commits or trees).
- **`atlasctl` CLI** for everything above.
- **Python SDK** (`atlas-sdk`) — a thin subprocess wrapper that the Phase 2
  PyO3 binding will replace without breaking the public surface.
- **Safetensors header parser** (`atlas-fmt-safetensors`) — Phase 1
  building block for tensor-aware reads.

## Not yet wired

- FUSE / WinFsp / FileProvider / native miniport adapters
  (`atlas-fuse` is a stub today; T0.7 lights it up).
- Distributed replication, gossip, gRPC adapters (Phase 2+).
- Lineage, governance, and semantic planes (Phase 3+).

## Repo layout

```
crates/
  atlas-core/             shared types — Hash, errors, time, format version
  atlas-chunk/            content-addressed chunk store + LocalChunkStore
  atlas-object/           manifests + canonical serialization + self-hash
  atlas-meta/             MetaStore trait + sled backend
  atlas-fs/               filesystem engine — put/get/list/rename/delete
  atlas-version/          commits, branches, log, diff, checkout
  atlas-fuse/             FUSE adapter (stub for T0.7)
  atlas-fmt-safetensors/  safetensors header parser
  atlasctl/               CLI binary
clients/
  py/                     atlas-sdk — Python wrapper around atlasctl
docs/
  spec/v0.1.md            on-disk format spec
  adr/                    architecture decisions
```

## Build

```bash
cargo build --workspace --release
cargo test  --workspace
```

Phase 0 ships **zero native dependencies** — no CMake, no LLVM, no system
RocksDB. A stable Rust toolchain (>= 1.75) is enough on Linux, macOS, and
Windows.

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
```

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

| Phase | Theme | Highlights |
|-------|-------|-----------|
| 0     | Substrate | chunk store, manifests, FUSE plumbing, atlasctl |
| 1     | Versioning | commits, branches, diff, Python SDK, safetensors |
| 2     | Distribution | gRPC, replication, RocksDB adapter, PyO3 native binding |
| 3     | Semantics  | embeddings, content-aware diff, search |
| 4     | Lineage   | provenance graph, signatures, audit |
| 5+    | Governance, multi-tenant, federation |

See [ATLAS_implementation_plan.md](ATLAS_implementation_plan.md) for the
full task breakdown and dependency graph.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). The short version: keep the
public surface stable across phase boundaries, and don't merge changes
that the spec doesn't cover.

## License

Apache-2.0 — see [LICENSE](LICENSE).
