//! Prometheus-style text metrics endpoint (T6.8).

use atlas_fs::Fs;

/// Render basic metrics as Prometheus text format.
pub fn render(fs: &Fs) -> String {
    let store_path = fs.store_path().display().to_string();
    format!(
        "# HELP atlas_store_info ATLAS store metadata\n\
         # TYPE atlas_store_info gauge\n\
         atlas_store_info{{path=\"{store_path}\"}} 1\n\
         \n\
         # HELP atlas_build_info ATLAS build information\n\
         # TYPE atlas_build_info gauge\n\
         atlas_build_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION"),
    )
}
