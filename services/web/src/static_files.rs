//! Embedded static files for the admin console SPA (T6.8).

/// The index HTML page — embedded at compile time.
pub static INDEX_HTML: &str = include_str!("../static/index.html");
