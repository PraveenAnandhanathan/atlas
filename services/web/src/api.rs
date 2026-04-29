//! `/api/*` route handler — proxies to the REST capability core (T6.8).

use crate::server::AppState;
use atlas_rest::{handle_request, RestRequest};
use serde_json::json;

pub async fn handle(path: &str, state: &AppState) -> (u16, String, &'static str) {
    // Strip /api prefix and forward to the REST adapter.
    let atlas_path = path.strip_prefix("/api").unwrap_or(path);
    let req = RestRequest {
        method: "GET".into(),
        path: atlas_path.to_string(),
        principal: Some("web-console".into()),
        body: json!({}),
    };
    let resp = handle_request(&state.core, &req);
    let status = if resp.status == 200 { 200u16 } else { resp.status as u16 };
    (status, resp.body.to_string(), "application/json")
}
