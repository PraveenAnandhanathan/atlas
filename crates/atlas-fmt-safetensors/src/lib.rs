//! Safetensors format plugin (T1.8).
//!
//! Phase 1 scope: parse the header so ATLAS can answer
//! "what tensors are in this file?" and "where do the bytes for
//! `tensor_name` live?" without materializing the whole blob.
//!
//! The full feature — `read_tensor(path, tensor_name)` returning a
//! zero-copy slice from the chunk store — lands once we wire this
//! through `atlas_fs` as a format-aware reader. The header parser
//! here is the stable building block.
//!
//! On-disk layout (per the safetensors spec):
//!   - `[u64 little-endian]` header_size
//!   - `[u8; header_size]`   header (UTF-8 JSON)
//!   - `[u8; ...]`           tensor data, dense
//!
//! The header is a JSON object: `tensor_name -> {dtype, shape, data_offsets: [start, end]}`
//! plus an optional `__metadata__` field for free-form metadata.

use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SafetensorsError {
    #[error("file too small to contain a header")]
    TooSmall,
    #[error("declared header size ({0}) exceeds file size")]
    HeaderOverflow(u64),
    #[error("malformed JSON header: {0}")]
    BadJson(String),
    #[error("tensor not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub tensors: BTreeMap<String, TensorInfo>,
    pub metadata: BTreeMap<String, String>,
    /// Byte offset where tensor data begins (i.e. 8 + header_size).
    pub data_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TensorInfo {
    pub dtype: String,
    pub shape: Vec<usize>,
    /// `[start, end)` byte range relative to `data_offset`.
    pub data_offsets: [u64; 2],
}

#[derive(Debug, Deserialize)]
struct RawHeader(BTreeMap<String, serde_json::Value>);

/// Parse a safetensors header from the start of `bytes`.
///
/// Returns the parsed [`Header`]; subsequent reads can use
/// `tensor.data_offsets` plus `header.data_offset` to locate any
/// tensor's bytes inside the underlying blob (or map the byte range
/// onto chunk hashes via the [`atlas_object::BlobManifest`]).
pub fn parse_header(bytes: &[u8]) -> Result<Header, SafetensorsError> {
    if bytes.len() < 8 {
        return Err(SafetensorsError::TooSmall);
    }
    let mut size_bytes = [0u8; 8];
    size_bytes.copy_from_slice(&bytes[..8]);
    let header_size = u64::from_le_bytes(size_bytes);
    let total = 8u64.saturating_add(header_size);
    if total > bytes.len() as u64 {
        return Err(SafetensorsError::HeaderOverflow(header_size));
    }
    let header_bytes = &bytes[8..(8 + header_size as usize)];
    let raw: RawHeader = serde_json::from_slice(header_bytes)
        .map_err(|e| SafetensorsError::BadJson(e.to_string()))?;

    let mut tensors = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    for (k, v) in raw.0 {
        if k == "__metadata__" {
            if let Some(obj) = v.as_object() {
                for (mk, mv) in obj {
                    if let Some(s) = mv.as_str() {
                        metadata.insert(mk.clone(), s.to_string());
                    }
                }
            }
            continue;
        }
        let info: TensorInfo = serde_json::from_value(v)
            .map_err(|e| SafetensorsError::BadJson(format!("tensor {k}: {e}")))?;
        tensors.insert(k, info);
    }

    Ok(Header {
        tensors,
        metadata,
        data_offset: 8 + header_size,
    })
}

/// Look up the absolute byte range for a tensor in the underlying blob.
pub fn tensor_byte_range(
    header: &Header,
    tensor_name: &str,
) -> Result<(u64, u64), SafetensorsError> {
    let t = header
        .tensors
        .get(tensor_name)
        .ok_or_else(|| SafetensorsError::NotFound(tensor_name.into()))?;
    let start = header.data_offset + t.data_offsets[0];
    let end = header.data_offset + t.data_offsets[1];
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthesize(header_json: &str, tensor_bytes: &[u8]) -> Vec<u8> {
        let header = header_json.as_bytes();
        let mut out = Vec::with_capacity(8 + header.len() + tensor_bytes.len());
        out.extend_from_slice(&(header.len() as u64).to_le_bytes());
        out.extend_from_slice(header);
        out.extend_from_slice(tensor_bytes);
        out
    }

    #[test]
    fn parses_single_tensor_header() {
        let json = r#"{"q":{"dtype":"F32","shape":[2,2],"data_offsets":[0,16]}}"#;
        let bytes = synthesize(json, &[0u8; 16]);
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.tensors.len(), 1);
        let q = h.tensors.get("q").unwrap();
        assert_eq!(q.dtype, "F32");
        assert_eq!(q.shape, vec![2, 2]);
        let (start, end) = tensor_byte_range(&h, "q").unwrap();
        assert_eq!(end - start, 16);
    }

    #[test]
    fn parses_metadata() {
        let json = r#"{"__metadata__":{"format":"pt","author":"alice"},
                       "w":{"dtype":"F16","shape":[1],"data_offsets":[0,2]}}"#;
        let bytes = synthesize(json, &[0u8; 2]);
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.metadata.get("format").map(String::as_str), Some("pt"));
        assert_eq!(h.metadata.get("author").map(String::as_str), Some("alice"));
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(matches!(
            parse_header(&[0u8; 4]),
            Err(SafetensorsError::TooSmall)
        ));
    }

    #[test]
    fn rejects_oversized_header() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u64::MAX.to_le_bytes());
        bytes.extend_from_slice(b"{}");
        assert!(matches!(
            parse_header(&bytes),
            Err(SafetensorsError::HeaderOverflow(_))
        ));
    }

    #[test]
    fn rejects_invalid_json() {
        let bytes = synthesize("{not json}", &[]);
        assert!(matches!(
            parse_header(&bytes),
            Err(SafetensorsError::BadJson(_))
        ));
    }

    #[test]
    fn unknown_tensor_is_not_found() {
        let json = r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
        let bytes = synthesize(json, &[0u8; 4]);
        let h = parse_header(&bytes).unwrap();
        assert!(matches!(
            tensor_byte_range(&h, "missing"),
            Err(SafetensorsError::NotFound(_))
        ));
    }
}
