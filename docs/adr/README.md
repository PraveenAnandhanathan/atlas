# Architecture Decision Records

Index of accepted ADRs. Add new rows as ADRs land.

| # | Title | Status |
|---|---|---|
| [0001](./0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](./0002-blake3-and-4mib-chunks.md) | BLAKE3-256 as the content hash; 4 MiB default chunk size | Accepted |
| [0003](./0003-sled-as-initial-metadata-backend.md) | sled as the initial metadata backend; RocksDB and FoundationDB behind a trait | Accepted |

## ADR template

```markdown
# ADR-NNNN: Short imperative title

**Date:** YYYY-MM-DD
**Status:** Proposed | Accepted | Deprecated | Superseded by NNNN

## Context
Why this decision is being made. What forces are at play.

## Decision
The decision itself, stated positively.

## Consequences
What becomes easier / harder / different as a result.
```
