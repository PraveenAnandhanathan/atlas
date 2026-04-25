# ATLAS — project charter

**Purpose.** Build a POSIX-compatible, agent-native filesystem that unifies in a single product what today requires ten disconnected tools: content-addressed storage, versioning, semantic search, lineage, governance, MCP/A2A, and desktop integration. See [`ATLAS_design_report.md`](../ATLAS_design_report.md) for the full design and [`ATLAS_implementation_plan.md`](../ATLAS_implementation_plan.md) for the execution plan.

**In scope.**
- Single-node Solo through datacenter Scale deployments (same code, different configs).
- Full POSIX compatibility as the non-negotiable baseline.
- Extended SDK, CLI, ioctls, MCP, A2A, REST, gRPC, S3 frontends.
- Desktop integration on Linux, macOS, Windows.

**Out of scope for v1.**
- Native Linux kmod; native Windows miniport (v2).
- Multi-region replication.
- SOC 2 certification (readiness only).
- A2A beyond a reference implementation.
- GPU embedder autoscaling.

**Success at v1 GA (month ~33).** Measurable criteria in plan §1.3 and gate checklist in plan §17.

**Non-negotiables.**
1. Every legacy tool (`cat`, `cp`, Python `open`) works unmodified.
2. On-disk format versioned from day 1; migrations shipped before breaking changes.
3. Every privileged op goes through the audit log.
4. No protocol adapter bypasses the capability model or the governance plane.

**Governance of this repo.**
- Trunk-based development on `main`, short-lived branches, squash-merges.
- Required review from a CODEOWNER of the touched crate.
- Signed commits on `main` (once key infra is set up — see plan §18).
- ADR required for cross-cutting decisions.

**Versioning.**
- On-disk format: `vMAJOR.MINOR`.
- Crates and SDKs: SemVer; public APIs stabilize at v1.0.
- Release cadence: monthly pre-1.0; quarterly post-1.0.
