//! ATLAS S3 v4 gateway (T5.6).
//!
//! Mapping (design §8): `bucket = volume`, `key = path inside the volume`.
//! All requests funnel through [`atlas_mcp::CapabilityCore`] for
//! governance.  This crate handles SigV4 verification (see [`sigv4`])
//! and the four operations real-world clients hit hardest:
//!
//! - `GET /bucket/key` → `atlas.fs.read`
//! - `PUT /bucket/key` → `atlas.fs.write`
//! - `HEAD /bucket/key` → `atlas.fs.stat`
//! - `DELETE /bucket/key` → `atlas.fs.delete`
//! - `GET /bucket?list-type=2` → `atlas.fs.list`
//!
//! Streaming, multipart, and ACLs land later — v1 ships the
//! single-shot path so `aws s3 cp` works.

pub mod sigv4;

use atlas_mcp::{ApiError, CapabilityCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum S3Op {
    GetObject,
    PutObject,
    HeadObject,
    DeleteObject,
    ListObjectsV2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Request {
    pub op: S3Op,
    pub bucket: String,
    pub key: String,
    pub principal: Option<String>,
    /// Body bytes for PUT (hex-encoded for transport simplicity).
    #[serde(default)]
    pub body_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Response {
    pub status: u16,
    pub body_hex: String,
    pub headers: Vec<(String, String)>,
    pub xml: Option<String>,
}

impl S3Response {
    fn err(e: ApiError) -> Self {
        let (status, code) = match e.code {
            atlas_mcp::core::ErrorCode::NotFound => (404, "NoSuchKey"),
            atlas_mcp::core::ErrorCode::Forbidden => (403, "AccessDenied"),
            atlas_mcp::core::ErrorCode::InvalidArgument => (400, "InvalidRequest"),
            atlas_mcp::core::ErrorCode::NotImplemented => (501, "NotImplemented"),
            atlas_mcp::core::ErrorCode::Internal => (500, "InternalError"),
        };
        let xml = format!(
            "<?xml version=\"1.0\"?><Error><Code>{code}</Code><Message>{}</Message></Error>",
            xml_escape(&e.message)
        );
        Self {
            status,
            body_hex: String::new(),
            headers: vec![("content-type".into(), "application/xml".into())],
            xml: Some(xml),
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Translate `key` to an absolute ATLAS path.  Buckets stay informational
/// in v1 (single volume per gateway).
fn key_to_path(key: &str) -> String {
    if key.starts_with('/') {
        key.to_string()
    } else {
        format!("/{key}")
    }
}

/// Dispatch one S3 operation through the capability core.
pub fn handle(core: &CapabilityCore, req: &S3Request) -> S3Response {
    let principal = req.principal.as_deref().unwrap_or("s3-anon");
    let path = key_to_path(&req.key);
    match req.op {
        S3Op::GetObject => match core.invoke(principal, "atlas.fs.read", &json!({"path": path})) {
            Ok(v) => {
                let bytes_hex = v["bytes_hex"].as_str().unwrap_or("").to_string();
                S3Response {
                    status: 200,
                    body_hex: bytes_hex,
                    headers: vec![(
                        "content-length".into(),
                        v["size"].as_u64().unwrap_or(0).to_string(),
                    )],
                    xml: None,
                }
            }
            Err(e) => S3Response::err(e),
        },
        S3Op::PutObject => {
            let bytes = match hex::decode(&req.body_hex) {
                Ok(b) => b,
                Err(e) => return S3Response::err(ApiError::invalid(e.to_string())),
            };
            let content = String::from_utf8(bytes).unwrap_or_default();
            match core.invoke(
                principal,
                "atlas.fs.write",
                &json!({"path": path, "content": content}),
            ) {
                Ok(v) => S3Response {
                    status: 200,
                    body_hex: String::new(),
                    headers: vec![("etag".into(), v["hash"].as_str().unwrap_or("").to_string())],
                    xml: None,
                },
                Err(e) => S3Response::err(e),
            }
        }
        S3Op::HeadObject => match core.invoke(principal, "atlas.fs.stat", &json!({"path": path})) {
            Ok(v) => S3Response {
                status: 200,
                body_hex: String::new(),
                headers: vec![
                    ("etag".into(), v["hash"].as_str().unwrap_or("").to_string()),
                    (
                        "content-length".into(),
                        v["size"].as_u64().unwrap_or(0).to_string(),
                    ),
                ],
                xml: None,
            },
            Err(e) => S3Response::err(e),
        },
        S3Op::DeleteObject => {
            match core.invoke(principal, "atlas.fs.delete", &json!({"path": path})) {
                Ok(_) => S3Response {
                    status: 204,
                    body_hex: String::new(),
                    headers: vec![],
                    xml: None,
                },
                Err(e) => S3Response::err(e),
            }
        }
        S3Op::ListObjectsV2 => {
            let prefix = if req.key.is_empty() { "/" } else { &path };
            let listed = match core.invoke(
                principal,
                "atlas.fs.list",
                &json!({"path": prefix}),
            ) {
                Ok(v) => v,
                Err(e) => return S3Response::err(e),
            };
            let entries = listed.as_array().cloned().unwrap_or_default();
            let mut xml = String::from(
                "<?xml version=\"1.0\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
            );
            xml.push_str(&format!("<Name>{}</Name>", xml_escape(&req.bucket)));
            for e in entries {
                let p = e["path"].as_str().unwrap_or("").trim_start_matches('/');
                let h = e["hash"].as_str().unwrap_or("");
                let size = e["size"].as_u64().unwrap_or(0);
                xml.push_str(&format!(
                    "<Contents><Key>{}</Key><ETag>{}</ETag><Size>{}</Size></Contents>",
                    xml_escape(p),
                    h,
                    size
                ));
            }
            xml.push_str("</ListBucketResult>");
            S3Response {
                status: 200,
                body_hex: String::new(),
                headers: vec![("content-type".into(), "application/xml".into())],
                xml: Some(xml),
            }
        }
    }
}

/// Helper for callers that don't use serde — builds the request from
/// raw S3 wire fields.
pub fn make_request(
    method: &str,
    bucket: &str,
    key: &str,
    body: Option<&[u8]>,
) -> Option<S3Request> {
    let op = match (method.to_ascii_uppercase().as_str(), key.is_empty()) {
        ("GET", true) => S3Op::ListObjectsV2,
        ("GET", false) => S3Op::GetObject,
        ("PUT", _) => S3Op::PutObject,
        ("HEAD", _) => S3Op::HeadObject,
        ("DELETE", _) => S3Op::DeleteObject,
        _ => return None,
    };
    Some(S3Request {
        op,
        bucket: bucket.into(),
        key: key.into(),
        principal: None,
        body_hex: body.map(hex::encode).unwrap_or_default(),
    })
}

/// Re-export for adapters that want to verify SigV4 before calling [`handle`].
pub use sigv4::{sign as sigv4_sign, verify as sigv4_verify, SignedRequest};

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_fs::Fs;
    use std::sync::Arc;

    fn core() -> CapabilityCore {
        let d = tempfile::tempdir().unwrap();
        let fs = Fs::init(d.path()).unwrap();
        std::mem::forget(d);
        CapabilityCore::new(Arc::new(fs))
    }

    #[test]
    fn put_then_get_round_trip() {
        let c = core();
        let put = handle(
            &c,
            &S3Request {
                op: S3Op::PutObject,
                bucket: "vol".into(),
                key: "hello".into(),
                principal: Some("u".into()),
                body_hex: hex::encode(b"hello world"),
            },
        );
        assert_eq!(put.status, 200);
        let get = handle(
            &c,
            &S3Request {
                op: S3Op::GetObject,
                bucket: "vol".into(),
                key: "hello".into(),
                principal: Some("u".into()),
                body_hex: String::new(),
            },
        );
        assert_eq!(get.status, 200);
        let bytes = hex::decode(&get.body_hex).unwrap();
        assert_eq!(bytes, b"hello world");
    }

    #[test]
    fn list_returns_xml() {
        let c = core();
        handle(
            &c,
            &S3Request {
                op: S3Op::PutObject,
                bucket: "vol".into(),
                key: "a".into(),
                principal: Some("u".into()),
                body_hex: hex::encode(b"x"),
            },
        );
        let r = handle(
            &c,
            &S3Request {
                op: S3Op::ListObjectsV2,
                bucket: "vol".into(),
                key: "".into(),
                principal: Some("u".into()),
                body_hex: String::new(),
            },
        );
        assert_eq!(r.status, 200);
        assert!(r.xml.unwrap().contains("ListBucketResult"));
    }

    #[test]
    fn delete_returns_204() {
        let c = core();
        handle(
            &c,
            &S3Request {
                op: S3Op::PutObject,
                bucket: "vol".into(),
                key: "k".into(),
                principal: Some("u".into()),
                body_hex: hex::encode(b"x"),
            },
        );
        let r = handle(
            &c,
            &S3Request {
                op: S3Op::DeleteObject,
                bucket: "vol".into(),
                key: "k".into(),
                principal: Some("u".into()),
                body_hex: String::new(),
            },
        );
        assert_eq!(r.status, 204);
    }

    #[test]
    fn head_missing_is_404() {
        let c = core();
        let r = handle(
            &c,
            &S3Request {
                op: S3Op::HeadObject,
                bucket: "vol".into(),
                key: "ghost".into(),
                principal: Some("u".into()),
                body_hex: String::new(),
            },
        );
        assert_eq!(r.status, 404);
    }
}

// Suppress unused import warning for serde when no derives are exercised.
#[allow(dead_code)]
fn _value_marker(_v: Value) {}
