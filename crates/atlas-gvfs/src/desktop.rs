//! XDG `.desktop` and D-Bus `.service` file generation (T6.5).

use serde::{Deserialize, Serialize};

/// An XDG `.desktop` entry for the ATLAS file manager integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub mime_types: Vec<String>,
}

impl Default for DesktopEntry {
    fn default() -> Self {
        Self {
            name: "ATLAS Explorer".into(),
            exec: "atlas-explorer %u".into(),
            icon: "atlas-explorer".into(),
            mime_types: vec![
                "x-scheme-handler/atlas".into(),
                "application/x-atlas-volume".into(),
            ],
        }
    }
}

impl DesktopEntry {
    /// Render as a `.desktop` file string.
    pub fn render(&self) -> String {
        format!(
            "[Desktop Entry]\n\
             Version=1.0\n\
             Type=Application\n\
             Name={}\n\
             Exec={}\n\
             Icon={}\n\
             MimeType={}\n\
             Categories=System;FileManager;\n\
             StartupNotify=true\n",
            self.name,
            self.exec,
            self.icon,
            self.mime_types.join(";"),
        )
    }
}

/// GVfs D-Bus service file that auto-starts the `gvfsd-atlas` daemon.
pub fn gvfs_service_file() -> String {
    "[D-BUS Service]\n\
     Name=org.gnome.VfsBackend.Atlas\n\
     Exec=/usr/lib/gvfs/gvfsd-atlas\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_render_contains_exec() {
        let entry = DesktopEntry::default();
        let rendered = entry.render();
        assert!(rendered.contains("atlas-explorer"));
        assert!(rendered.contains("[Desktop Entry]"));
    }

    #[test]
    fn gvfs_service_file_has_dbus_name() {
        let service = gvfs_service_file();
        assert!(service.contains("org.gnome.VfsBackend.Atlas"));
    }
}
