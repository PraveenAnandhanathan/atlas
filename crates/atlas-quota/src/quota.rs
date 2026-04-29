//! Quota definitions and limits (T7.7).

use serde::{Deserialize, Serialize};

/// Storage quota for a single tenant or namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quota {
    pub tenant_id: String,
    /// Maximum total bytes of stored data (0 = unlimited).
    pub max_bytes: u64,
    /// Maximum number of objects (0 = unlimited).
    pub max_objects: u64,
    /// Maximum aggregate read bandwidth in bytes/s (0 = unlimited).
    pub max_read_bps: u64,
    /// Maximum aggregate write bandwidth in bytes/s (0 = unlimited).
    pub max_write_bps: u64,
    /// Maximum number of concurrent API requests (0 = unlimited).
    pub max_concurrent_requests: u32,
}

impl Quota {
    pub fn unlimited(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into(), max_bytes: 0, max_objects: 0, max_read_bps: 0, max_write_bps: 0, max_concurrent_requests: 0 }
    }

    pub fn is_unlimited(&self) -> bool {
        self.max_bytes == 0 && self.max_objects == 0 && self.max_read_bps == 0 && self.max_write_bps == 0
    }
}

/// Current usage snapshot for a tenant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub tenant_id: String,
    pub bytes_used: u64,
    pub objects_used: u64,
    pub read_bps: u64,
    pub write_bps: u64,
    pub concurrent_requests: u32,
}

impl Usage {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into(), ..Default::default() }
    }

    /// Returns `true` if usage fits within the given quota.
    pub fn within_quota(&self, q: &Quota) -> bool {
        (q.max_bytes == 0 || self.bytes_used <= q.max_bytes)
            && (q.max_objects == 0 || self.objects_used <= q.max_objects)
            && (q.max_read_bps == 0 || self.read_bps <= q.max_read_bps)
            && (q.max_write_bps == 0 || self.write_bps <= q.max_write_bps)
            && (q.max_concurrent_requests == 0 || self.concurrent_requests <= q.max_concurrent_requests)
    }

    /// Fraction of byte quota consumed (0.0–1.0; 0.0 if unlimited).
    pub fn bytes_fraction(&self, q: &Quota) -> f64 {
        if q.max_bytes == 0 { return 0.0; }
        self.bytes_used as f64 / q.max_bytes as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_quota_passes_when_under() {
        let q = Quota { tenant_id: "t1".into(), max_bytes: 1_000, max_objects: 100, max_read_bps: 0, max_write_bps: 0, max_concurrent_requests: 0 };
        let u = Usage { tenant_id: "t1".into(), bytes_used: 500, objects_used: 10, ..Default::default() };
        assert!(u.within_quota(&q));
    }

    #[test]
    fn within_quota_fails_when_over() {
        let q = Quota { tenant_id: "t1".into(), max_bytes: 1_000, max_objects: 100, max_read_bps: 0, max_write_bps: 0, max_concurrent_requests: 0 };
        let u = Usage { tenant_id: "t1".into(), bytes_used: 2_000, objects_used: 10, ..Default::default() };
        assert!(!u.within_quota(&q));
    }

    #[test]
    fn unlimited_quota_always_passes() {
        let q = Quota::unlimited("t1");
        let u = Usage { tenant_id: "t1".into(), bytes_used: u64::MAX, objects_used: u64::MAX, ..Default::default() };
        assert!(u.within_quota(&q));
    }
}
