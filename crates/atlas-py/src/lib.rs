//! Python bindings for ATLAS via PyO3.
//!
//! Build as a Python wheel:
//!   maturin build -m crates/atlas-py/Cargo.toml --release
//!
//! Then `import atlas_py` exposes:
//!   Store(path)            — open or init an ATLAS store
//!   store.write(p, bytes)  — write a file
//!   store.read(p) -> bytes
//!   store.exists(p) -> bool
//!   store.delete(p)
//!   store.commit(author, email, message) -> str   # commit hash
//!   store.checkout_branch(name)
//!   store.create_branch(name)
//!   store.head() -> str    # commit hash hex
//!
//! The binding holds the `Fs` behind a `Mutex` because Python can call
//! into us from multiple threads, and the underlying types aren't
//! `Send`-safe across the GIL boundary without serialization.

use atlas_core::Author;
use atlas_fs::Fs;
use atlas_version::Version;
use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::path::PathBuf;
use std::sync::Mutex;

#[pyclass(name = "Store")]
struct PyStore {
    fs: Mutex<Fs>,
}

fn into_pyerr<E: std::fmt::Display>(e: E) -> PyErr {
    PyIOError::new_err(format!("{e}"))
}

#[pymethods]
impl PyStore {
    /// Open or initialise an ATLAS store at `path`.
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let p = PathBuf::from(path);
        let fs = if p.exists() && p.join("config").exists() {
            Fs::open(&p).map_err(into_pyerr)?
        } else {
            Fs::init(&p).map_err(into_pyerr)?
        };
        Ok(Self { fs: Mutex::new(fs) })
    }

    /// Write `data` to logical path `path` (must start with '/').
    fn write(&self, path: &str, data: &[u8]) -> PyResult<()> {
        let fs = self.fs.lock().unwrap();
        fs.write(path, data).map_err(into_pyerr)
    }

    /// Read the bytes at `path`.
    fn read<'py>(&self, py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyBytes>> {
        let fs = self.fs.lock().unwrap();
        let bytes = fs.read(path).map_err(into_pyerr)?;
        Ok(PyBytes::new_bound(py, &bytes))
    }

    fn exists(&self, path: &str) -> PyResult<bool> {
        let fs = self.fs.lock().unwrap();
        match fs.stat(path) {
            Ok(_) => Ok(true),
            Err(atlas_core::Error::NotFound(_)) => Ok(false),
            Err(e) => Err(into_pyerr(e)),
        }
    }

    fn delete(&self, path: &str) -> PyResult<()> {
        let fs = self.fs.lock().unwrap();
        fs.delete(path).map_err(into_pyerr)
    }

    /// Snapshot the working root and advance the current branch.
    fn commit(&self, author: &str, email: &str, message: &str) -> PyResult<String> {
        let fs = self.fs.lock().unwrap();
        let v = Version::new(&*fs);
        let h = v
            .commit(Author::new(author, email), message)
            .map_err(into_pyerr)?;
        Ok(h.to_hex())
    }

    fn create_branch(&self, name: &str) -> PyResult<()> {
        let fs = self.fs.lock().unwrap();
        let v = Version::new(&*fs);
        v.branch_create(name, None).map_err(into_pyerr)?;
        Ok(())
    }

    fn checkout_branch(&self, name: &str) -> PyResult<()> {
        let fs = self.fs.lock().unwrap();
        let v = Version::new(&*fs);
        v.checkout_branch(name).map_err(into_pyerr)
    }

    fn head(&self) -> PyResult<String> {
        let fs = self.fs.lock().unwrap();
        let v = Version::new(&*fs);
        Ok(v.head_commit().map_err(into_pyerr)?.to_hex())
    }

    fn list_branches(&self) -> PyResult<Vec<String>> {
        let fs = self.fs.lock().unwrap();
        let v = Version::new(&*fs);
        Ok(v.branch_list()
            .map_err(into_pyerr)?
            .into_iter()
            .map(|b| b.name)
            .collect())
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok("Store(...)".into())
    }
}

/// Hash a byte string with BLAKE3-256, return lowercase hex.
#[pyfunction]
fn blake3_hex(data: &[u8]) -> PyResult<String> {
    if data.is_empty() {
        return Err(PyValueError::new_err("input must be non-empty"));
    }
    Ok(atlas_core::Hash::of(data).to_hex())
}

#[pymodule]
fn atlas_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyStore>()?;
    m.add_function(wrap_pyfunction!(blake3_hex, m)?)?;
    Ok(())
}
