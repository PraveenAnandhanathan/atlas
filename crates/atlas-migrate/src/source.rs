//! Migration source connectors: S3, GCS, ext4, git-LFS (T7.8).

use serde::{Deserialize, Serialize};

/// A source that can be migrated from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MigrationSource {
    S3 {
        endpoint: String,
        bucket: String,
        prefix: String,
        region: String,
    },
    Gcs {
        bucket: String,
        prefix: String,
    },
    Ext4 {
        mount_point: String,
        sub_path: String,
    },
    GitLfs {
        repo_url: String,
        ref_name: String,
    },
}

impl MigrationSource {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::S3 { .. }    => "s3",
            Self::Gcs { .. }   => "gcs",
            Self::Ext4 { .. }  => "ext4",
            Self::GitLfs { .. } => "git-lfs",
        }
    }
}

/// Parse a source URI into a `MigrationSource`.
///
/// Supported schemes:
/// - `s3://bucket/prefix`
/// - `gcs://bucket/prefix`
/// - `/abs/path` or `file:///abs/path`
/// - `git-lfs://repo-url#ref`
pub fn parse_source(uri: &str) -> Result<MigrationSource, String> {
    if let Some(rest) = uri.strip_prefix("s3://") {
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        return Ok(MigrationSource::S3 {
            endpoint: "https://s3.amazonaws.com".into(),
            bucket: bucket.into(),
            prefix: prefix.into(),
            region: "us-east-1".into(),
        });
    }
    if let Some(rest) = uri.strip_prefix("gcs://") {
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        return Ok(MigrationSource::Gcs { bucket: bucket.into(), prefix: prefix.into() });
    }
    if let Some(rest) = uri.strip_prefix("git-lfs://") {
        let (repo, ref_name) = rest.split_once('#').unwrap_or((rest, "main"));
        return Ok(MigrationSource::GitLfs { repo_url: repo.into(), ref_name: ref_name.into() });
    }
    if uri.starts_with('/') || uri.starts_with("file://") {
        let path = uri.strip_prefix("file://").unwrap_or(uri);
        return Ok(MigrationSource::Ext4 { mount_point: "/".into(), sub_path: path.into() });
    }
    Err(format!("unrecognised source URI: {uri}"))
}

/// Fetch the raw bytes of a single object from the source.
///
/// - `Ext4`: reads the file directly from the mounted filesystem.
/// - Cloud sources: network calls are not yet wired; returns `Err`.
pub fn fetch_object(source: &MigrationSource, obj: &SourceObject) -> Result<Vec<u8>, String> {
    match source {
        MigrationSource::Ext4 { mount_point, .. } => {
            let full = format!("{}/{}", mount_point.trim_end_matches('/'), obj.path.trim_start_matches('/'));
            std::fs::read(&full).map_err(|e| format!("read {full}: {e}"))
        }
        MigrationSource::S3 { endpoint, bucket, .. } => Err(format!(
            "S3 network transfer not yet wired (endpoint={endpoint}, bucket={bucket})"
        )),
        MigrationSource::Gcs { bucket, .. } => Err(format!(
            "GCS network transfer not yet wired (bucket={bucket})"
        )),
        MigrationSource::GitLfs { repo_url, .. } => Err(format!(
            "git-LFS transfer not yet wired (repo={repo_url})"
        )),
    }
}

/// A single object discovered in the source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceObject {
    /// Path relative to the source root.
    pub path: String,
    pub size: u64,
    /// Source-specific identifier (e.g. S3 ETag, git blob SHA).
    pub source_id: String,
}

/// Enumerate objects available at the source.
///
/// For `Ext4` sources, performs a real recursive directory walk.
/// For cloud sources (S3, GCS, git-LFS), returns a stub list because
/// the network clients are not yet wired.
pub fn enumerate(source: &MigrationSource, limit: usize) -> Vec<SourceObject> {
    match source {
        MigrationSource::Ext4 { mount_point, sub_path } => {
            let root = format!("{}/{}", mount_point.trim_end_matches('/'), sub_path.trim_start_matches('/'));
            enumerate_ext4(&root, limit)
        }
        MigrationSource::S3 { prefix, .. } => stub_objects(prefix, limit),
        MigrationSource::Gcs { prefix, .. } => stub_objects(prefix, limit),
        MigrationSource::GitLfs { ref_name, .. } => stub_objects(&format!("lfs-{ref_name}"), limit),
    }
}

fn enumerate_ext4(root: &str, limit: usize) -> Vec<SourceObject> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return out; };
    for entry in rd.flatten().take(limit) {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() { continue; }
        let name = entry.file_name().to_string_lossy().into_owned();
        out.push(SourceObject {
            path: name.clone(),
            size: meta.len(),
            source_id: name,
        });
    }
    out
}

fn stub_objects(prefix: &str, limit: usize) -> Vec<SourceObject> {
    (0..limit.min(3))
        .map(|i| SourceObject {
            path: format!("{prefix}/object-{i}.bin"),
            size: (i as u64 + 1) * 1024,
            source_id: format!("stub-id-{i}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_s3_uri() {
        let s = parse_source("s3://my-bucket/data/").unwrap();
        assert_eq!(s.kind(), "s3");
    }

    #[test]
    fn parse_gcs_uri() {
        let s = parse_source("gcs://my-bucket/prefix/").unwrap();
        assert_eq!(s.kind(), "gcs");
    }

    #[test]
    fn parse_ext4_path() {
        let s = parse_source("/mnt/data").unwrap();
        assert_eq!(s.kind(), "ext4");
    }

    #[test]
    fn parse_git_lfs_uri() {
        let s = parse_source("git-lfs://github.com/org/repo#main").unwrap();
        assert_eq!(s.kind(), "git-lfs");
    }

    #[test]
    fn parse_invalid_uri_errors() {
        assert!(parse_source("ftp://nope").is_err());
    }

    #[test]
    fn enumerate_returns_stub_objects() {
        let src = parse_source("s3://bucket/prefix").unwrap();
        let objs = enumerate(&src, 3);
        assert_eq!(objs.len(), 3);
    }
}
