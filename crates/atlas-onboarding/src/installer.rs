//! Platform-specific installer logic (T6.7).

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Target installation platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    /// Detect the current compile-time platform.
    pub fn current() -> Self {
        #[cfg(target_os = "windows")]
        return Self::Windows;
        #[cfg(target_os = "macos")]
        return Self::MacOs;
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        Self::Linux
    }
}

/// Configuration for an installer run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    pub platform: Platform,
    /// Where to install binaries (e.g. `/usr/local/bin`, `C:\Program Files\ATLAS`).
    pub prefix: PathBuf,
    /// Where to write data (store root, config, cache).
    pub data_dir: PathBuf,
    /// Enable the FUSE/WinFsp/FileProvider mount at login.
    pub auto_mount: bool,
    /// Register shell extensions (Win) / Finder sync (macOS) / GVFS (Linux).
    pub shell_integration: bool,
    /// Install the ATLAS Explorer GUI.
    pub install_gui: bool,
}

impl Default for InstallConfig {
    fn default() -> Self {
        let platform = Platform::current();
        let (prefix, data_dir) = match platform {
            Platform::Windows => (
                PathBuf::from(r"C:\Program Files\ATLAS"),
                PathBuf::from(r"C:\ProgramData\ATLAS"),
            ),
            Platform::MacOs => (
                PathBuf::from("/usr/local"),
                PathBuf::from("/Library/Application Support/ATLAS"),
            ),
            Platform::Linux => (
                PathBuf::from("/usr/local"),
                PathBuf::from("/var/lib/atlas"),
            ),
        };
        Self {
            platform,
            prefix,
            data_dir,
            auto_mount: true,
            shell_integration: true,
            install_gui: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("insufficient privileges — run as administrator/root")]
    InsufficientPrivileges,
    #[error("install path not writable: {0}")]
    NotWritable(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("platform {0:?} not supported by this installer build")]
    UnsupportedPlatform(Platform),
    #[error("dependency missing: {0}")]
    MissingDependency(String),
}

/// Validate an `InstallConfig` without actually installing anything.
pub fn validate(config: &InstallConfig) -> Result<(), InstallError> {
    // Check prefix parent is writable as a proxy for privilege check.
    let parent = config.prefix.parent().unwrap_or(&config.prefix);
    if parent.exists() && !is_writable(parent) {
        return Err(InstallError::NotWritable(config.prefix.clone()));
    }
    Ok(())
}

/// Perform the installation described by `config`.
///
/// This is a simplified stub — the production implementation generates
/// the OS-specific package (MSI via WiX, `.pkg` via `pkgbuild`,
/// `.deb` via `dpkg-deb`) and registers system services.
pub fn install(config: &InstallConfig) -> anyhow::Result<()> {
    validate(config).context("config validation")?;
    std::fs::create_dir_all(&config.prefix).context("create prefix")?;
    std::fs::create_dir_all(&config.data_dir).context("create data_dir")?;

    // Write a marker so `atlasctl doctor` can find the install.
    let marker = config.prefix.join("atlas-install.json");
    std::fs::write(&marker, serde_json::to_string_pretty(config)?)
        .context("write install marker")?;

    if config.shell_integration {
        tracing::info!(platform = ?config.platform, "registering shell integration");
        // Real: call atlas_shellext_win::registry::register() on Windows,
        // launchctl + FinderSync appex on macOS, xdg-mime on Linux.
    }

    tracing::info!(prefix = %config.prefix.display(), "ATLAS installation complete");
    Ok(())
}

/// Remove an ATLAS installation.
pub fn uninstall(config: &InstallConfig) -> anyhow::Result<()> {
    let marker = config.prefix.join("atlas-install.json");
    if marker.exists() {
        std::fs::remove_file(&marker)?;
    }
    tracing::info!(prefix = %config.prefix.display(), "ATLAS uninstallation complete");
    Ok(())
}

fn is_writable(path: &Path) -> bool {
    // Heuristic: try to create a temp file.
    let tmp = path.join(".atlas_write_test");
    if std::fs::write(&tmp, b"").is_ok() {
        let _ = std::fs::remove_file(&tmp);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_config_platform() {
        let cfg = InstallConfig::default();
        assert_eq!(cfg.platform, Platform::current());
    }

    #[test]
    fn install_and_uninstall() {
        let dir = TempDir::new().unwrap();
        let cfg = InstallConfig {
            prefix: dir.path().join("prefix"),
            data_dir: dir.path().join("data"),
            shell_integration: false,
            ..InstallConfig::default()
        };
        install(&cfg).unwrap();
        assert!(cfg.prefix.join("atlas-install.json").exists());
        uninstall(&cfg).unwrap();
        assert!(!cfg.prefix.join("atlas-install.json").exists());
    }
}
