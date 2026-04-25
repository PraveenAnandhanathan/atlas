# Contributing to ATLAS

Welcome. This file describes how to contribute code, docs, and ADRs.

## 1. Before you start

1. Read [`ATLAS_design_report.md`](./ATLAS_design_report.md) and [`ATLAS_implementation_plan.md`](./ATLAS_implementation_plan.md).
2. Skim the [ADR index](./docs/adr/README.md) for past decisions.
3. For anything non-trivial, open an issue or ADR proposal first.

## 2. Dev setup

```bash
# Rust toolchain
rustup install stable
rustup component add rustfmt clippy

# Clone
git clone https://github.com/PraveenAnandhanathan/atlas.git
cd atlas

# Build and test
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Phase 0 has **zero native deps** — no CMake, no LLVM, no FoundationDB. (sled is pure-Rust. See [ADR-0003](./docs/adr/0003-sled-as-initial-metadata-backend.md).)

## 3. Branching and commits

- Branch from `main`, short-lived.
- **Conventional commits**, enforced by CI:
  - `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `perf:`, `spec:`, `adr:`
- Link the issue in the PR body.
- Squash-merge into `main`.

## 4. Code review rules

- **Two-person review** for anything touching on-disk format, signing, or governance.
- Single-reviewer is fine for docs, tests, and local refactors.
- CODEOWNERS enforce per-crate review.

## 5. ADR workflow

1. Copy `docs/adr/README.md` template into `docs/adr/NNNN-short-slug.md`.
2. Status starts `Proposed`. Open a PR, tag `adr:` in the commit.
3. On merge, the ADR is `Accepted`. Add it to the index.
4. Never edit an accepted ADR — supersede it with a new one.

## 6. On-disk format changes

- A format change needs a `spec:` commit + an ADR.
- Minor format bumps must be forward-compatible (new `Option<...>` fields).
- Major bumps require a migration under `crates/atlas-migrate` before merge.
- Test vectors under `docs/spec/vectors/` must be extended.

## 7. Testing

- **Unit tests** colocated with source (`#[cfg(test)]`).
- **Property tests** (`proptest`) for anything with invariants: hashes, manifests, CoW.
- **Integration tests** under each crate's `tests/` directory.
- **Fuzz targets** under `fuzz/` once a surface is stable.
- New code should not lower coverage; add tests for new branches.

## 8. Docs

- Every public item needs a doc comment.
- Cross-link relevant ADRs.
- User-facing changes update `docs/guide/` when that directory lands in Phase 1.
