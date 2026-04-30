//! Format-aware text extraction (T3.4).
//!
//! Each format handler inspects either the file extension or magic bytes
//! and returns plain UTF-8 text suitable for indexing.
//!
//! Supported (real extraction):
//!   txt, md, rs, py, go, ts, js, sh, toml, yaml, yml, json, jsonl, csv, xml, html
//!   docx, doc, xlsx, xls, pptx, ppt  — ZIP + XML (Office Open XML)
//!   pdf                              — raw PDF stream scan (BT … ET text blocks)
//!
//! Binary-only (path returned; no content extraction):
//!   parquet, arrow, zarr, png/jpg, mp3/wav, safetensors/gguf

use std::str;
use tracing::debug;

/// Detect the file format from path + bytes and return extracted text.
pub fn extract_text(path: &str, bytes: &[u8]) -> String {
    let ext = extension(path);
    match ext {
        // Plain text / code files — decode as UTF-8, fall back lossy.
        "txt" | "md" | "rst" | "log" | "cfg" | "ini" | "env" | "gitignore" | "dockerignore"
        | "rs" | "py" | "go" | "ts" | "js" | "jsx" | "tsx" | "c" | "cpp" | "h" | "java" | "cs"
        | "rb" | "php" | "swift" | "kt" | "scala" | "sh" | "bash" | "zsh" | "fish" | "toml"
        | "yaml" | "yml" | "xml" | "html" | "htm" | "css" | "scss" | "sql" => {
            String::from_utf8_lossy(bytes).into_owned()
        }

        // JSON — concatenate all string values for richer indexing.
        "json" => extract_json(bytes),

        // JSONL — process each line.
        "jsonl" | "ndjson" => extract_jsonl(bytes),

        // CSV / TSV — extract all cell text.
        "csv" | "tsv" => extract_csv(bytes, if ext == "tsv" { b'\t' } else { b',' }),

        // Office Open XML formats (ZIP containers).
        "docx" | "doc" => extract_ooxml(bytes, OoxmlKind::Word),
        "xlsx" | "xls" => extract_ooxml(bytes, OoxmlKind::Excel),
        "pptx" | "ppt" => extract_ooxml(bytes, OoxmlKind::PowerPoint),

        // PDF — scan raw byte stream for text blocks.
        "pdf" => extract_pdf(bytes),

        // Columnar / array formats — extract schema/column names from file header.
        "parquet" => extract_parquet_schema(bytes),
        "arrow" | "ipc" | "feather" => extract_arrow_schema(bytes),
        "zarr" => extract_zarr_meta(bytes),

        // Image formats — no text extraction; indexed by path only.
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "svg" => {
            debug!(path, "image file — no text extraction");
            String::new()
        }

        // Audio — no text extraction.
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" => {
            debug!(path, "audio file — no text extraction");
            String::new()
        }

        // Binary ML artefacts — no text extraction.
        "safetensors" | "gguf" | "ggml" | "pt" | "pth" | "bin" | "npz" | "npy" => {
            debug!(path, "binary ML artefact — no text extraction");
            String::new()
        }

        // Unknown — try UTF-8 decode; if it fails emit empty.
        _ => {
            if looks_like_text(bytes) {
                String::from_utf8_lossy(bytes).into_owned()
            } else {
                String::new()
            }
        }
    }
}

// ---- Office Open XML (docx / xlsx / pptx) --------------------------------

#[derive(Clone, Copy)]
enum OoxmlKind {
    Word,
    Excel,
    PowerPoint,
}

impl OoxmlKind {
    /// Return the XML entry paths inside the ZIP that contain visible text.
    fn text_entry_prefix(self) -> &'static str {
        match self {
            OoxmlKind::Word => "word/document",
            OoxmlKind::Excel => "xl/sharedStrings",
            OoxmlKind::PowerPoint => "ppt/slides/slide",
        }
    }
}

/// Extract text from an Office Open XML archive (docx, xlsx, pptx).
///
/// All three formats are ZIP files containing XML. We:
/// 1. Open the ZIP in-memory.
/// 2. Enumerate entries matching the format-specific path prefix.
/// 3. Parse the XML and collect all text nodes (ignoring tags).
fn extract_ooxml(bytes: &[u8], kind: OoxmlKind) -> String {
    use std::io::{Cursor, Read};

    let cursor = Cursor::new(bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(e) => {
            debug!("ooxml zip open failed: {e}");
            return String::new();
        }
    };

    let prefix = kind.text_entry_prefix();
    let mut out = String::new();

    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| {
            let entry = zip.by_index(i).ok()?;
            let name = entry.name().to_owned();
            if name.contains(prefix) && name.ends_with(".xml") {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    for name in &names {
        let mut entry = match zip.by_name(name) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut xml_bytes = Vec::new();
        if entry.read_to_end(&mut xml_bytes).is_err() {
            continue;
        }
        let xml_text = String::from_utf8_lossy(&xml_bytes);
        extract_xml_text(&xml_text, &mut out);
        out.push(' ');
    }

    out.trim().to_string()
}

/// Walk an XML string and collect all character data (text nodes), skipping tags.
fn extract_xml_text(xml: &str, out: &mut String) {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    loop {
        match reader.read_event() {
            Ok(Event::Text(e)) => {
                if let Ok(text) = e.unescape() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push_str(trimmed);
                        out.push(' ');
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

// ---- PDF text extraction -------------------------------------------------

/// Extract text from a PDF by scanning for BT…ET (Begin Text…End Text) blocks
/// and collecting string literals inside parentheses `(...)` and hex strings `<…>`.
///
/// This is a simplified surface-level scanner — it handles the common case of
/// unencrypted PDFs with straightforward text streams. Encrypted PDFs and PDFs
/// with embedded CJK fonts return whatever printable ASCII is visible in the stream.
fn extract_pdf(bytes: &[u8]) -> String {
    // Find all content streams by looking for `stream\r\n` / `stream\n` markers.
    let mut out = String::new();
    let mut pos = 0;

    while pos < bytes.len() {
        // Find next `stream` keyword.
        let Some(stream_start) = find_subsequence(&bytes[pos..], b"stream") else {
            break;
        };
        let abs_start = pos + stream_start + 6; // skip past "stream"
        // Skip \r\n or \n after "stream".
        let data_start = skip_newline(bytes, abs_start);

        // Find the matching `endstream`.
        let Some(end_rel) = find_subsequence(&bytes[data_start..], b"endstream") else {
            break;
        };
        let data_end = data_start + end_rel;
        pos = data_end + 9; // continue after "endstream"

        let stream_data = &bytes[data_start..data_end];
        extract_pdf_stream(stream_data, &mut out);
    }

    out
}

fn extract_pdf_stream(data: &[u8], out: &mut String) {
    // Scan for BT...ET blocks.
    let mut i = 0;
    while i < data.len() {
        // Look for BT (Begin Text) marker.
        if i + 1 < data.len() && data[i] == b'B' && data[i + 1] == b'T' {
            i += 2;
            // Scan until ET (End Text).
            while i < data.len() {
                if i + 1 < data.len() && data[i] == b'E' && data[i + 1] == b'T' {
                    i += 2;
                    break;
                }
                // PDF string literal: (text here)
                if data[i] == b'(' {
                    i += 1;
                    let start = i;
                    let mut depth = 1u32;
                    while i < data.len() && depth > 0 {
                        match data[i] {
                            b'\\' => { i += 2; } // escape
                            b'(' => { depth += 1; i += 1; }
                            b')' => { depth -= 1; i += 1; }
                            _ => { i += 1; }
                        }
                    }
                    // data[start..i-1] is the raw string content.
                    let end = if i > 0 { i - 1 } else { i };
                    if let Ok(s) = std::str::from_utf8(&data[start..end]) {
                        let printable: String = s
                            .chars()
                            .filter(|c| c.is_ascii_graphic() || *c == ' ')
                            .collect();
                        if !printable.trim().is_empty() {
                            out.push_str(printable.trim());
                            out.push(' ');
                        }
                    }
                    continue;
                }
                // Hex string: <4865 6c6c 6f>
                if data[i] == b'<' && i + 1 < data.len() && data[i + 1] != b'<' {
                    i += 1;
                    let start = i;
                    while i < data.len() && data[i] != b'>' {
                        i += 1;
                    }
                    let hex_str: String = data[start..i]
                        .iter()
                        .filter(|&&b| !b.is_ascii_whitespace())
                        .map(|&b| b as char)
                        .collect();
                    // Decode pairs of hex digits into bytes.
                    let decoded: Vec<u8> = hex_str
                        .as_bytes()
                        .chunks(2)
                        .filter_map(|pair| {
                            let s = std::str::from_utf8(pair).ok()?;
                            u8::from_str_radix(s, 16).ok()
                        })
                        .collect();
                    if let Ok(s) = std::str::from_utf8(&decoded) {
                        let printable: String = s
                            .chars()
                            .filter(|c| c.is_ascii_graphic() || *c == ' ')
                            .collect();
                        if !printable.trim().is_empty() {
                            out.push_str(printable.trim());
                            out.push(' ');
                        }
                    }
                    if i < data.len() { i += 1; } // skip '>'
                    continue;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

// ---- Columnar format schema extraction -----------------------------------

/// Extract column names from Parquet file footer magic + schema.
///
/// A Parquet file ends with a 4-byte magic `PAR1` at both start and end.
/// The footer contains a Thrift-encoded `FileMetaData` struct whose field 2
/// (schema) is a list of `SchemaElement` objects each having a `name` string.
/// We do a lightweight scan: find all length-prefixed UTF-8 strings near the footer.
fn extract_parquet_schema(bytes: &[u8]) -> String {
    // Verify magic: first 4 bytes must be "PAR1".
    if bytes.len() < 8 || &bytes[..4] != b"PAR1" {
        return String::new();
    }

    // Footer length is the 4 bytes before the final "PAR1" magic.
    let footer_len_offset = bytes.len().saturating_sub(8);
    let footer_len = u32::from_le_bytes([
        bytes[footer_len_offset],
        bytes[footer_len_offset + 1],
        bytes[footer_len_offset + 2],
        bytes[footer_len_offset + 3],
    ]) as usize;

    let footer_start = bytes
        .len()
        .saturating_sub(8 + footer_len);
    let footer = &bytes[footer_start..footer_len_offset];

    // Scan footer for length-prefixed UTF-8 strings (Thrift binary: type=11 (string),
    // then 4-byte BE length, then bytes). We collect anything that looks like an
    // identifier (starts with a letter, all printable, length 1–128).
    extract_thrift_strings(footer)
}

/// Extract schema/column names from Arrow IPC file magic + metadata.
///
/// Arrow IPC files start with `ARROW1` (6 bytes) or `ARROW2` magic.
/// The schema is in the first message. We scan for length-prefixed strings.
fn extract_arrow_schema(bytes: &[u8]) -> String {
    const ARROW_MAGIC: &[u8] = b"ARROW1";
    if bytes.len() < 6 || &bytes[..6] != ARROW_MAGIC {
        return String::new();
    }
    // The schema message starts after the magic + padding (8 bytes total) and
    // a continuation marker (4 bytes 0xFF 0xFF 0xFF 0xFF) + metadata length (4 bytes).
    // Scan the first 64 KB for printable identifiers.
    let scan_region = &bytes[..bytes.len().min(65536)];
    extract_thrift_strings(scan_region)
}

/// Extract metadata from a Zarr archive (.zarr directory or zip).
///
/// Zarr v2 stores `.zarray` and `.zattrs` JSON metadata files.
/// Zarr v3 uses `zarr.json`. If `bytes` is a ZIP, open it and extract those files.
/// If `bytes` starts with `{`, try parsing it as JSON directly.
fn extract_zarr_meta(bytes: &[u8]) -> String {
    // Try as a ZIP container first.
    if bytes.starts_with(b"PK") {
        use std::io::{Cursor, Read};
        let cursor = Cursor::new(bytes);
        let Ok(mut zip) = zip::ZipArchive::new(cursor) else {
            return String::new();
        };
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| {
                let e = zip.by_index(i).ok()?;
                let n = e.name().to_owned();
                if n.ends_with(".zarray")
                    || n.ends_with(".zattrs")
                    || n.ends_with("zarr.json")
                {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        let mut out = String::new();
        for name in &names {
            let mut entry = match zip.by_name(name) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let mut buf = Vec::new();
            if entry.read_to_end(&mut buf).is_ok() {
                if let Ok(text) = std::str::from_utf8(&buf) {
                    out.push_str(&extract_json(text.as_bytes()));
                    out.push(' ');
                }
            }
        }
        return out.trim().to_string();
    }

    // Try as raw JSON metadata.
    if bytes.first() == Some(&b'{') {
        return extract_json(bytes);
    }

    String::new()
}

// ---- Shared helpers ------------------------------------------------------

/// Scan a byte buffer for Thrift-style length-prefixed strings and collect
/// those that look like schema field names (printable ASCII identifiers).
fn extract_thrift_strings(data: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i + 4 < data.len() {
        // Read a big-endian 4-byte length.
        let len = u32::from_be_bytes([data[i], data[i+1], data[i+2], data[i+3]]) as usize;
        if len == 0 || len > 256 || i + 4 + len > data.len() {
            i += 1;
            continue;
        }
        let candidate = &data[i + 4..i + 4 + len];
        if let Ok(s) = std::str::from_utf8(candidate) {
            // Accept strings that look like identifiers or labels.
            if s.chars().all(|c| c.is_ascii_alphanumeric() || "_-. /".contains(c))
                && s.chars().next().map_or(false, |c| c.is_alphanumeric() || c == '_')
            {
                out.push_str(s);
                out.push(' ');
                i += 4 + len;
                continue;
            }
        }
        i += 1;
    }
    out.trim().to_string()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

fn skip_newline(bytes: &[u8], pos: usize) -> usize {
    let mut p = pos;
    if p < bytes.len() && bytes[p] == b'\r' { p += 1; }
    if p < bytes.len() && bytes[p] == b'\n' { p += 1; }
    p
}

fn extension(path: &str) -> &str {
    path.rsplit('.')
        .next()
        .map(|e| e.split('?').next().unwrap_or(e))
        .unwrap_or("")
        .to_lowercase()
        .leak()
}

fn extract_json(bytes: &[u8]) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return String::new();
    };
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => collect_strings(&v),
        Err(_) => text.to_string(),
    }
}

fn extract_jsonl(bytes: &[u8]) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return String::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| collect_strings(&v))
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_strings(v: &serde_json::Value) -> String {
    let mut out = String::new();
    collect_strings_rec(v, &mut out);
    out
}

fn collect_strings_rec(v: &serde_json::Value, out: &mut String) {
    match v {
        serde_json::Value::String(s) => {
            out.push_str(s);
            out.push(' ');
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_strings_rec(x, out)),
        serde_json::Value::Object(m) => {
            for (k, val) in m {
                out.push_str(k);
                out.push(' ');
                collect_strings_rec(val, out);
            }
        }
        _ => {}
    }
}

fn extract_csv(bytes: &[u8], delimiter: u8) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return String::new();
    };
    text.lines()
        .flat_map(|line| {
            line.split(delimiter as char)
                .map(|cell| cell.trim().trim_matches('"'))
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Heuristic: if < 30% of the first 1024 bytes are non-ASCII-printable,
/// treat as text.
fn looks_like_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(1024)];
    if sample.is_empty() {
        return false;
    }
    let non_text = sample
        .iter()
        .filter(|&&b| !(b.is_ascii_graphic() || b.is_ascii_whitespace()))
        .count();
    non_text * 10 < sample.len() * 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_decoded() {
        let t = extract_text("/doc/notes.txt", b"hello world");
        assert_eq!(t, "hello world");
    }

    #[test]
    fn json_extracts_strings() {
        let j = r#"{"title":"atlas","version":1,"tags":["fast","safe"]}"#;
        let t = extract_text("/data.json", j.as_bytes());
        assert!(t.contains("atlas"));
        assert!(t.contains("fast"));
    }

    #[test]
    fn jsonl_extracts_all_lines() {
        let j = "{\"text\":\"first\"}\n{\"text\":\"second\"}\n";
        let t = extract_text("/data.jsonl", j.as_bytes());
        assert!(t.contains("first"));
        assert!(t.contains("second"));
    }

    #[test]
    fn csv_extracts_cells() {
        let c = "name,score\nalice,99\nbob,80\n";
        let t = extract_text("/scores.csv", c.as_bytes());
        assert!(t.contains("alice"));
        assert!(t.contains("bob"));
    }

    #[test]
    fn binary_returns_empty() {
        let b: Vec<u8> = (0..=255).collect();
        assert!(extract_text("/model.bin", &b).is_empty());
    }

    #[test]
    fn docx_extracts_text_via_ooxml() {
        // Build a minimal OOXML ZIP containing word/document.xml.
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(
                b"<w:document><w:body><w:p><w:r><w:t>Hello ATLAS</w:t></w:r></w:p></w:body></w:document>",
            ).unwrap();
            zip.finish().unwrap();
        }
        let text = extract_text("/report.docx", &buf);
        assert!(text.contains("Hello ATLAS"), "got: {text}");
    }

    #[test]
    fn xlsx_extracts_shared_strings() {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("xl/sharedStrings.xml", opts).unwrap();
            zip.write_all(
                b"<sst><si><t>Column A</t></si><si><t>Column B</t></si></sst>",
            ).unwrap();
            zip.finish().unwrap();
        }
        let text = extract_text("/data.xlsx", &buf);
        assert!(text.contains("Column A"), "got: {text}");
        assert!(text.contains("Column B"), "got: {text}");
    }

    #[test]
    fn pdf_extracts_bt_et_text() {
        // Minimal PDF snippet with a BT block containing a string literal.
        let pdf = b"%PDF-1.4\nstream\nBT (Hello from PDF) Tj ET\nendstream";
        let text = extract_text("/doc.pdf", pdf);
        assert!(text.contains("Hello from PDF"), "got: {text}");
    }

    #[test]
    fn pptx_extracts_slide_text() {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("ppt/slides/slide1.xml", opts).unwrap();
            zip.write_all(
                b"<p:sld><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Slide Title</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>",
            ).unwrap();
            zip.finish().unwrap();
        }
        let text = extract_text("/deck.pptx", &buf);
        assert!(text.contains("Slide Title"), "got: {text}");
    }
}
