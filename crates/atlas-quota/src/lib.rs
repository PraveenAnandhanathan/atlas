//! ATLAS multi-tenant quotas, isolation, and noisy-neighbour controls (T7.7).
//!
//! - [`quota`]: quota definitions and per-tenant usage tracking.
//! - [`tenant`]: tenant registry backed by an `Arc<Mutex<HashMap>>`.
//! - [`enforcer`]: enforcement engine — allow / throttle / deny decisions.

pub mod enforcer;
pub mod quota;
pub mod tenant;

pub use enforcer::{Decision, Enforcer};
pub use quota::{Quota, Usage};
pub use tenant::{Tenant, TenantRegistry};
