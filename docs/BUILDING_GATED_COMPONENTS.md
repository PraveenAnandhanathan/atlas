# Building the Toolchain-Gated Components

The default `cargo build --workspace` produces a complete, production-ready
ATLAS server, SDK, and CLI on Linux. A handful of components are gated behind
Cargo features or `#[cfg(target_os = …)]` because they require external
libraries, hardware, or a platform toolchain that is not present in a generic
Linux CI image. This document explains how to build and verify each one.

| Component | Gate | Requires | Verifiable in generic Linux CI? |
|-----------|------|----------|---------------------------------|
| FoundationDB metadata backend | `--features fdb` (crate `atlas-meta-fdb`) | `libfdb_c` 7.3 client + a running FDB cluster | **Validated ✅** — live test in CI (`fdb-integration` job) |
| RDMA transport | `--features rdma` (crate `atlas-transport`) | `libibverbs` + InfiniBand/RoCE NIC | Handshake **validated ✅** (CI) / verbs data path needs hardware |
| Windows Explorer shell extension | `#[cfg(windows)]` (`atlas-shellext-win`) | Windows SDK + MSVC (`comshim.cpp`) | ❌ needs Windows |
| Windows WinFsp driver | `#[cfg(windows)]` (`atlas-wfsp`) | WinFsp runtime + MSVC | ❌ needs Windows |
| macOS FileProvider extension | `#[cfg(target_os="macos")]` (`atlas-fileprovider-mac`) | Xcode + Swift (`swift/` package) | ❌ needs macOS |
| GNOME/KDE virtual filesystem | `atlas-gvfs` C-ABI exports | GLib/GIO + KIO dev headers | Core + C-ABI **validated ✅** / daemon mount needs GNOME |

## FoundationDB backend (`fdb` feature)

```bash
# Install the matching client (API version 7.3 — see workspace Cargo.toml).
curl -sSL -o fdb-clients.deb \
  https://github.com/apple/foundationdb/releases/download/7.3.27/foundationdb-clients_7.3.27-1_amd64.deb
sudo dpkg -i fdb-clients.deb

# Install the server too (auto-starts a local single/memory cluster):
curl -sSL -o fdb-server.deb \
  https://github.com/apple/foundationdb/releases/download/7.3.27/foundationdb-server_7.3.27-1_amd64.deb
sudo dpkg -i fdb-server.deb
fdbcli --exec "status minimal"          # confirm "database is available"

# Run the live integration test (round-trips put/get/delete, scan_prefix,
# and atomic transactions against the real cluster):
cargo test -p atlas-meta-fdb --features fdb
```

The live test boots the FDB network once (`unsafe { foundationdb::boot() }`),
opens an `FdbMetaStore` in a per-PID namespace, and asserts the full `MetaStore`
contract. This is wired into CI as the `fdb-integration` job.

**Note on key layout:** `FdbMetaStore` stores keys as `<namespace>\x1f<key>`
with the logical key appended as **raw bytes** — it does *not* tuple-encode the
key. Tuple encoding null-terminates strings, which silently truncates prefix
scans (an earlier version had this bug; `scan_prefix` returned nothing). The
raw-prefix layout makes ASCII prefix scans (`ref:`, `commit:`) correct.

The client API version is pinned in the workspace `Cargo.toml`
(`foundationdb = { version = "0.9", features = ["fdb-7_3"] }`). Installing a
client whose header reports an older API version fails the `foundationdb-sys`
build with *"Requested API version requires a newer version of this header"* —
match the client to the feature.

## RDMA transport (`rdma` feature)

```bash
sudo apt-get install -y libibverbs-dev librdmacm-dev
cargo test -p atlas-transport --features rdma      # compiles + handshake tests
```

**What is validated (no hardware needed):** the `QpParams` control-channel
handshake — the big-endian wire format and the write-before-read exchange
ordering — is unit-tested over a real TCP socket pair (`rdma::qp_params_tests`,
run by the `rdma-transport` CI job). This is the pure-data half of RDMA
connection setup and a common source of endianness/deadlock bugs.

**What still needs hardware:** the RDMA module compiles and allocates protection
domains, completion queues, queue pairs, and memory regions, but the **data
path** (QP state-machine transitions INIT → RTR → RTS via `ibv_modify_qp`, plus
the CQ-polling task that bridges `ibv_post_send`/`ibv_post_recv` to the async
channels) is intentionally **not wired**: `RdmaTransport::connect` /
`RdmaAcceptor::accept` return an explicit error directing callers to
`TcpTransport`. Silently allocating hardware resources and then falling back to
TCP would mislead callers into believing they have RDMA connectivity.

Completing and validating the data path requires an InfiniBand/RoCE NIC, or a
Soft-RoCE `rdma_rxe` software device. Note that `rdma_rxe` needs kernel-module
loading (`modprobe rdma_rxe`), which is unavailable on hosted CI runners and
inside containers without a `/lib/modules` tree — so this is a **self-hosted
lab job**, not part of the hosted matrix. **TCP is the supported production
transport today and is feature-complete.**

## Windows: shell extension + WinFsp driver

Build on Windows with the MSVC toolchain:

```powershell
# Shell extension (compiles src/comshim.cpp via the build script)
cargo build -p atlas-shellext-win --release
# WinFsp filesystem driver (install WinFsp first: https://winfsp.dev)
cargo build -p atlas-wfsp --release
```

On non-Windows hosts both crates compile to inert shims so the workspace stays
green; the real implementations are behind `#[cfg(windows)]`.

## macOS: FileProvider extension

```bash
# Rust bridge:
cargo build -p atlas-fileprovider-mac --release
# Swift app extension (.appex), built + notarized via Xcode:
cd crates/atlas-fileprovider-mac/swift && swift build -c release
```

## Linux desktop: GVFS / KIO

The Rust core (`atlas-gvfs`) compiles everywhere and exposes C-ABI entry points
consumed by a thin `libgvfsbackend-atlas.so` (GLib/GIO) and a KIO worker.

**What is validated (no desktop daemon needed):**

1. `VfsCore` — the actual filesystem operations (`stat`/`list`/`read`/`write`/
   `mkdir`/`rename`/`delete`) driven through `atlas://` URIs against a real
   ATLAS store — is unit-tested end-to-end (`core::tests`).
2. The **C-ABI boundary** is exercised from real C. The crate builds a
   `cdylib` (`libatlas_gvfs.so`) and `ctest/abi_smoke.c` links it, calls
   `atlas_gvfs_mount_info` / `atlas_gvfs_free_string`, and checks the mount
   JSON plus null/bad-scheme rejection. Run by the `gvfs-abi` CI job:

   ```bash
   cargo build -p atlas-gvfs
   cc crates/atlas-gvfs/ctest/abi_smoke.c -L target/debug -latlas_gvfs -o abi_smoke
   LD_LIBRARY_PATH=target/debug ./abi_smoke      # -> "C-ABI smoke test PASSED"
   ```

**What still needs a desktop session:** the thin GLib/GIO backend and KIO
worker that call these entry points, built against your distro's
`libglib2.0-dev` / KDE `kio-dev` packages and registered under
`/usr/share/gvfs/mounts/`, then mounted with `gio mount atlas://…` and browsed
in Nautilus/Dolphin. That last mile is a desktop-session integration test, not
a headless CI job.

## CI matrix recommendation

- **linux-default** (this repo's default): `cargo build/test/clippy --workspace`
- **linux-fdb**: install libfdb_c 7.3 → `cargo test -p atlas-meta-fdb --features fdb` against a docker FDB
- **linux-rdma**: `cargo check -p atlas-transport --features rdma` (hardware lab job for runtime)
- **windows**: `cargo build -p atlas-shellext-win -p atlas-wfsp`
- **macos**: `cargo build -p atlas-fileprovider-mac` + `swift build`
