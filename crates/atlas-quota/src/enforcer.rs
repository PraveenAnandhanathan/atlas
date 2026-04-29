//! Quota enforcement and noisy-neighbour controls (T7.7).

use crate::quota::{Quota, Usage};
use crate::tenant::TenantRegistry;

/// Decision returned by the enforcer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Throttle { reason: String },
    Deny    { reason: String },
}

impl Decision {
    pub fn is_allow(&self) -> bool { matches!(self, Decision::Allow) }
}

/// Quota enforcer — checks a proposed operation against current usage.
pub struct Enforcer {
    registry: TenantRegistry,
    /// If usage exceeds this fraction of the quota, throttle (don't deny yet).
    throttle_threshold: f64,
}

impl Enforcer {
    pub fn new(registry: TenantRegistry) -> Self {
        Self { registry, throttle_threshold: 0.9 }
    }

    pub fn with_throttle_threshold(mut self, t: f64) -> Self {
        self.throttle_threshold = t.clamp(0.0, 1.0);
        self
    }

    /// Check whether `tenant_id` may write `additional_bytes` of data.
    pub fn check_write(&self, tenant_id: &str, additional_bytes: u64) -> Decision {
        let Some(tenant) = self.registry.get(tenant_id) else {
            return Decision::Deny { reason: format!("unknown tenant {tenant_id}") };
        };
        let q = &tenant.quota;
        if q.is_unlimited() {
            return Decision::Allow;
        }
        let usage = self.registry.get_usage(tenant_id).unwrap_or_else(|| Usage::new(tenant_id));
        let projected = usage.bytes_used.saturating_add(additional_bytes);
        if q.max_bytes > 0 && projected > q.max_bytes {
            return Decision::Deny { reason: format!("byte quota exceeded: {projected} > {}", q.max_bytes) };
        }
        let fraction = usage.bytes_fraction(q);
        if fraction >= self.throttle_threshold {
            return Decision::Throttle { reason: format!("usage at {:.1}% of quota", fraction * 100.0) };
        }
        Decision::Allow
    }

    /// Check whether `tenant_id` may issue another concurrent request.
    pub fn check_concurrency(&self, tenant_id: &str) -> Decision {
        let Some(tenant) = self.registry.get(tenant_id) else {
            return Decision::Deny { reason: format!("unknown tenant {tenant_id}") };
        };
        let q = &tenant.quota;
        if q.max_concurrent_requests == 0 {
            return Decision::Allow;
        }
        let usage = self.registry.get_usage(tenant_id).unwrap_or_else(|| Usage::new(tenant_id));
        if usage.concurrent_requests >= q.max_concurrent_requests {
            Decision::Deny { reason: format!("concurrency limit {} reached", q.max_concurrent_requests) }
        } else {
            Decision::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::Quota;
    use crate::tenant::{Tenant, TenantRegistry};

    fn setup(max_bytes: u64, used: u64) -> (Enforcer, &'static str) {
        let reg = TenantRegistry::new();
        let q = Quota { tenant_id: "t1".into(), max_bytes, max_objects: 0, max_read_bps: 0, max_write_bps: 0, max_concurrent_requests: 0 };
        reg.register(Tenant::new("t1", "Test", q)).unwrap();
        let mut u = Usage::new("t1");
        u.bytes_used = used;
        reg.set_usage(u).unwrap();
        let enforcer = Enforcer::new(reg).with_throttle_threshold(0.9);
        (enforcer, "t1")
    }

    #[test]
    fn allows_when_under_quota() {
        let (e, tid) = setup(1_000, 100);
        assert_eq!(e.check_write(tid, 100), Decision::Allow);
    }

    #[test]
    fn throttles_near_limit() {
        let (e, tid) = setup(1_000, 920);
        assert!(matches!(e.check_write(tid, 10), Decision::Throttle { .. }));
    }

    #[test]
    fn denies_over_limit() {
        let (e, tid) = setup(1_000, 950);
        assert!(matches!(e.check_write(tid, 100), Decision::Deny { .. }));
    }

    #[test]
    fn unknown_tenant_denied() {
        let reg = TenantRegistry::new();
        let e = Enforcer::new(reg);
        assert!(matches!(e.check_write("ghost", 1), Decision::Deny { .. }));
    }
}
