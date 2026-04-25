//! Format-aware text extraction (T3.4).
//!
//! Each format handler inspects either the file extension or magic bytes
//! and returns plain UTF-8 text suitable for indexing. Handlers that
//! require heavy dependencies (PDF, docx, audio, image, parquet, zarr)
//! are stubbed with an informative placeholder — their real bodies will
//! activate once the relevant Rust crates are integrated.
//!
//! Supported today (real extraction):
//!   txt, md, rs, py, go, ts, js, sh, toml, yaml, yml, json, jsonl, csv, xml, html
//!
//! Stubbed (returns empty string + log warning):
//!   pdf, docx, xlsx, pptx, parquet, arrow, zarr, png/jpg/gif/mp3/wav

use std::str;
use tracing::debug;

/// Detect the file format from path + bytes and return extracted text.
///
/// The returned string is plain UTF-8 suitable for Tantivy indexing.
/// An empty string is returned for binary files that can't be decoded yet.
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

        // JSON — decode as UTF-8; concatenate string values for richer indexing.
        "json" => extract_json(bytes),

        // JSONL — process each line.
        "jsonl" | "ndjson" => extract_jsonl(bytes),

        // CSV — extract all cell text.
        "csv" | "tsv" => extract_csv(bytes, if ext == "tsv" { b'\t' } else { b',' }),

        // Formats that need heavy dependencies — return stub text so the
        // path is at least findable by filename.
        "pdf" => stub(path, "pdf"),
        "docx" | "doc" => stub(path, "docx"),
        "xlsx" | "xls" => stub(path, "xlsx"),
        "pptx" | "ppt" => stub(path, "pptx"),
        "parquet" => stub(path, "parquet"),
        "arrow" | "ipc" | "feather" => stub(path, "arrow"),
        "zarr" => stub(path, "zarr"),

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

        // SafeTensors — just emit the filename (real slice-read is atlas-fmt-safetensors).
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

// -- Helpers -----------------------------------------------------------------

fn extension(path: &str) -> &str {
    path.rsplit('.')
        .next()
        .map(|e| e.split('?').next().unwrap_or(e))
        .unwrap_or("")
        .to_lowercase()
        .leak()
}

fn stub(path: &str, format: &str) -> String {
    debug!(
        path,
        format, "stub extractor — install format plugin for full text"
    );
    // Return the filename so at least keyword search by name works.
    path.rsplit('/').next().unwrap_or(path).to_string()
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
}
