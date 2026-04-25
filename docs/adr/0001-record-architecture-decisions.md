# ADR-0001: Record architecture decisions

**Date:** 2026-04-25
**Status:** Accepted

## Context

ATLAS is a multi-year, multi-workstream project with many consequential design choices. We need a durable record of *why* each decision was made so future contributors can revisit trade-offs without archaeology.

## Decision

We use **Architecture Decision Records** (ADRs) stored in [`docs/adr/`](./), numbered sequentially (`NNNN-short-slug.md`). Every non-trivial design choice lands as an ADR before the code merges.

Each ADR has: Context · Decision · Consequences · Status. Status transitions: `Proposed → Accepted → (Deprecated | Superseded by N)`.

## Consequences

- PRs that introduce a new cross-cutting choice must link an ADR.
- ADRs are immutable once accepted — supersession is explicit, never edit-in-place.
- The index in [`docs/adr/README.md`](./README.md) is kept current.
