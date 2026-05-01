//! Sample data seeding for a fresh ATLAS store (T6.7).
//!
//! Creates a small but realistic volume so the user has something to
//! browse, search, and inspect on first launch without having to import
//! their own data first.
//!
//! Seeded layout:
//! ```text
//! /
//! +-- README.md                - welcome text
//! +-- datasets/
//! |   +-- iris.parquet         - 150-row synthetic Parquet stub
//! |   +-- labels.jsonl         - 10 synthetic JSONL records
//! +-- models/
//!     +-- tiny.safetensors     - minimal SafeTensors header (2 tensors)
//! ```

use atlas_fs::Fs;
use std::path::Path;

/// Seed the sample dataset into an already-initialised store at `root`.
pub fn seed_sample_data(root: &Path) -> anyhow::Result<()> {
    let fs = Fs::open(root)?;
    write_readme(&fs)?;
    write_datasets(&fs)?;
    write_models(&fs)?;
    tracing::info!(store = %root.display(), "sample data seeded");
    Ok(())
}

/// Build a minimal but spec-compliant Parquet file with 0 rows and 0 columns.
///
/// The format is:
/// ```text
/// [4]  magic: "PAR1"
/// [N]  footer: Thrift Compact-encoded FileMetaData
/// [4]  footer_length: u32 LE
/// [4]  magic: "PAR1"
/// ```
///
/// FileMetaData encoded fields:
///   field 1 (version=2, i32), field 2 (schema=[root group], list),
///   field 3 (num_rows=0, i64), field 4 (row_groups=[], list)
fn minimal_parquet() -> Vec<u8> {
    // Thrift Compact Protocol encoding of FileMetaData.
    // Field headers: (delta << 4) | type_id
    // Type IDs: I32=5, I64=6, BINARY=8, LIST=9, STRUCT=12
    // ZigZag varint: zigzag(n) = (n << 1) ^ (n >> 63)  →  0→0, 2→4
    let footer: &[u8] = &[
        // field 1 (version, i32): delta=1, type=5 → 0x15; zigzag(2)=4 → 0x04
        0x15, 0x04,
        // field 2 (schema, list): delta=1, type=9 → 0x19
        0x19,
        // list header: 1 element of type STRUCT(12) → (1<<4)|12 = 0x1C
        0x1C,
        // SchemaElement { name="schema", num_children=0 }
        //   field 4 (name, binary): delta=4, type=8 → 0x48; length=6 → 0x06; "schema"
        0x48, 0x06, b's', b'c', b'h', b'e', b'm', b'a',
        //   field 5 (num_children, i32): delta=1, type=5 → 0x15; zigzag(0)=0 → 0x00
        0x15, 0x00,
        //   stop SchemaElement
        0x00,
        // field 3 (num_rows, i64): delta=1, type=6 → 0x16; zigzag(0)=0 → 0x00
        0x16, 0x00,
        // field 4 (row_groups, list): delta=1, type=9 → 0x19
        0x19,
        // list header: 0 elements of type STRUCT(12) → (0<<4)|12 = 0x0C
        0x0C,
        // stop FileMetaData
        0x00,
    ];
    let footer_len = footer.len() as u32;
    let mut out = Vec::with_capacity(4 + footer.len() + 4 + 4);
    out.extend_from_slice(b"PAR1");
    out.extend_from_slice(footer);
    out.extend_from_slice(&footer_len.to_le_bytes());
    out.extend_from_slice(b"PAR1");
    out
}

fn write_readme(fs: &Fs) -> anyhow::Result<()> {
    let content = b"\
# ATLAS Sample Volume\n\
\n\
Welcome to ATLAS!  This sample volume contains:\n\
\n\
- `datasets/iris.parquet` - a synthetic Parquet stub\n\
- `datasets/labels.jsonl` - 10 JSONL label records\n\
- `models/tiny.safetensors` - a minimal SafeTensors checkpoint\n\
\n\
Try `atlasctl find --query \"label\"` to run a semantic search.\n\
Open ATLAS Explorer to browse, view lineage, and inspect policies.\n";
    fs.write("/README.md", content)?;
    Ok(())
}

fn write_datasets(fs: &Fs) -> anyhow::Result<()> {
    fs.mkdir("/datasets")?;

    fs.write("/datasets/iris.parquet", &minimal_parquet())?;

    // 10 JSONL records.
    let jsonl: String = (0..10)
        .map(|i| {
            format!(
                r#"{{"id":{i},"label":"class_{cls}","sepal_length":{sl:.1},"petal_length":{pl:.1}}}"#,
                cls = i % 3,
                sl = 4.5 + i as f64 * 0.3,
                pl = 1.0 + i as f64 * 0.2,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs.write("/datasets/labels.jsonl", jsonl.as_bytes())?;

    Ok(())
}

fn write_models(fs: &Fs) -> anyhow::Result<()> {
    fs.mkdir("/models")?;

    // Minimal SafeTensors file:
    //   [8-byte LE header length] [header JSON] [data: all zeros]
    let header = serde_json::json!({
        "weight": { "dtype": "F32", "shape": [4, 4], "data_offsets": [0, 64] },
        "bias":   { "dtype": "F32", "shape": [4],    "data_offsets": [64, 80] }
    });
    let header_bytes = header.to_string().into_bytes();
    let header_len = header_bytes.len() as u64;
    let mut st: Vec<u8> = header_len.to_le_bytes().to_vec();
    st.extend(&header_bytes);
    st.extend(vec![0u8; 80]); // tensor data bytes

    fs.write("/models/tiny.safetensors", &st)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_store(dir: &Path) -> Fs {
        Fs::init(dir).unwrap()
    }

    #[test]
    fn seed_creates_expected_files() {
        let dir = TempDir::new().unwrap();
        init_store(dir.path());
        seed_sample_data(dir.path()).unwrap();

        let fs = Fs::open(dir.path()).unwrap();
        assert!(fs.stat("/README.md").is_ok());
        assert!(fs.stat("/datasets/iris.parquet").is_ok());
        assert!(fs.stat("/datasets/labels.jsonl").is_ok());
        assert!(fs.stat("/models/tiny.safetensors").is_ok());
    }

    #[test]
    fn parquet_file_is_structurally_valid() {
        let p = minimal_parquet();
        // Magic at start and end.
        assert_eq!(&p[..4], b"PAR1");
        assert_eq!(&p[p.len() - 4..], b"PAR1");
        // Footer length at bytes [-8..-4] matches actual footer size.
        let footer_len =
            u32::from_le_bytes(p[p.len() - 8..p.len() - 4].try_into().unwrap()) as usize;
        assert!(footer_len > 0, "footer must be non-empty");
        // Footer region is within the file.
        let footer_start = p.len() - 8 - footer_len;
        assert!(footer_start >= 4, "footer must not overlap the leading magic");
    }

    #[test]
    fn safetensors_header_parses() {
        let dir = TempDir::new().unwrap();
        init_store(dir.path());
        seed_sample_data(dir.path()).unwrap();

        let fs = Fs::open(dir.path()).unwrap();
        let bytes = fs.read("/models/tiny.safetensors").unwrap();
        assert!(bytes.len() >= 8);
        let header_len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
        let header_json = &bytes[8..8 + header_len];
        let v: serde_json::Value = serde_json::from_slice(header_json).unwrap();
        assert!(v.get("weight").is_some());
        assert!(v.get("bias").is_some());
    }
}
