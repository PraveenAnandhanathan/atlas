//! Tenant registry (T7.7).

use crate::quota::{Quota, Usage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub display_name: String,
    pub quota: Quota,
}

impl Tenant {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>, quota: Quota) -> Self {
        Self { id: id.into(), display_name: display_name.into(), quota }
    }
}

/// Thread-safe in-memory tenant registry.
#[derive(Default, Clone)]
pub struct TenantRegistry {
    tenants: Arc<Mutex<HashMap<String, Tenant>>>,
    usage: Arc<Mutex<HashMap<String, Usage>>>,
}

impl TenantRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, tenant: Tenant) -> Result<(), String> {
        let mut map = self.tenants.lock().map_err(|e| e.to_string())?;
        if map.contains_key(&tenant.id) {
            return Err(format!("tenant {} already registered", tenant.id));
        }
        let id = tenant.id.clone();
        map.insert(id.clone(), tenant);
        drop(map);
        self.usage.lock().map_err(|e| e.to_string())?.insert(id.clone(), Usage::new(id));
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Tenant> {
        self.tenants.lock().ok()?.get(id).cloned()
    }

    pub fn set_usage(&self, usage: Usage) -> Result<(), String> {
        let mut map = self.usage.lock().map_err(|e| e.to_string())?;
        map.insert(usage.tenant_id.clone(), usage);
        Ok(())
    }

    pub fn get_usage(&self, id: &str) -> Option<Usage> {
        self.usage.lock().ok()?.get(id).cloned()
    }

    pub fn list(&self) -> Vec<Tenant> {
        self.tenants.lock().map(|m| m.values().cloned().collect()).unwrap_or_default()
    }

    pub fn remove(&self, id: &str) -> bool {
        let removed = self.tenants.lock().ok().map(|mut m| m.remove(id).is_some()).unwrap_or(false);
        if removed {
            let _ = self.usage.lock().map(|mut m| m.remove(id));
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(id: &str) -> Tenant {
        Tenant::new(id, id, Quota::unlimited(id))
    }

    #[test]
    fn register_and_get() {
        let reg = TenantRegistry::new();
        reg.register(tenant("acme")).unwrap();
        assert_eq!(reg.get("acme").unwrap().id, "acme");
    }

    #[test]
    fn duplicate_register_errors() {
        let reg = TenantRegistry::new();
        reg.register(tenant("acme")).unwrap();
        assert!(reg.register(tenant("acme")).is_err());
    }

    #[test]
    fn usage_starts_at_zero() {
        let reg = TenantRegistry::new();
        reg.register(tenant("beta")).unwrap();
        let u = reg.get_usage("beta").unwrap();
        assert_eq!(u.bytes_used, 0);
    }
}
