# ATLAS

## Agentic, Tiered, Lineage-Aware Storage

### A Next-Generation Filesystem Design Report

---

**Document type:** Design report / whitepaper draft
**Status:** Initial architecture proposal, iteration 1
**Scope:** Universal design — single-user laptop through multi-tenant datacenter
**Compatibility posture:** Hybrid — full POSIX compatibility plus extended APIs

---

## Table of contents

1. Executive summary
2. The problem: why current filesystems fail AI workloads
3. Design principles
4. System architecture at a glance
5. The seven layers in detail
6. User experience and desktop integration
7. MCP-native: the filesystem as an agent toolkit
8. Multi-protocol tool invocation (MCP, A2A, REST, gRPC, native SDK)
9. Data model and schemas
10. API surface — POSIX-extended, SDK, CLI, ioctls
11. Versioning, branching, and lineage
12. Semantic layer
13. Governance and security
14. Deployment modes
15. Engineering inventory — what to build
16. Phased implementation roadmap
17. Step-by-step build procedure
18. Trade-offs and open questions
19. Comparison to existing systems
20. Glossary
21. References

---

## 1. Executive summary

ATLAS is a POSIX-compatible, agent-native filesystem designed for the AI era. It unifies in a single product what today requires a stack of disconnected tools:

- **Parallel high-throughput I/O** comparable to DeepSeek's 3FS for training and inference workloads
- **Content-addressed storage** with automatic deduplication and verifiable integrity
- **Git-style versioning and branching** for datasets, models, and experiments
- **Semantic indexing** so files can be found by meaning, not just name
- **Lineage tracking** recording what data produced what artifact
- **Governance** enforcing licenses, permissions, and redaction at read time
- **Native MCP server** exposing every filesystem capability as an agent tool
- **Multi-protocol front-ends** — MCP, A2A, REST, gRPC, SDK — over one core
- **Desktop integration** as a mountable drive on Windows, macOS, and Linux, with Explorer/Finder/Nautilus extensions
- **Plain-user friendliness** — an ordinary user sees a normal folder that happens to be searchable by meaning

The compatibility posture is explicit: every existing tool (your text editor, `cp`, Python's `open`, a legacy C program) works unmodified. Aware tools opt into richer behavior through extended flags, extended attributes, a native SDK, and a built-in MCP endpoint.

The single biggest insight behind the design: every "AI data tool" in existence today — LakeFS, DVC, Pachyderm, Lance, MLflow, W&B artifacts, Pinecone, HuggingFace cache, WebDataset, safetensors, Sigstore, IPFS — is solving a piece of what a proper filesystem should solve natively. ATLAS is the unification.

---

## 2. The problem: why current filesystems fail AI workloads

Today's filesystems (NTFS, ext4, APFS, even ZFS and Btrfs) were designed for documents, databases, and developer tools. They make assumptions that no longer hold:

| Assumption | Breaks because |
|---|---|
| Files are opaque byte bags | AI pipelines need to address tensors, columns, chunks, embeddings inside files |
| You find files by path | Agents and humans increasingly search by meaning |
| Metadata is tiny and fixed (name, size, times, perms) | Modern workflows need license, lineage, embedding, schema, token count, sensitivity class |
| Read caching helps | Training deliberately randomizes reads; caching them can bias the model |
| Versioning is user-managed | Datasets, checkpoints, experiments need cheap branching and time-travel |
| POSIX permissions are enough | Agent-era access needs capabilities, license propagation, and read-time redaction |
| One consistency model fits all | Training checkpoints need strong consistency; random reads don't |
| Tiering is manual | Hot checkpoints, warm datasets, cold archives should flow automatically |
| The filesystem is a passive store | AI agents want to *ask* the filesystem things — "did this dataset ever touch PII?" |

Every pain point above has an industry workaround bolted on top: DVC for versioning, MLflow for lineage, Pinecone/Weaviate for semantic search, HuggingFace cache for model files, S3FS/GCSFuse for object-store access, WebDataset and Parquet for the small-files problem. They do not compose cleanly. ATLAS collapses them into one system.

---

## 3. Design principles

ATLAS is designed against these principles, in priority order:

1. **Throughput before intelligence.** If the substrate is slower than alternatives, nothing else matters. Borrow from 3FS: disaggregated architecture, RDMA, NVMe, cacheless random reads for training.
2. **POSIX-compatible always.** Every legacy tool continues to work. Mount ATLAS and `ls` it.
3. **Invisible until needed.** Normal users see a normal folder. Advanced users get extended APIs. Agents get MCP tools.
4. **One namespace, many protocol front-ends.** MCP, A2A, REST, gRPC, SDK, FUSE, S3, NFS, WebDAV all sit over the same object model.
5. **Content-addressed by default.** Every chunk is hashed; every manifest is a hash; every commit is signed. Integrity is free.
6. **Versioning is the default, not the feature.** Every write creates a new version. Branching and time-travel are built in.
7. **Metadata is first-class.** Rich, typed, extensible, queryable.
8. **Policy is enforced at open-time.** Governance is not a layer above; it's a check inside the kernel path.
9. **Pluggable intelligence.** Embedders, indexers, lineage recorders, and policy engines are services — swappable, versioned, auditable.
10. **Same code from laptop to datacenter.** Single-node mode is not a toy; it's the same system with smaller backings.

---

## 4. System architecture at a glance

ATLAS is organized into seven layers. The bottom three (substrate, chunk, object) are the performance substrate. The metadata plane is the spine. The three intelligence planes (semantic, lineage, governance) hang off metadata. Protocol adapters expose everything to the outside world, and user surfaces (desktop mount, CLI, GUI, MCP) sit on top of those adapters.

```
+-------------------------------------------------------------------------+
|  User surfaces                                                          |
|  Desktop mount | ATLAS Explorer (GUI) | atlasctl (CLI) | Web console    |
+-------------------------------------------------------------------------+
|  Protocol adapters                                                      |
|  FUSE | Native kext/miniport | MCP | A2A | REST | gRPC | S3 | NFS | WebDAV |
+-------------------------------------------------------------------------+
|  Intelligence planes                                                    |
|  Semantic plane   |   Lineage plane   |   Governance plane              |
+-------------------------------------------------------------------------+
|  Metadata plane                                                         |
|  Distributed KV: inodes, xattrs, versions, policies, refs               |
+-------------------------------------------------------------------------+
|  Object model                                                           |
|  Manifests, files, directories, versions, branches, commits, CoW        |
+-------------------------------------------------------------------------+
|  Chunk layer                                                            |
|  Content-addressed chunks, dedup, erasure coding, replication chains    |
+-------------------------------------------------------------------------+
|  Storage substrate                                                      |
|  RDMA fabric | NVMe pool | Storage services | CRAQ-replicated chains    |
+-------------------------------------------------------------------------+
```

---

## 5. The seven layers in detail

### 5.1 Storage substrate

Responsible for raw bytes in and out, with maximum parallelism and minimum tail latency.

**Key design choices:**
- Disaggregated: storage nodes are separate from compute. Any compute node sees the full pool.
- RDMA-first (InfiniBand, RoCE v2). Fallback to TCP for commodity deployments.
- NVMe SSDs as primary media. Optional tape/object-store tier for cold.
- Replication via CRAQ-style chains (Chain Replication with Apportioned Queries) for strong consistency without losing read parallelism.
- Stateless storage services: metadata lives separately, so storage nodes can be drained, replaced, and scaled independently.
- **Cacheless-by-default for training mode.** Read caching is an opt-in per-open flag. Cache pressure and GC are tuned for throughput, not for traditional filesystem workloads.
- **Aggressive caching for inference mode.** KV-cache-like hot-prefix sharing across nodes. This directly supports the KV-cache-offload pattern used by llm-d, vLLM, and similar serving stacks.

**Single-node version:** local NVMe, no RDMA, no replication. Same code path with different configuration.

### 5.2 Chunk layer

Every blob is split into fixed-size chunks (4 MiB default, configurable). Each chunk is identified by its cryptographic hash (BLAKE3) and stored once globally — identical chunks dedupe automatically. Chunks are erasure-coded (Reed-Solomon or similar) for durability, or simply replicated if latency matters more than space.

**Why content-addressing:**
- Free deduplication across datasets, checkpoints, branches, users
- Verifiable integrity: a corrupted chunk can't masquerade as a valid one
- Cheap versioning: a new version that changes 2% of a 1 TB file only stores 20 GB of new chunks
- Free sharing: any two users with the same hash have the same data, proven

### 5.3 Object model

Above chunks, everything is a hash-identified manifest:

- **Blob manifest:** ordered list of chunk hashes + optional format descriptor (`parquet`, `safetensors`, `jsonl`, etc.). Immutable.
- **File manifest:** references a blob manifest plus structured metadata (xattrs, embedding refs, lineage refs, policy ref, signature). Immutable.
- **Directory manifest:** ordered list of (name → object-hash) entries plus directory-level metadata. Immutable.
- **Ref:** mutable named pointer from a path to an object hash. All mutation lives here.
- **Commit:** `{tree_hash, parent_commit_hashes[], author, timestamp, message, signature}` — records a coherent point-in-time state of a subtree. Immutable and signed.
- **Branch:** a ref pointing to a commit, advanceable.

This model gives ATLAS for free: snapshots, branching, time-travel, reproducibility, signing, and dedup.

### 5.4 Metadata plane

A distributed transactional key-value store (FoundationDB is the reference choice; RocksDB for single-node) holds the actual state:

- Inode-like records keyed by object hash
- Extended attributes (both system and user namespaces)
- Version chains and commit graph
- Branch and ref pointers
- Policy records
- Embedding descriptors
- Lineage edges
- Open file sessions (for crash recovery)

Transactional semantics let versioning, lineage recording, and policy evaluation stay consistent with the bytes they describe.

### 5.5 Semantic plane

On write, objects flow through a pipeline:

1. **Format detection** — identify PDF, DOCX, JSONL, Parquet, safetensors, image, audio, etc.
2. **Content extraction** — text from PDFs, captions from images, schema from Parquet, tensor shapes from safetensors.
3. **Chunking** — natural semantic chunks (sections, pages, rows, shards), not byte-offset chunks.
4. **Embedding** — pluggable embedders produce vectors. Files can carry multiple embeddings (text, image, code) from multiple models.
5. **Indexing** — vectors go into a DiskANN/HNSW index; text goes into a BM25/BM25F index; structured metadata goes into a secondary KV index.

Queries are **hybrid**: vector + keyword + structured filters in one call. The plane supports query-time filters on access control, so queries don't surface results the caller can't read.

**Pluggability matters.** Embedder model, chunking strategy, and index backend are all configurable per-subtree. A laptop uses a small CPU embedder; a cluster uses a GPU embedding service; a specialized deployment might use a code-specific embedder for a `code/` subtree.

### 5.6 Lineage plane

Every process that accesses ATLAS is tagged with an execution context (process ID, job ID, experiment ID, agent ID). When the process reads and writes, the plane records edges:

```
edge: {
  source_hash: chunk_or_object_hash,
  target_hash: chunk_or_object_hash,
  process_context: {pid, job_id, experiment_id, agent_id},
  timestamp: iso8601,
  edge_type: read_input | wrote_output | derived_from | signed_by
}
```

Over time, the lineage graph answers:
- "What datasets trained this model?"
- "What artifacts depend on this deprecated dataset?"
- "Did a file tagged PII ever flow into this training set?"
- "Reproduce this experiment" — gives the exact manifest of inputs.

**Fidelity is tunable.** Full tracking is expensive; the plane supports sampling (e.g., log every write + 1% of reads) and context-level rollups.

### 5.7 Governance plane

Policies are objects too — versioned, signed, inheritable. A policy describes:
- **Capabilities required** to read, write, delete, branch, share
- **License** (SPDX-compatible structured)
- **Retention** rules (minimum retention, deletion-after)
- **Redaction** rules (PII detectors + replacement at read time)
- **Lineage constraints** ("files tagged X cannot flow into files tagged Y")
- **Watermarking** rules (stamp derived artifacts)

The engine evaluates at `open()` and on every relevant operation. Results are audit-logged. Policies inherit along directory trees and propagate along lineage edges — a model whose ancestors are non-commercial is automatically non-commercial.

---

## 6. User experience and desktop integration

This is the single hardest thing for an FS project to get right and the single thing that determines adoption. ATLAS has to appear as a **normal drive** on every OS, and everyday file operations (copy, rename, open in Word, attach to email) have to work unmodified.

### 6.1 Mount as a native drive

**Windows:**
- Ships an ATLAS filesystem driver (miniport or a WinFsp-based user-mode driver for v1, then a native miniport for v2).
- Mounted as a drive letter (default `A:`) or as a mount-point folder.
- Appears in File Explorer with a custom ATLAS volume icon and a rich tooltip showing capacity, branch, and sync status.

**macOS:**
- Ships as a FileProvider extension (the modern, supported path — same mechanism OneDrive and iCloud use) so it appears in Finder's sidebar and supports sparse/cloud semantics. A FUSE-based fallback exists for advanced users.
- Quick Look integration for AI file formats: preview tensors in safetensors, row-level preview in Parquet, embedding-space visualizer for embeddings.
- Spotlight integration surfaces semantic search results directly in system-wide search.

**Linux:**
- FUSE client for compatibility. Optional native kernel module for power users.
- GVfs (GNOME) and KIO (KDE) integration so Nautilus, Dolphin, and Files show ATLAS natively, with custom column providers (license, lineage, token count, last-embedded-at).

### 6.2 Shell / file-manager extensions

All three OSes get context-menu extensions and custom columns:

Right-click menu additions (platform-appropriate naming):
- **Find similar** — semantic search for files like this one
- **Show lineage** — open the lineage graph viewer rooted at this file
- **Branch here** — create a branch from this file or folder
- **View history** — version timeline
- **Open as MCP tool** — share this folder as an MCP server others can connect to
- **Check policy** — show who can access and under what terms
- **Copy as hash** — copy the content-addressable hash for sharing

Custom columns in list views:
- `Embedding status` (indexed / pending / skipped)
- `License`
- `Last lineage write`
- `Version count`
- `Token count` (for text files)

### 6.3 ATLAS Explorer (GUI)

A dedicated graphical app for the capabilities that don't fit in a file manager. Sections:

- **Browser** — tree + semantic search bar + column customization
- **Semantic search** — natural-language query across mounted volumes; results show ranked previews
- **Lineage viewer** — interactive DAG, pan/zoom/filter
- **Version control** — branch and commit UI similar to GitHub Desktop
- **Policy inspector** — who can do what, with diff against previous policy versions
- **Embedder health** — model info, embedding progress, queue depth
- **MCP tools panel** — which MCP tools are exposed, connection log, audit view
- **Connection manager** — mount/unmount remote ATLAS endpoints

### 6.4 CLI — atlasctl

A comprehensive CLI for scripting and power users. Subcommands:

```
atlasctl mount <endpoint> [--as A:]           # mount a remote ATLAS volume
atlasctl ls <path>                            # list with rich metadata
atlasctl find <query>                         # semantic search
atlasctl branch create <path> <name>          # branch a subtree
atlasctl branch list <path>
atlasctl commit -m <msg> [paths...]
atlasctl checkout <branch-or-commit>
atlasctl diff <a> <b>
atlasctl log <path>                           # version/commit history
atlasctl lineage <path> [--depth N] [--direction up|down]
atlasctl policy show <path>
atlasctl policy set <path> <policy.yaml>
atlasctl embed <path> [--model bge-m3]        # force re-embed
atlasctl verify <path>                        # check all signatures
atlasctl export <path> --format sqlite|tar|parquet
atlasctl mcp serve <path> [--port 7788]       # expose as MCP server
atlasctl a2a serve <path>                     # expose as A2A agent
atlasctl tier <path> --target hot|warm|cold
atlasctl quota show
atlasctl doctor                               # health check
```

### 6.5 Web console (for clusters)

A browser UI for admins and teams: capacity, quotas, embedder GPUs, policy editing, audit log, user and agent management.

---

## 7. MCP-native: the filesystem as an agent toolkit

This is one of ATLAS's differentiators. An MCP server is not a plugin — it's a first-class front-end that ships with the filesystem. Starting ATLAS starts the MCP endpoint.

### 7.1 Tools exposed

Every ATLAS capability is an MCP tool. Tools are organized into namespaces:

**Read and write**
- `atlas.fs.stat(path)` — metadata for a path
- `atlas.fs.list(path, recursive=false)` — directory listing with rich metadata
- `atlas.fs.read(path, offset=0, length=-1)` — raw bytes
- `atlas.fs.read_text(path)` — format-aware text extraction
- `atlas.fs.read_tensor(path, tensor_name)` — format-aware tensor slice
- `atlas.fs.read_schema(path)` — extract schema from Parquet/JSON/CSV
- `atlas.fs.write(path, content)` — create or overwrite
- `atlas.fs.append(path, content)`
- `atlas.fs.delete(path)` — moves to trash (restorable)

**Semantic**
- `atlas.semantic.query(q, filters?, limit?)` — natural-language search, returns ranked paths + snippets
- `atlas.semantic.similar(path, limit?)` — find files like this one
- `atlas.semantic.embed(path, model?)` — force embed
- `atlas.semantic.describe(path)` — LLM-generated summary (cached)

**Versioning**
- `atlas.version.log(path, limit?)` — commit history
- `atlas.version.diff(path_a, path_b | commit_a, commit_b)`
- `atlas.version.branch_create(path, name)`
- `atlas.version.branch_list(path)`
- `atlas.version.checkout(branch_or_commit)`
- `atlas.version.commit(paths, message)`
- `atlas.version.tag(commit, name)`

**Lineage**
- `atlas.lineage.upstream(path, depth?)` — what produced this
- `atlas.lineage.downstream(path, depth?)` — what depends on this
- `atlas.lineage.provenance(path)` — signed provenance statement
- `atlas.lineage.record(inputs[], output)` — explicit edge recording for agent workflows

**Governance**
- `atlas.policy.show(path)` — current effective policy
- `atlas.policy.check(path, operation)` — can I do this?
- `atlas.policy.audit(path, since?)` — audit log entries
- `atlas.policy.set(path, policy)` — requires elevated capability

**Agent and workflow**
- `atlas.agent.scratchpad_create(name)` — isolated working area with auto-cleanup
- `atlas.agent.checkpoint(state)` — snapshot agent memory into the FS
- `atlas.agent.fork(from_path, to_path)` — CoW fork for parallel exploration

### 7.2 How tools are served

The MCP server is built in. No separate process, no configuration wizard. On startup:
- A local Unix socket / Windows named pipe is exposed for same-machine agents.
- A TLS-secured TCP port is exposed (authenticated) for remote agents.
- An optional WebSocket endpoint is available for browser-based tools.

Tool discovery uses the standard MCP handshake. Each tool advertises its JSON schema, capability requirements, and a human-readable description.

### 7.3 Authentication and capability scoping

Agents identify themselves with capability tokens (signed, scoped, time-bounded). Example:

```json
{
  "agent_id": "research-assistant-7",
  "scope": {
    "paths": ["/projects/mamba-paper/", "/datasets/arxiv/"],
    "operations": ["read", "semantic_query", "lineage_upstream"],
    "denied": ["write", "branch_create"]
  },
  "expires_at": "2026-05-01T00:00:00Z",
  "signed_by": "user:alice"
}
```

Every MCP call is checked against the token. Every call is audit-logged with the token's agent ID.

### 7.4 Subtree-scoped MCP servers

`atlasctl mcp serve /projects/foo --port 7788` exposes just that subtree as an MCP server. This means a user can share a scoped slice of their data with an agent without exposing the whole FS.

---

## 8. Multi-protocol tool invocation

ATLAS treats protocol support as a first-class concern. The core filesystem exposes a single capability model; many adapter front-ends project that model into the protocol a given caller speaks. The current adapter matrix:

| Protocol | Purpose | Status in v1 |
|---|---|---|
| **POSIX / FUSE** | Every legacy tool, every scripting language | Required |
| **Native kernel drivers** | Performance-critical OS integration | v2 target |
| **MCP** | Agent tool invocation (Anthropic ecosystem and MCP-compatible clients) | Required |
| **A2A (Agent-to-Agent)** | Agent-to-agent cooperation (Google's A2A spec and similar) | Required |
| **REST / OpenAPI** | Web apps, scripts, general-purpose integration | Required |
| **gRPC** | High-performance service-to-service | Required |
| **GraphQL** | Rich metadata queries from UIs | Optional v1 |
| **S3 API** | Compatibility with the data-lake world; drop-in for anything that speaks S3 | Required |
| **NFS v4** | Legacy enterprise compatibility | Optional |
| **WebDAV** | Browser and mobile access, Office apps | Optional |
| **Anthropic tool-use JSON / OpenAI function-calling JSON** | Wire format adapters on top of MCP | Required |

### 8.1 The adapter pattern

Every adapter is a thin translator. It speaks the external protocol, authenticates the caller, maps the request to an internal capability invocation, applies the policy check, and translates the response back. Critically:

- **No protocol bypasses governance.** Every adapter funnels through the same policy engine.
- **No protocol gets its own capability model.** One capability model, N wire formats.
- **Protocol versioning is adapter-local.** When MCP v2 arrives, the core doesn't change — the adapter does.

### 8.2 A2A specifically

Agent-to-agent protocols (like Google's A2A) let agents discover and invoke capabilities on other agents. ATLAS ships with an A2A adapter that lets an ATLAS deployment announce itself as an A2A agent whose skills include search, fetch, branch, diff, attest. Other agents can then cooperate with an ATLAS instance as a peer.

### 8.3 Future-proofing

New agent protocols emerge regularly (MCP, A2A, OpenAI Realtime, Anthropic Computer Use, various proprietary formats). The adapter pattern means adding a new one is a one-shot translator plus capability mapping — the core doesn't shift.

---

## 9. Data model and schemas

Here are the concrete on-disk records.

### 9.1 Chunk

```
chunk: {
  hash: BLAKE3(data),
  size: u64,
  data: bytes,        // typically 4 MiB
  compressed: bool,
  storage_chain: [node_id, ...]
}
```

### 9.2 Blob manifest

```
blob_manifest: {
  hash: BLAKE3(serialized),
  chunks: [{hash, length}, ...],
  total_size: u64,
  format_hint: string | null,   // "parquet" | "safetensors" | "jsonl" | ...
  format_index: bytes | null    // format-specific index (e.g. parquet footer)
}
```

### 9.3 File manifest

```
file_manifest: {
  hash: BLAKE3(serialized),
  blob_hash: hash,
  created_at: timestamp,
  xattrs: {
    "user.*": map,
    "system.atlas.*": map
  },
  embeddings: [
    {model_id, model_version, chunks: [{range, vector_ref}]}
  ],
  schema_ref: hash | null,
  lineage_ref: edge_batch_hash | null,
  policy_ref: policy_hash | null,
  signatures: [{signer_id, algorithm, signature}]
}
```

### 9.4 Directory manifest

```
directory_manifest: {
  hash: BLAKE3(serialized),
  entries: [
    {name, object_hash, object_kind: file|dir|symlink|refspec}
  ],
  xattrs: map,
  policy_ref: policy_hash | null
}
```

### 9.5 Ref, commit, branch

```
ref: {
  path: string,
  current: object_hash,
  updated_at: timestamp
}

commit: {
  hash: BLAKE3(serialized),
  tree_hash: directory_manifest_hash,
  parent_hashes: [commit_hash, ...],
  author: {agent_id | user_id, name, email},
  timestamp: iso8601,
  message: string,
  signature: {...}
}

branch: {
  name: string,
  head: commit_hash,
  protection: {require_signed, require_reviewed, ...}
}
```

### 9.6 Lineage edge

```
lineage_edge: {
  source_hash: object_hash,
  target_hash: object_hash,
  process_context: {
    pid: int,
    agent_id: string | null,
    job_id: string | null,
    experiment_id: string | null,
    host: string
  },
  timestamp: iso8601,
  edge_type: "read_input" | "wrote_output" | "derived_from" | "signed_by" | "declared"
}
```

### 9.7 Policy

```
policy: {
  hash: BLAKE3(serialized),
  version: int,
  parent: policy_hash | null,
  capabilities: {
    read: [capability_spec],
    write: [capability_spec],
    branch: [capability_spec],
    delete: [capability_spec],
    share: [capability_spec]
  },
  license: {
    spdx_id: string | null,
    custom_terms: url | null,
    commercial_use: bool,
    attribution_required: bool,
    share_alike: bool
  },
  retention: {
    min_retention_days: int,
    delete_after_days: int | null
  },
  redaction: [
    {detector: string, action: "redact" | "deny" | "warn"}
  ],
  lineage_constraints: [
    {tag: string, forbidden_destinations: [tag]}
  ],
  signature: {...}
}
```

### 9.8 Extended attribute namespaces

```
user.*                            # user-defined, preserved by ATLAS
system.atlas.embedding.<model>    # vector reference
system.atlas.license              # SPDX or structured
system.atlas.lineage.upstream     # comma-separated hashes
system.atlas.schema               # schema reference hash
system.atlas.format               # format identifier
system.atlas.signature            # signature bundle
system.atlas.token_count          # for text: approximate token count
system.atlas.sensitivity          # "public" | "internal" | "restricted" | ...
system.atlas.policy               # policy hash
system.atlas.version_chain        # version linked list head
system.atlas.mcp_exposed          # bool: currently served as MCP tool
```

---

## 10. API surface — POSIX-extended, SDK, CLI, ioctls

### 10.1 Extended open() flags

New flags, compatible with the existing `open(2)` signature:

```c
#define O_AI_TRAIN_RANDOM   0x01000000  /* no read cache; streaming mode */
#define O_AI_INFER_PREFIX   0x02000000  /* aggressive prefix caching */
#define O_AI_CHECKPOINT     0x04000000  /* parallel striped write, strong consistency */
#define O_AI_AGENT          0x08000000  /* policy-enforced, may redact */
#define O_AI_STREAM         0x10000000  /* prefetch from cold storage aggressively */
#define O_AI_VERSION        0x20000000  /* open at a specific manifest hash (pass via fcntl) */
```

### 10.2 Extended attributes

Get and set via standard `getxattr` / `setxattr`. Applications unaware of the extensions simply don't read them; aware applications get rich data.

### 10.3 New ioctls

```c
ATLAS_IOC_QUERY_SEMANTIC    /* struct {query, filters, limit} -> paths[] */
ATLAS_IOC_BRANCH_CREATE     /* struct {path, name, base_commit?} */
ATLAS_IOC_COMMIT            /* struct {paths, message} -> commit_hash */
ATLAS_IOC_CHECKOUT          /* struct {branch_or_commit} */
ATLAS_IOC_LINEAGE_QUERY     /* struct {path, direction, depth} -> graph */
ATLAS_IOC_POLICY_CHECK      /* struct {path, operation} -> allowed + reason */
ATLAS_IOC_ATTEST            /* struct {path} -> signed provenance */
ATLAS_IOC_TIER              /* struct {path, target_tier} */
```

### 10.4 Native Python SDK

```python
import atlas

# Basic IO with hints
with atlas.open("models/llama-70b.safetensors",
                hint=atlas.Hint.INFERENCE_STREAM) as f:
    q_proj = f.tensor("layer.4.attn.q_proj")

# Semantic query
for hit in atlas.query("papers on state space models since 2024",
                       limit=10,
                       filters={"license.commercial_use": True}):
    print(hit.path, hit.score, hit.snippet)

# Branching like git
with atlas.branch("datasets/corpus", "filter-short-docs") as b:
    for doc in b.walk("*.jsonl"):
        if doc.metadata["token_count"] < 100:
            doc.delete()
    b.commit("filtered out docs under 100 tokens")

# Lineage
graph = atlas.lineage("models/llama-ft.safetensors", direction="up", depth=3)
for node in graph.nodes:
    print(node.path, node.hash, node.policy.license.spdx_id)

# Write with explicit lineage declaration
with atlas.write("models/llama-ft-v2.safetensors",
                 inputs=["datasets/corpus@filter-short-docs",
                         "models/llama-70b.safetensors",
                         "configs/sft.yaml"]) as f:
    torch.save(model, f)

# MCP
with atlas.mcp_serve("/projects/mamba-paper", port=7788) as server:
    print("MCP tools available at", server.endpoint)
```

### 10.5 Rust and C SDKs

Analogous surface with language-idiomatic types. Rust SDK uses ownership for policy capability tokens so they can't leak. C SDK for legacy integration.

---

## 11. Versioning, branching, and lineage

### 11.1 Write semantics

A write creates a new blob manifest, then a new file manifest, then a new parent-directory manifest, then advances a ref. All structures are content-addressed, so this is fast (parent directories only change at the entries that point to the new file).

### 11.2 Branches

A branch is a named ref pointing at a commit. Branch creation is O(1). Branch deletion marks the branch dead but preserves commits for a retention window.

### 11.3 Commits

A commit is a signed manifest tying a tree to its parents. Commits are hashed and immutable. Branches can require signed commits as a policy.

### 11.4 Time-travel

Mount `atlas://volume/@commit_abc123/path` to see the state at that commit. Read-only by default; branch to modify.

### 11.5 Lineage recording modes

- **Implicit:** process-level tracking, automatic
- **Explicit:** agent calls `atlas.lineage.record(inputs, output)` to declare
- **Declarative:** a `lineage.yaml` in a directory documents expected inputs, and violations are flagged

### 11.6 Reproducibility

Every execution context gets a **manifest fingerprint** — the set of object hashes it read, plus the commits of the volumes it was mounted against. Reproducing a run is: pin the fingerprint, rerun.

---

## 12. Semantic layer

### 12.1 Pluggable embedders

Embedders are services registered with the FS. A registration specifies:
- Model ID and version
- Modalities supported (text, image, code, audio)
- Chunking strategy
- Cost tier (fast/cheap vs slow/accurate)
- Optional: subtree where this embedder applies

A single file can have multiple embeddings from multiple models, stored in xattrs.

### 12.2 Indexes

- **Vector index:** DiskANN for scale, HNSW for smaller deployments.
- **Text index:** Tantivy / Lucene-class BM25F for keyword and phrase.
- **Structured index:** on xattrs, for filtering (license, sensitivity, token count).
- **Graph index:** on lineage edges for provenance queries.

Queries compose these: `query("transformers") AND license.commercial=true AND lineage.upstream contains dataset/arxiv`.

### 12.3 Freshness

Embedding happens asynchronously by default (write returns quickly; indexing catches up in the background). A write can request synchronous embedding via a flag, at a latency cost.

### 12.4 Re-embedding

When an embedder is upgraded, a background job re-embeds. Old and new embeddings coexist, tagged by model version, so queries can be pinned to a specific model for reproducibility.

---

## 13. Governance and security

### 13.1 Capability tokens

Instead of user IDs mapped to permissions, ATLAS uses capability tokens: scoped, signed, time-bounded grants. This fits the agent era — you can hand an agent a token scoped to "read `/projects/foo/`, expires in 4 hours" without giving it your user account.

### 13.2 Policy inheritance

Policies attach to directories and propagate down. File-level overrides are allowed with justification (logged).

### 13.3 Read-time redaction

Configurable PII detectors (names, emails, SSNs, API keys) can redact content on the fly for agents with limited capabilities. The un-redacted content is never exposed to these callers.

### 13.4 Lineage-based enforcement

"Files tagged non-commercial cannot produce files tagged commercial." The governance plane consults the lineage graph on write and can refuse the operation or require an acknowledgment.

### 13.5 Audit log

Every privileged operation (policy change, capability issuance, cross-tenant access) is appended to a tamper-evident audit log (Merkle-tree structured, exportable).

### 13.6 Cryptographic signing

Every commit and policy is signed. Keys are managed either locally (single-user) or via a KMS (team/cluster). Signatures make "is this the real Llama-4 weights file?" answerable by verifying against a known public key.

### 13.7 Local admin override

To avoid becoming DRM, ATLAS always lets the storage owner run in an "enforcement disabled" mode with clear audit marking. Enterprises can forbid this mode; individuals can use it.

---

## 14. Deployment modes

The same code, three physical backings:

### 14.1 ATLAS Solo (single workstation)

- Local NVMe as chunk store
- Embedded KV (RocksDB) for metadata
- Local CPU embedder (small model)
- Single-user, no authentication
- Mounted as a drive on the OS
- MCP server on localhost Unix socket

**Hardware minimum:** modern laptop, 16 GB RAM, SSD.

### 14.2 ATLAS Team (small cluster, 3–20 nodes)

- Pooled NVMe across nodes
- FoundationDB cluster for metadata
- Shared embedding service on a handful of GPUs
- Team authentication (OIDC, capability tokens)
- Desktop mounts on every team member's machine
- MCP server on an authenticated network port

**Hardware minimum:** 3 storage nodes with NVMe, 10 GbE minimum, optional GPU for embeddings.

### 14.3 ATLAS Scale (datacenter / cloud)

- Hundreds of storage nodes, RDMA fabric
- Distributed metadata with partitioning
- Dedicated GPU embedding cluster
- Multi-tenant with strict quotas and isolation
- Cross-region replication (eventual)
- Full audit and compliance mode

**Hardware minimum:** RDMA fabric (100 GbE or IB), multiple storage tiers, GPU pool.

### 14.4 Cloud-native mode

ATLAS can run on top of a cloud object store (S3, GCS) using the object store as its chunk backing. This is slower but removes the hardware requirement — useful for cost-sensitive or burstable workloads.

---

## 15. Engineering inventory — what to build

Here is the concrete component list. Each is a separate codebase (or a module of a larger codebase) with its own tests, docs, and release cadence.

### 15.1 Core services

| Component | Language | Purpose |
|---|---|---|
| `atlas-storage` | Rust | Chunk storage daemon, replication chains |
| `atlas-meta` | Rust | Metadata service (or FoundationDB adapter) |
| `atlas-indexer` | Rust | Vector + text + structured index service |
| `atlas-embedder` | Python | Embedding service (multi-model, GPU-aware) |
| `atlas-lineage` | Rust | Lineage edge journal and graph service |
| `atlas-governor` | Rust | Policy engine and audit log |
| `atlas-tiering` | Rust | Hot/warm/cold mover |
| `atlas-gc` | Rust | Garbage collection (unreferenced chunks, old versions) |

### 15.2 Protocol adapters

| Component | Language | Purpose |
|---|---|---|
| `atlas-fuse` | Rust + C | FUSE client |
| `atlas-wfsp` | C++ | Windows WinFsp-based driver (v1) |
| `atlas-miniport` | C | Windows native miniport (v2) |
| `atlas-fileprovider-mac` | Swift | macOS FileProvider extension |
| `atlas-gvfs` | C | GNOME Files / GVfs backend |
| `atlas-kio` | C++ | KDE KIO slave |
| `atlas-mcp` | Rust | MCP server |
| `atlas-a2a` | Rust | A2A agent adapter |
| `atlas-rest` | Rust | REST / OpenAPI server |
| `atlas-grpc` | Rust | gRPC server |
| `atlas-graphql` | Rust | GraphQL server |
| `atlas-s3` | Rust | S3-compatible gateway |
| `atlas-nfs` | Rust | NFS v4 server |
| `atlas-webdav` | Rust | WebDAV server |

### 15.3 SDKs

| Component | Language | Purpose |
|---|---|---|
| `atlas-sdk-py` | Python | Python SDK, PyTorch/HF integration |
| `atlas-sdk-rs` | Rust | Rust SDK |
| `atlas-sdk-c` | C | C/C++ SDK |
| `atlas-sdk-js` | TypeScript | Browser/Node SDK |
| `atlas-sdk-go` | Go | Go SDK |

### 15.4 User surfaces

| Component | Language | Purpose |
|---|---|---|
| `atlasctl` | Rust | CLI |
| `atlas-explorer` | Rust + Tauri or TypeScript + Electron | Cross-platform GUI |
| `atlas-shellext-win` | C++ | Windows Explorer context menu / columns |
| `atlas-finder-ext` | Swift | macOS Finder Sync + Quick Look |
| `atlas-nautilus-ext` | Python | GNOME Nautilus extension |
| `atlas-dolphin-ext` | C++ | KDE Dolphin extension |
| `atlas-web` | TypeScript | Web admin console |

### 15.5 Content and format plugins

| Component | Purpose |
|---|---|
| `atlas-fmt-parquet` | Format-aware reads (row/column slices) |
| `atlas-fmt-safetensors` | Tensor-level reads |
| `atlas-fmt-pdf` | Text extraction, per-page chunking |
| `atlas-fmt-docx` | Text extraction, structure preservation |
| `atlas-fmt-arrow` | Arrow/Feather support |
| `atlas-fmt-jsonl` | Line-level access |
| `atlas-fmt-zarr` | Array-level access |
| `atlas-fmt-image` | Captioning and perceptual hashing |
| `atlas-fmt-audio` | Transcription and embedding |

### 15.6 Embedders

| Component | Purpose |
|---|---|
| `atlas-embed-text-small` | Local CPU embedder (bge-small-class) |
| `atlas-embed-text-large` | GPU embedder (bge-m3-class) |
| `atlas-embed-code` | Code-specific embedder |
| `atlas-embed-image` | CLIP-class vision embedder |
| `atlas-embed-audio` | Audio embedder |

### 15.7 Infrastructure

| Component | Purpose |
|---|---|
| `atlas-deploy` | Installer for single-node and cluster |
| `atlas-ops` | Operational tooling, health checks, backup |
| `atlas-migrate` | Import from S3, GCS, ext4 trees, git repos |
| `atlas-bench` | Benchmarking suite |
| `atlas-chaos` | Fault-injection tester |

**Total: around 40–55 components.** Not all need to exist in v1. See the roadmap.

---

## 16. Phased implementation roadmap

Realistic estimate for a small dedicated team (8–15 engineers): 24–36 months to a usable v1; 48+ months to a production-grade v2. The phasing:

### Phase 0 — Foundation (months 0–6)
**Goal:** basic content-addressed storage with FUSE mount.

- `atlas-storage` — chunk daemon on a single node
- `atlas-meta` — single-node metadata (RocksDB)
- `atlas-fuse` — minimal FUSE client
- `atlasctl` — minimum viable CLI (mount, ls, read, write)
- Basic POSIX compliance; passes a subset of pjdfstest

**Deliverable:** mount a local ATLAS volume on Linux, read and write files, content-addressed under the hood.

### Phase 1 — Versioning (months 6–10)
**Goal:** branches, commits, time-travel.

- Extend object model with commits and branches
- Add commit graph to metadata plane
- Extend CLI with `branch`, `commit`, `checkout`, `log`, `diff`
- Python SDK v0.1 with these primitives

**Deliverable:** git-like operations on arbitrary subtrees; reproducible mounts at past commits.

### Phase 2 — Distributed (months 10–16)
**Goal:** scale-out storage and metadata.

- Multi-node `atlas-storage` with CRAQ replication chains
- FoundationDB-backed metadata
- RDMA support
- Benchmarking against NFS, 3FS, Lustre

**Deliverable:** 3FS-class throughput on a small cluster.

### Phase 3 — Semantic (months 14–20, overlaps Phase 2)
**Goal:** meaning-based search.

- `atlas-embedder` with pluggable models
- `atlas-indexer` with vector + text + structured indexes
- `atlas.semantic.*` SDK and CLI
- Format plugins for PDF, DOCX, JSONL, Parquet

**Deliverable:** natural-language search across a mounted ATLAS volume.

### Phase 4 — Lineage and governance (months 18–24)
**Goal:** provenance and policy.

- `atlas-lineage` service and SDK
- `atlas-governor` with policy engine and audit log
- Capability token issuance and verification
- Read-time redaction

**Deliverable:** "who made this, what touched it, can this agent read it" answered.

### Phase 5 — MCP and protocols (months 22–28)
**Goal:** agent-native and multi-protocol.

- `atlas-mcp` server with full tool catalog
- `atlas-a2a` adapter
- `atlas-rest` and `atlas-grpc`
- `atlas-s3` gateway

**Deliverable:** ATLAS is usable as an MCP tool and A2A agent; plays nicely with existing data-lake tooling.

### Phase 6 — Desktop integration (months 26–34)
**Goal:** ordinary-user friendliness.

- Windows WinFsp driver + shell extensions
- macOS FileProvider extension + Finder Sync + Quick Look
- Linux GVfs and KIO backends
- `atlas-explorer` GUI v1
- Onboarding flows: install, mount, first use

**Deliverable:** an ATLAS drive appears on the desktop like any other; non-developer users can use it.

### Phase 7 — Production hardening (months 32–48+)
- Chaos testing, fault injection
- Backup and cross-region replication
- Enterprise auth (SAML, OIDC)
- SOC 2 / compliance readiness
- Disaster recovery runbooks
- Performance tuning across workloads

**Deliverable:** production-grade v2 suitable for datacenter and regulated use.

---

## 17. Step-by-step build procedure

Granular procedure for the first year, enough to start executing:

### Step 1 — Specification freeze (weeks 1–4)
Write the concrete binary spec for chunks, blob manifests, file manifests, directory manifests, commits. Versioned so future changes are non-breaking.

### Step 2 — Reference implementation of the chunk layer (weeks 4–10)
In Rust, a single-node chunk daemon with content-addressed write, read, delete, verify. Back-end: local NVMe, one file per chunk or a packed format for small chunks. Protocol: gRPC over Unix socket.

### Step 3 — Metadata store (weeks 8–14, overlaps)
RocksDB-backed KV with a schema for inodes, xattrs, refs. Simple transactional API. Single-process first.

### Step 4 — Object model in code (weeks 12–18)
Compose chunks + metadata into files and directories. Implement writes through manifest creation. Integrity verification at every step.

### Step 5 — Minimum FUSE client (weeks 16–22)
FUSE operations: `getattr`, `readdir`, `open`, `read`, `write`, `create`, `unlink`, `rename`, `setxattr`, `getxattr`. Enough to pass a meaningful subset of pjdfstest.

### Step 6 — CLI (weeks 20–24)
`atlasctl` with `mount`, `ls`, `stat`, `cat`, `cp`, `mv`, `rm`.

### Step 7 — Versioning primitives (weeks 24–32)
Add commits, branches, checkout, log, diff. Write tests that branch a 10 GB tree, modify 1%, verify dedup works.

### Step 8 — Python SDK MVP (weeks 28–36)
`atlas.open`, `atlas.branch`, `atlas.commit`, plus the PyTorch-friendly tensor slicing for safetensors via `atlas-fmt-safetensors`.

### Step 9 — First external users (weeks 34–40)
Invite a small cohort (internal team, maybe a research lab) to use ATLAS Solo on their laptops. Collect feedback.

### Step 10 — Semantic plane alpha (weeks 40–52)
Integrate a small embedder, a DiskANN index, and a BM25 text index. `atlas.semantic.query` works on text files. Proof point: "find the paper where I wrote about SSMs."

### Milestones beyond year 1
Follow the phase plan from section 16.

### Engineering practices (applies throughout)
- Fuzz testing from day one (the spec is a binary format)
- Property-based tests on the object model
- Chaos testing from phase 2 onward
- Public benchmarks against NFS/ext4/3FS/ZFS
- Semantic versioning on the on-disk format
- Migration tools at every format version change
- Audit-logged everything from day one

---

## 18. Trade-offs and open questions

### 18.1 Embedding cost at scale
Auto-embedding everything costs serious GPU compute. Options:
- Embed lazily on first query rather than on write
- Tiered embedders (cheap for all, expensive for marked-important subtrees)
- User-opt-in for subtrees where cost matters

### 18.2 FUSE vs native kernel drivers
FUSE is portable and fast enough for most workloads but has a performance ceiling. Native drivers perform better but are complex to write and maintain per OS. The plan: ship FUSE everywhere in v1, add native Windows miniport and Linux kmod in v2 for specific high-performance deployments.

### 18.3 Protocol proliferation
Every new agent protocol adds an adapter. This is manageable if adapters stay thin, but there's a real risk of adapter-sprawl. Mitigation: publish a capability spec and make adapters mechanically generated where possible.

### 18.4 Governance vs autonomy
Strict enforcement feels like DRM. The design decision: policies are **enforceable but always auditable**, and local admin can override with a clear marker in the audit log. Enterprises forbid override; individuals keep control.

### 18.5 Semantic drift
When the default embedder changes, old queries return subtly different results. The plan: embeddings are model-tagged, queries can pin a model version, re-embedding is a managed rolling operation.

### 18.6 Compatibility with existing data lakes
ATLAS needs to coexist with S3, GCS, HDFS, and existing file stores for years. The S3 gateway plus an ingest tool (`atlas-migrate`) make this incremental rather than all-or-nothing.

### 18.7 The bootstrap problem
A filesystem is only as valuable as the tools that use it. Strategy: aggressive POSIX compatibility plus SDKs for the ML ecosystem (PyTorch, HuggingFace, JAX, LangChain, LlamaIndex) delivered in phase 3.

### 18.8 Metadata-plane scalability
Billions of tiny files at the metadata plane is a known hard problem (3FS uses FoundationDB precisely because of this). At extreme scale, ATLAS may want a "packed object" primitive that bundles many small files into one chunk from the metadata's perspective.

---

## 19. Comparison to existing systems

| System | Throughput | Semantic | Versioning | Lineage | Governance | MCP-native | Desktop |
|---|---|---|---|---|---|---|---|
| ext4 / NTFS / APFS | Baseline | No | No | No | Basic POSIX | No | Native |
| ZFS / Btrfs | Good | No | Snapshots only | No | POSIX + ACLs | No | Native |
| 3FS | Excellent | No | No | No | Minimal | No | FUSE |
| LakeFS | Baseline | No | Yes (git-like) | Partial | Minimal | No | No |
| DVC | N/A | No | Yes | Partial | No | No | No |
| Pachyderm | Baseline | No | Yes | Yes | Minimal | No | No |
| IPFS | Variable | No | Content-addressed | No | No | No | Partial |
| Lance | N/A (format) | Partial | Yes | No | No | No | No |
| LSFS | N/A (overlay) | Yes | No | No | No | No | No |
| **ATLAS** | **Excellent** | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | **Native** |

ATLAS is the first unification. That's the wager.

---

## 20. Glossary

- **Blob manifest:** immutable record of the chunk list that makes up a file's bytes
- **Branch:** named mutable pointer to a commit
- **Capability token:** signed, scoped, time-bounded grant of permission
- **Chunk:** fixed-size byte range, content-addressed by its hash
- **Commit:** signed record of a tree state and its parent commits
- **CoW:** copy-on-write — modifications create new versions without duplicating unchanged data
- **CRAQ:** Chain Replication with Apportioned Queries — consistent replication with parallel reads
- **DiskANN / HNSW:** disk-resident / in-memory approximate nearest neighbor indexes
- **File manifest:** immutable record of a file's bytes plus its metadata
- **Lineage:** graph of causal relationships between objects
- **Manifest fingerprint:** set of object hashes touched by an execution context
- **MCP:** Model Context Protocol — an agent tool-invocation protocol
- **A2A:** Agent-to-Agent protocol — lets agents discover and invoke capabilities on other agents
- **Policy:** signed, inheritable record governing how an object can be used
- **Ref:** mutable name pointing at an immutable object hash
- **Substrate:** the bottom storage layer — raw bytes over NVMe and RDMA
- **xattr:** extended attribute — key-value metadata attached to a file

---

## 21. References

- DeepSeek, *Fire-Flyer File System (3FS): A High-Performance Distributed File System*, open-sourced 2025 — the reference for the performance substrate
- llm-d project, *Hierarchical KV offloading across CPU and filesystem tiers*, 2025 — inference-cache pattern
- Shi et al., *From Commands to Prompts: LLM-based Semantic File System for AIOS*, ICLR 2025 — the semantic-file-system paradigm
- FoundationDB, Apple — transactional distributed KV
- Chain Replication (van Renesse & Schneider, 2004) and CRAQ (Terrace & Freedman, 2009)
- IPFS — content-addressable storage at internet scale
- LakeFS, DVC, Pachyderm — data versioning prior art
- Lance (Eto Labs) — columnar format for ML
- Meta, *Tectonic Filesystem* — exabyte-scale distributed FS
- Anthropic, *Model Context Protocol* specification
- Google, *Agent-to-Agent (A2A) protocol* specification
- SPDX, *Software Package Data Exchange* — license identifiers
- Sigstore — code/artifact signing infrastructure

---

**End of report.**

This is iteration 1. Concrete next steps are in section 16 (roadmap) and section 17 (step-by-step procedure). The document is designed to be forkable and iterable — every section is independent enough to rewrite without disturbing others.
