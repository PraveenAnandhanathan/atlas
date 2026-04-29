//! Shared application state managed by Tauri (T6.6).

use atlas_explorer_ipc::{BrowserResponse, LineageResponse, PolicyResponse, SearchResponse, VersionResponse};
use atlas_fs::Fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Tauri managed state — one open ATLAS store at a time.
#[derive(Default)]
pub struct AppState {
    pub fs: Arc<Mutex<Option<Fs>>>,
    pub store_path: Arc<Mutex<Option<PathBuf>>>,
}

impl AppState {
    pub fn with_fs<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Fs) -> Result<T, String>,
    {
        let guard = self.fs.lock().map_err(|e| e.to_string())?;
        match guard.as_ref() {
            Some(fs) => f(fs),
            None => Err("No ATLAS store is open. Use 'Open Store…' first.".into()),
        }
    }

    pub fn open_store(&self, path: PathBuf) -> Result<(), String> {
        let fs = Fs::open(&path).map_err(|e| e.to_string())?;
        *self.fs.lock().map_err(|e| e.to_string())? = Some(fs);
        *self.store_path.lock().map_err(|e| e.to_string())? = Some(path);
        Ok(())
    }
}
