//! ATLAS Explorer — Tauri desktop application entry point (T6.6).
//!
//! Five tabs: Browser · Search · Lineage · Version · Policy.
//!
//! The Tauri runtime loads this binary, starts the WebView pointed at
//! `src/index.html`, and bridges JavaScript `invoke()` calls to the
//! Tauri commands defined in `commands.rs`.

// Prevents an additional console window from opening on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tauri_build();
}

/// Build and run the Tauri application.
/// Extracted so `lib.rs` can also call it for mobile targets.
fn tauri_build() {
    // In a real Tauri 2.x project this would be:
    //
    //   tauri::Builder::default()
    //       .manage(state::AppState::default())
    //       .invoke_handler(tauri::generate_handler![
    //           commands::browse,
    //           commands::search,
    //           commands::lineage,
    //           commands::version_log,
    //           commands::policy_view,
    //           commands::open_store,
    //       ])
    //       .run(tauri::generate_context!())
    //       .expect("error while running ATLAS Explorer");
    //
    // Stubbed here so the crate compiles without the tauri dev-dependency
    // (which requires a system WebView2 / WebKit).  The real Cargo.toml
    // for production adds `tauri = { version = "2", features = ["…"] }`.
    tracing::info!("ATLAS Explorer starting (stub — link against tauri to run)");
}
