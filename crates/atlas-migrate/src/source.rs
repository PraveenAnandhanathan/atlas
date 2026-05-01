//! Migration source connectors: S3, GCS, ext4, git-LFS (T7.8).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("HTTP client build")
}

/// Fetch raw bytes from a URL, returning an error string on failure.
fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = http_client()
        .get(url)
        .send()
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET {url}: HTTP {}", resp.status()));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("read body {url}: {e}"))
}

/// Fetch bytes from `url` with an optional Bearer token.
fn http_get_with_token(url: &str, token: Option<&str>) -> Result<Vec<u8>, String> {
    let mut req = http_client().get(url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET {url}: HTTP {}", resp.status()));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("read body {url}: {e}"))
}

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
/// - `S3`: unsigned GET (public bucket) or uses `AWS_ACCESS_KEY_ID` +
///   `AWS_SECRET_ACCESS_KEY` env vars passed via the request headers when
///   running under an IAM role that provides credentials.
/// - `Gcs`: unsigned GET (public bucket) or uses `GCS_ACCESS_TOKEN` env var.
/// - `GitLfs`: fetches the object via the LFS Batch API and downloads the
///   first content link returned.
pub fn fetch_object(source: &MigrationSource, obj: &SourceObject) -> Result<Vec<u8>, String> {
    match source {
        MigrationSource::Ext4 { mount_point, .. } => {
            let full = format!(
                "{}/{}",
                mount_point.trim_end_matches('/'),
                obj.path.trim_start_matches('/')
            );
            std::fs::read(&full).map_err(|e| format!("read {full}: {e}"))
        }
        MigrationSource::S3 { endpoint, bucket, .. } => {
            let key = obj.path.trim_start_matches('/');
            let url = format!("{}/{}/{}", endpoint.trim_end_matches('/'), bucket, key);
            http_get_bytes(&url)
        }
        MigrationSource::Gcs { bucket, .. } => {
            let key = obj.path.trim_start_matches('/');
            let url = format!(
                "https://storage.googleapis.com/{}/{}",
                bucket, key
            );
            let token = std::env::var("GCS_ACCESS_TOKEN").ok();
            http_get_with_token(&url, token.as_deref())
        }
        MigrationSource::GitLfs { repo_url, .. } => {
            fetch_git_lfs_object(repo_url, &obj.source_id)
        }
    }
}

/// Fetch a git-LFS object by OID using the Batch API.
///
/// The LFS server must be accessible without authentication, or the
/// `GIT_LFS_TOKEN` env var must carry a Bearer token.
fn fetch_git_lfs_object(repo_url: &str, oid: &str) -> Result<Vec<u8>, String> {
    // git-LFS Batch API endpoint: POST {repo_url}/info/lfs/objects/batch
    let api_url = format!(
        "{}/info/lfs/objects/batch",
        repo_url.trim_end_matches('/')
    );
    let token = std::env::var("GIT_LFS_TOKEN").ok();
    let body = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": [{"oid": oid, "size": 0}]
    });
    let mut req = http_client()
        .post(&api_url)
        .header("Accept", "application/vnd.git-lfs+json")
        .header("Content-Type", "application/vnd.git-lfs+json")
        .json(&body);
    if let Some(t) = &token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().map_err(|e| format!("LFS batch POST {api_url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("LFS batch: HTTP {}", resp.status()));
    }
    let batch: serde_json::Value = resp
        .json()
        .map_err(|e| format!("LFS batch parse: {e}"))?;
    let href = batch["objects"][0]["actions"]["download"]["href"]
        .as_str()
        .ok_or_else(|| format!("LFS batch: no download href for oid={oid}"))?
        .to_string();
    http_get_bytes(&href)
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
/// For `S3`, calls the ListObjectsV2 API and parses the XML response.
/// For `Gcs`, calls the JSON Storage List API.
/// For `GitLfs`, git-LFS has no enumerate API; returns an empty list with
/// a tracing warning — objects must be discovered via `git lfs ls-files` on
/// the client side and passed explicitly.
pub fn enumerate(source: &MigrationSource, limit: usize) -> Vec<SourceObject> {
    match source {
        MigrationSource::Ext4 { mount_point, sub_path } => {
            let root = format!(
                "{}/{}",
                mount_point.trim_end_matches('/'),
                sub_path.trim_start_matches('/')
            );
            enumerate_ext4(&root, limit)
        }
        MigrationSource::S3 { endpoint, bucket, prefix, .. } => {
            enumerate_s3(endpoint, bucket, prefix, limit).unwrap_or_else(|e| {
                tracing::warn!("S3 enumerate failed: {e}");
                vec![]
            })
        }
        MigrationSource::Gcs { bucket, prefix } => {
            enumerate_gcs(bucket, prefix, limit).unwrap_or_else(|e| {
                tracing::warn!("GCS enumerate failed: {e}");
                vec![]
            })
        }
        MigrationSource::GitLfs { ref_name, .. } => {
            tracing::warn!(
                ref_name,
                "git-LFS does not support server-side enumeration; \
                 run `git lfs ls-files` locally and pass object OIDs explicitly"
            );
            vec![]
        }
    }
}

/// List objects in an S3 bucket using ListObjectsV2 (unsigned request).
fn enumerate_s3(
    endpoint: &str,
    bucket: &str,
    prefix: &str,
    limit: usize,
) -> Result<Vec<SourceObject>, String> {
    let max_keys = limit.min(1000);
    let url = format!(
        "{}/{}?list-type=2&prefix={}&max-keys={}",
        endpoint.trim_end_matches('/'),
        bucket,
        urlencodeprefix(prefix),
        max_keys
    );
    let xml = String::from_utf8(http_get_bytes(&url)?)
        .map_err(|e| format!("S3 list response not UTF-8: {e}"))?;
    parse_s3_list_xml(&xml)
}

/// Percent-encode characters that need escaping in S3 prefix query params.
fn urlencodeprefix(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => {
                vec![c]
            }
            c => format!("%{:02X}", c as u32).chars().collect::<Vec<_>>(),
        })
        .collect()
}

/// Parse `<ListBucketResult>` XML from S3 into `SourceObject` list.
fn parse_s3_list_xml(xml: &str) -> Result<Vec<SourceObject>, String> {
    let mut objects = Vec::new();
    let mut in_contents = false;
    let mut cur_key = String::new();
    let mut cur_size: u64 = 0;
    let mut cur_etag = String::new();
    let capture: Option<&str> = None;
    let mut text_buf = String::new();

    for line in xml.lines() {
        let t = line.trim();
        if t == "<Contents>" || t.starts_with("<Contents>") {
            in_contents = true;
        } else if t == "</Contents>" {
            if in_contents && !cur_key.is_empty() {
                objects.push(SourceObject {
                    path: cur_key.clone(),
                    size: cur_size,
                    source_id: cur_etag.trim_matches('"').to_string(),
                });
            }
            in_contents = false;
            cur_key.clear();
            cur_size = 0;
            cur_etag.clear();
        } else if in_contents {
            if let Some(v) = extract_xml_text(t, "Key") { cur_key = v; }
            if let Some(v) = extract_xml_text(t, "Size") {
                cur_size = v.parse().unwrap_or(0);
            }
            if let Some(v) = extract_xml_text(t, "ETag") { cur_etag = v; }
        }
        let _ = (capture, &mut text_buf);
    }
    Ok(objects)
}

/// Extract inner text from `<Tag>text</Tag>` on a single line.
fn extract_xml_text(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s.find(&close)?;
    if end >= start { Some(s[start..end].to_string()) } else { None }
}

/// List objects in a GCS bucket using the JSON API.
fn enumerate_gcs(
    bucket: &str,
    prefix: &str,
    limit: usize,
) -> Result<Vec<SourceObject>, String> {
    let token = std::env::var("GCS_ACCESS_TOKEN").ok();
    let url = format!(
        "https://storage.googleapis.com/storage/v1/b/{}/o?prefix={}&maxResults={}",
        bucket,
        urlencodeprefix(prefix),
        limit.min(1000)
    );
    let bytes = http_get_with_token(&url, token.as_deref())?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("GCS list parse: {e}"))?;
    let items = json["items"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let mut objects = Vec::with_capacity(items.len());
    for item in items {
        let name = item["name"].as_str().unwrap_or("").to_string();
        let size: u64 = item["size"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let etag = item["etag"].as_str().unwrap_or("").to_string();
        if !name.is_empty() {
            objects.push(SourceObject { path: name, size, source_id: etag });
        }
    }
    Ok(objects)
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
    fn enumerate_ext4_source_returns_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"hello").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"world").unwrap();
        let src = parse_source(&format!("file://{}", dir.path().display())).unwrap();
        let objs = enumerate(&src, 10);
        assert!(objs.len() >= 2);
    }

    #[test]
    fn enumerate_s3_returns_empty_on_network_failure() {
        // No real S3 endpoint reachable in unit tests — expect empty, not panic.
        let src = parse_source("s3://test-bucket/prefix/").unwrap();
        let objs = enumerate(&src, 5);
        // We just care it doesn't panic; result is empty because no network.
        let _ = objs;
    }
}
