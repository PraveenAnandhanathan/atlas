# Building the Toolchain-Gated Components

The default `cargo build --workspace` produces a complete, production-ready
ATLAS server, SDK, and CLI on Linux. A handful of components are gated behind
Cargo features or `#[cfg(target_os = …)]` because they require external
libraries, hardware, or a platform toolchain that is not present in a generic
Linux CI image. This document explains how to build and verify each one.

| Component | Gate | Requires | Verifiable in generic Linux CI? |
|-----------|------|----------|---------------------------------|
| FoundationDB metadata backend | `--features fdb` (crate `atlas-meta-fdb`) | `libfdb_c` 7.3 client + a running FDB cluster | Compiles ✅ / runtime needs cluster |
| RDMA transport | `--features rdma` (crate `atlas-transport`) | `libibverbs` + InfiniBand/RoCE NIC | Compiles ✅ / runtime needs hardware |
| Windows Explorer shell extension | `#[cfg(windows)]` (`atlas-shellext-win`) | Windows SDK + MSVC (`comshim.cpp`) | ❌ needs Windows |
| Windows WinFsp driver | `#[cfg(windows)]` (`atlas-wfsp`) | WinFsp runtime + MSVC | ❌ needs Windows |
| macOS FileProvider extension | `#[cfg(target_os="macos")]` (`atlas-fileprovider-mac`) | Xcode + Swift (`swift/` package) | ❌ needs macOS |
| GNOME/KDE virtual filesystem | `atlas-gvfs` C-ABI exports | GLib/GIO + KIO dev headers | Partially (Rust core ✅) |

## FoundationDB backend (`fdb` feature)

```bash
# Install the matching client (API version 7.3 — see workspace Cargo.toml).
curl -sSL -o fdb-clients.deb \
  https://github.com/apple/foundationdb/releases/download/7.3.27/foundationdb-clients_7.3.27-1_amd64.deb
sudo dpkg -i fdb-clients.deb

# Compile-check the Rust backend against the real client headers.
cargo check -p atlas-meta-fdb --features fdb

# To run integration tests you also need a cluster (foundationdb-server
# package, or a docker FDB). Point ATLAS at it with:
#   FdbMetaStore::open(Some("/etc/foundationdb/fdb.cluster"), "atlas")
```

The client API version is pinned in the workspace `Cargo.toml`
(`foundationdb = { version = "0.9", features = ["fdb-7_3"] }`). Installing a
client whose header reports an older API version fails the `foundationdb-sys`
build with *"Requested API version requires a newer version of this header"* —
match the client to the feature.

## RDMA transport (`rdma` feature)

```bash
sudo apt-get install -y libibverbs-dev librdmacm-dev
cargo check -p atlas-transport --features rdma     # compiles
```

The RDMA module compiles and allocates protection domains, completion queues,
queue pairs, and memory regions. The **data path** (QP state-machine
transitions INIT → RTR → RTS via `ibv_modify_qp`, plus the CQ-polling task that
bridges `ibv_post_send`/`ibv_post_recv` to the async channels) is intentionally
**not wired**: `RdmaTransport::connect` / `RdmaAcceptor::accept` return an
explicit error directing callers to `TcpTransport`. This is deliberate — silently
allocating hardware resources and then falling back to TCP would mislead callers
into believing they have RDMA connectivity. Completing the data path requires an
InfiniBand/RoCE NIC for verification (the generic CI image has no IB devices, so
`ibv_get_device_list` returns empty). **TCP is the supported production
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
consumed by a thin `libgvfsbackend-atlas.so` (GLib/GIO) and a KIO worker. Build
those native shims against your distro's `libglib2.0-dev` / KDE `kio-dev`
packages and register the backend under `/usr/share/gvfs/mounts/`.

## CI matrix recommendation

- **linux-default** (this repo's default): `cargo build/test/clippy --workspace`
- **linux-fdb**: install libfdb_c 7.3 → `cargo test -p atlas-meta-fdb --features fdb` against a docker FDB
- **linux-rdma**: `cargo check -p atlas-transport --features rdma` (hardware lab job for runtime)
- **windows**: `cargo build -p atlas-shellext-win -p atlas-wfsp`
- **macos**: `cargo build -p atlas-fileprovider-mac` + `swift build`
