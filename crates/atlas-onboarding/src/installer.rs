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
        register_shell_integration(config)?;
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

fn register_shell_integration(config: &InstallConfig) -> anyhow::Result<()> {
    match config.platform {
        Platform::Windows => register_shell_windows(config),
        Platform::MacOs => register_shell_macos(config),
        Platform::Linux => register_shell_linux(config),
    }
}

fn register_shell_windows(config: &InstallConfig) -> anyhow::Result<()> {
    // Register the COM shell-extension DLL with Windows Explorer via regsvr32.
    // The DLL must already be present at prefix\bin\atlas_shell.dll.
    let dll = config.prefix.join("bin").join("atlas_shell.dll");
    if !dll.exists() {
        tracing::warn!(dll = %dll.display(), "shell DLL not found; skipping regsvr32");
        return Ok(());
    }
    let status = std::process::Command::new("regsvr32")
        .args(["/s", &dll.to_string_lossy()])
        .status()
        .map_err(|e| anyhow::anyhow!("regsvr32: {e}"))?;
    if !status.success() {
        anyhow::bail!("regsvr32 failed with exit code {:?}", status.code());
    }
    tracing::info!("Windows shell extension registered via regsvr32");
    Ok(())
}

fn register_shell_macos(config: &InstallConfig) -> anyhow::Result<()> {
    // Write a LaunchAgent plist so the sync daemon starts at login.
    let launch_agents = dirs_macos_launch_agents();
    std::fs::create_dir_all(&launch_agents)?;
    let plist_path = launch_agents.join("io.atlasfs.sync.plist");
    let daemon_bin = config.prefix.join("bin").join("atlas-sync");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>       <string>io.atlasfs.sync</string>
    <key>ProgramArguments</key>
    <array>
        <string>{daemon}</string>
        <string>--store</string>
        <string>{store}</string>
    </array>
    <key>RunAtLoad</key>   <true/>
    <key>KeepAlive</key>   <true/>
</dict>
</plist>
"#,
        daemon = daemon_bin.display(),
        store = config.data_dir.display()
    );
    std::fs::write(&plist_path, &plist)?;

    // Load the agent now (non-fatal if launchctl is unavailable in tests).
    let _ = std::process::Command::new("launchctl")
        .args(["load", "-w", &plist_path.to_string_lossy()])
        .status();

    // Activate the Finder Sync extension if the .appex bundle exists.
    let appex = config.prefix
        .join("Applications")
        .join("ATLAS.app")
        .join("Contents")
        .join("PlugIns")
        .join("ATLASFinderSync.appex");
    if appex.exists() {
        let _ = std::process::Command::new("pluginkit")
            .args(["-a", &appex.to_string_lossy()])
            .status();
        tracing::info!(appex = %appex.display(), "Finder Sync extension activated");
    }

    tracing::info!(plist = %plist_path.display(), "macOS LaunchAgent registered");
    Ok(())
}

fn register_shell_linux(config: &InstallConfig) -> anyhow::Result<()> {
    // Register ATLAS MIME types so the file manager shows custom icons.
    let mime_xml = config.prefix.join("share").join("mime").join("packages").join("atlas.xml");
    if mime_xml.exists() {
        let _ = std::process::Command::new("xdg-mime")
            .args(["install", "--novendor", &mime_xml.to_string_lossy()])
            .status();
        tracing::info!(mime_xml = %mime_xml.display(), "xdg-mime types registered");
    }

    // Install the desktop file so the launcher picks up ATLAS Explorer.
    let desktop_file = config.prefix.join("share").join("applications").join("atlas-explorer.desktop");
    if desktop_file.exists() {
        let _ = std::process::Command::new("xdg-desktop-menu")
            .args(["install", "--novendor", &desktop_file.to_string_lossy()])
            .status();
        tracing::info!("xdg desktop menu entry installed");
    }

    // Write and enable a systemd user service for the sync daemon.
    if let Some(systemd_user) = dirs_systemd_user() {
        std::fs::create_dir_all(&systemd_user)?;
        let unit_path = systemd_user.join("atlas-sync.service");
        let daemon_bin = config.prefix.join("bin").join("atlas-sync");
        let unit = format!(
            "[Unit]\nDescription=ATLAS sync daemon\nAfter=network.target\n\n\
             [Service]\nExecStart={daemon} --store {store}\nRestart=on-failure\n\n\
             [Install]\nWantedBy=default.target\n",
            daemon = daemon_bin.display(),
            store = config.data_dir.display()
        );
        std::fs::write(&unit_path, &unit)?;
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "atlas-sync.service"])
            .status();
        tracing::info!(unit = %unit_path.display(), "systemd user service enabled");
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn dirs_macos_launch_agents() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())
    ).join("Library").join("LaunchAgents")
}

#[cfg(not(target_os = "macos"))]
fn dirs_macos_launch_agents() -> std::path::PathBuf {
    std::path::PathBuf::from("/tmp")
}

fn dirs_systemd_user() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".config").join("systemd").join("user"))
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
