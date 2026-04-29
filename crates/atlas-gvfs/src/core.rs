//! Shared pure-Rust core for both GVFS and KIO backends (T6.5).

use atlas_fs::{Entry, Fs};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UriError {
    #[error("invalid atlas:// URI: {0}")]
    InvalidUri(String),
    #[error("filesystem error: {0}")]
    Fs(#[from] atlas_core::Error),
}

/// A parsed `atlas://` URI.
///
/// ```
/// // atlas://myhost/myvolume/path/to/file
/// //         ^^^^^^  ^^^^^^^^  ^^^^^^^^^^^^^^
/// //         host    volume    atlas_path
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasUri {
    pub host: String,
    pub volume: String,
    pub atlas_path: String,
}

impl AtlasUri {
    /// Parse an `atlas://` URI string.
    pub fn parse(uri: &str) -> Result<Self, UriError> {
        let rest = uri
            .strip_prefix("atlas://")
            .ok_or_else(|| UriError::InvalidUri(uri.to_string()))?;

        let slash = rest.find('/').unwrap_or(rest.len());
        let host = rest[..slash].to_string();
        let after_host = &rest[slash..];

        let slash2 = after_host[1..].find('/').map(|i| i + 1).unwrap_or(after_host.len());
        let volume = after_host[1..slash2].to_string();
        let atlas_path = if slash2 < after_host.len() {
            after_host[slash2..].to_string()
        } else {
            "/".to_string()
        };

        if volume.is_empty() {
            return Err(UriError::InvalidUri(format!("missing volume in {uri}")));
        }

        Ok(Self { host, volume, atlas_path: if atlas_path.is_empty() { "/".into() } else { atlas_path } })
    }

    /// Convert back to a canonical `atlas://` string.
    pub fn to_uri(&self) -> String {
        format!("atlas://{}/{}{}", self.host, self.volume, self.atlas_path)
    }
}

/// Shared logic called by both the GVFS backend and the KIO worker.
pub struct VfsCore {
    pub fs: Fs,
    pub volume: String,
}

impl VfsCore {
    pub fn new(fs: Fs, volume: impl Into<String>) -> Self {
        Self { fs, volume: volume.into() }
    }

    pub fn stat(&self, uri: &AtlasUri) -> Result<Entry, UriError> {
        Ok(self.fs.stat(&uri.atlas_path)?)
    }

    pub fn list(&self, uri: &AtlasUri) -> Result<Vec<Entry>, UriError> {
        Ok(self.fs.list(&uri.atlas_path)?)
    }

    pub fn read(&self, uri: &AtlasUri) -> Result<Vec<u8>, UriError> {
        Ok(self.fs.read(&uri.atlas_path)?)
    }

    pub fn write(&self, uri: &AtlasUri, data: &[u8]) -> Result<(), UriError> {
        self.fs.write(&uri.atlas_path, data)?;
        Ok(())
    }

    pub fn delete(&self, uri: &AtlasUri) -> Result<(), UriError> {
        Ok(self.fs.delete(&uri.atlas_path)?)
    }

    pub fn mkdir(&self, uri: &AtlasUri) -> Result<(), UriError> {
        Ok(self.fs.mkdir(&uri.atlas_path)?)
    }

    pub fn rename(&self, from: &AtlasUri, to: &AtlasUri) -> Result<(), UriError> {
        Ok(self.fs.rename(&from.atlas_path, &to.atlas_path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_uri() {
        let uri = AtlasUri::parse("atlas://myhost/myvolume/path/to/file.txt").unwrap();
        assert_eq!(uri.host, "myhost");
        assert_eq!(uri.volume, "myvolume");
        assert_eq!(uri.atlas_path, "/path/to/file.txt");
    }

    #[test]
    fn parse_volume_only() {
        let uri = AtlasUri::parse("atlas://localhost/myvol").unwrap();
        assert_eq!(uri.atlas_path, "/");
    }

    #[test]
    fn parse_wrong_scheme() {
        assert!(AtlasUri::parse("file:///foo").is_err());
    }

    #[test]
    fn parse_missing_volume() {
        assert!(AtlasUri::parse("atlas://host/").is_err());
    }

    #[test]
    fn round_trip() {
        let orig = "atlas://host/vol/a/b/c";
        let uri = AtlasUri::parse(orig).unwrap();
        assert_eq!(uri.to_uri(), orig);
    }
}
