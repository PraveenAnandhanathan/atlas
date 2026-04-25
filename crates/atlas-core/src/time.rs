//! Time helpers. All ATLAS timestamps are unix millis in UTC.

/// Current wall-clock time, in milliseconds since the unix epoch.
///
/// If the system clock is earlier than the epoch (impossible on any real
/// system we support), returns 0.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
