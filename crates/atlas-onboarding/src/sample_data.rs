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

    // Synthetic Parquet stub — just the PAR1 magic; not a real file.
    let mut parquet = b"PAR1".to_vec();
    parquet.extend(vec![0u8; 64]);
    parquet.extend(b"PAR1");
    fs.write("/datasets/iris.parquet", &parquet)?;

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
