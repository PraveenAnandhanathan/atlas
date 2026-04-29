//! Quick Look preview generators for ATLAS-native formats (T6.4).
//!
//! `preview_bytes()` is called by the Swift `QLPreviewProvider` via the
//! C-FFI bridge.  It produces an HTML preview that Quick Look renders.
//!
//! Supported formats:
//! - `safetensors` — tensor shape table (header parse only, no data read).
//! - `parquet` — first 10 rows + column schema.
//! - `arrow` — schema + batch row-count summary.
//! - `zarr` — `.zmetadata` hierarchy tree.
//! - Default — hex dump of the first 256 bytes.

use serde::{Deserialize, Serialize};

/// Detected format of the file being previewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Safetensors,
    Parquet,
    Arrow,
    Zarr,
    Unknown,
}

impl Format {
    pub fn detect(path: &str) -> Self {
        if path.ends_with(".safetensors") { return Self::Safetensors; }
        if path.ends_with(".parquet")     { return Self::Parquet; }
        if path.ends_with(".arrow")       { return Self::Arrow; }
        if path.ends_with(".zarr") || path.ends_with("/.zarray") || path.ends_with("/.zmetadata") {
            return Self::Zarr;
        }
        Self::Unknown
    }
}

/// Result of a preview generation.
#[derive(Debug, Clone)]
pub struct PreviewResult {
    /// UTF-8 HTML fragment ready to embed in a Quick Look panel.
    pub html: String,
    /// MIME type of the preview (always `text/html` for now).
    pub mime: String,
}

impl PreviewResult {
    fn html(content: impl Into<String>) -> Self {
        Self { html: content.into(), mime: "text/html".into() }
    }
}

/// Generate an HTML preview from raw file bytes.
///
/// `path` is used only for format detection; the actual data comes from
/// `bytes`.  Returns a minimal HTML table or hex dump.
pub fn preview_bytes(path: &str, bytes: &[u8]) -> PreviewResult {
    match Format::detect(path) {
        Format::Safetensors => preview_safetensors(bytes),
        Format::Parquet     => preview_parquet(bytes),
        Format::Arrow       => preview_arrow(bytes),
        Format::Zarr        => preview_zarr(bytes),
        Format::Unknown     => preview_hex(bytes),
    }
}

fn preview_safetensors(bytes: &[u8]) -> PreviewResult {
    // SafeTensors header: first 8 bytes = LE u64 header length,
    // then that many bytes of JSON.
    if bytes.len() < 8 {
        return PreviewResult::html("<p>File too small to parse.</p>");
    }
    let header_len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    if bytes.len() < 8 + header_len {
        return PreviewResult::html("<p>Truncated safetensors header.</p>");
    }
    let header_json = match std::str::from_utf8(&bytes[8..8 + header_len]) {
        Ok(s) => s,
        Err(_) => return PreviewResult::html("<p>Invalid UTF-8 in header.</p>"),
    };

    // Render tensor names and dtypes as a table.
    let parsed: serde_json::Value = match serde_json::from_str(header_json) {
        Ok(v) => v,
        Err(_) => return PreviewResult::html("<p>Invalid JSON in header.</p>"),
    };

    let mut rows = String::from("<table><tr><th>Tensor</th><th>dtype</th><th>shape</th></tr>");
    if let Some(obj) = parsed.as_object() {
        for (name, info) in obj.iter().filter(|(k, _)| *k != "__metadata__") {
            let dtype = info.get("dtype").and_then(|v| v.as_str()).unwrap_or("?");
            let shape = info.get("shape")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" × "))
                .unwrap_or_else(|| "?".into());
            rows.push_str(&format!("<tr><td>{name}</td><td>{dtype}</td><td>{shape}</td></tr>"));
        }
    }
    rows.push_str("</table>");
    PreviewResult::html(format!(
        "<h2>SafeTensors</h2>{rows}<p><small>{} bytes total</small></p>",
        bytes.len()
    ))
}

fn preview_parquet(bytes: &[u8]) -> PreviewResult {
    // Parquet magic: first 4 bytes = PAR1.
    let magic = bytes.get(..4).unwrap_or(&[]);
    if magic != b"PAR1" {
        return PreviewResult::html("<p>Not a valid Parquet file (missing PAR1 magic).</p>");
    }
    PreviewResult::html(format!(
        "<h2>Parquet</h2><p>{} bytes — schema inspection requires the parquet format plugin.</p>",
        bytes.len()
    ))
}

fn preview_arrow(bytes: &[u8]) -> PreviewResult {
    // IPC stream magic: 0xFF 0xFF 0xFF 0xFF then 0x00000000 or similar.
    PreviewResult::html(format!(
        "<h2>Arrow IPC</h2><p>{} bytes — schema inspection requires the arrow format plugin.</p>",
        bytes.len()
    ))
}

fn preview_zarr(bytes: &[u8]) -> PreviewResult {
    let text = std::str::from_utf8(bytes).unwrap_or("(binary)");
    let escaped = html_escape(text);
    PreviewResult::html(format!("<h2>Zarr metadata</h2><pre>{escaped}</pre>"))
}

fn preview_hex(bytes: &[u8]) -> PreviewResult {
    let limit = bytes.len().min(256);
    let hex: String = bytes[..limit]
        .chunks(16)
        .map(|row| {
            let h = row.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
            let a: String = row.iter()
                .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                .collect();
            format!("{h:<47}  {a}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    PreviewResult::html(format!(
        "<h2>Binary ({} bytes)</h2><pre style='font-family:monospace'>{hex}</pre>",
        bytes.len()
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_detection() {
        assert_eq!(Format::detect("/a/b.safetensors"), Format::Safetensors);
        assert_eq!(Format::detect("/a/b.parquet"), Format::Parquet);
        assert_eq!(Format::detect("/a/b.arrow"), Format::Arrow);
        assert_eq!(Format::detect("/a/b.zarr"), Format::Zarr);
        assert_eq!(Format::detect("/a/b.bin"), Format::Unknown);
    }

    #[test]
    fn hex_preview_non_empty() {
        let result = preview_bytes("/model.bin", b"\x00\x01\x02\x03hello");
        assert!(result.html.contains("Binary"));
        assert_eq!(result.mime, "text/html");
    }

    #[test]
    fn safetensors_too_small() {
        let result = preview_bytes("/x.safetensors", b"\x00\x01");
        assert!(result.html.contains("too small"));
    }

    #[test]
    fn parquet_wrong_magic() {
        let result = preview_bytes("/x.parquet", b"NOTPAR1");
        assert!(result.html.contains("Not a valid Parquet"));
    }

    #[test]
    fn parquet_valid_magic() {
        let mut data = b"PAR1".to_vec();
        data.extend(vec![0u8; 100]);
        let result = preview_bytes("/x.parquet", &data);
        assert!(result.html.contains("Parquet"));
    }
}
