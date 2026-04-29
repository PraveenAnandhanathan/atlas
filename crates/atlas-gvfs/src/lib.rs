//! ATLAS Linux file-manager integrations (T6.5).
//!
//! Two separate integration points are implemented here:
//!
//! - **GVFS backend** ([`gvfs`]): a GVfs `GVfsBackend` subclass (C, via
//!   `glib-sys`) that advertises the URI scheme `atlas://` to GNOME and
//!   GTK applications.  Nautilus mounts ATLAS volumes by opening URIs like
//!   `atlas://hostname/volume/`.  The backend translates GVfs I/O calls to
//!   [`atlas_fs::Fs`] operations.
//!
//! - **KIO worker** ([`kio`]): a KIO `WorkerBase` subclass (C++, generated
//!   from the Rust side) that handles the `atlas://` URL scheme inside
//!   KDE's Dolphin, Krusader, Kate, etc.  It ships as a shared library
//!   (`kio_atlas.so`) installed into `$KDE_INSTALL_PLUGINDIR/kio/`.
//!
//! - **Desktop-file registration** ([`desktop`]): `.desktop` and
//!   `.service` files that register the `atlas://` MIME handler with XDG.
//!
//! Both backends share the same pure-Rust core ([`core`]) that converts
//! URI paths to ATLAS paths and drives the [`atlas_fs::Fs`] engine.

pub mod core;
pub mod desktop;
pub mod gvfs;
pub mod kio;

pub use core::{AtlasUri, UriError, VfsCore};
