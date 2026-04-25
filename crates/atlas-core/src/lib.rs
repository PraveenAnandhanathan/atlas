//! ATLAS shared types.
//!
//! This crate is dependency-free of other ATLAS crates. Everything
//! above the substrate layer imports its core vocabulary (hashes,
//! object kinds, authors, errors) from here.

pub mod error;
pub mod hash;
pub mod kind;
pub mod time;

pub use error::{Error, Result};
pub use hash::Hash;
pub use kind::ObjectKind;
pub use time::now_millis;

use serde::{Deserialize, Serialize};

/// Author of a write, commit, or signature.
///
/// `agent_id` is populated when the write is attributed to an
/// autonomous agent rather than a human user.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
    pub agent_id: Option<String>,
}

impl Author {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            agent_id: None,
        }
    }

    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }
}

/// On-disk format version embedded in `StoreConfig`.
///
/// Minor bumps add Option<…> fields. Major bumps require a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatVersion {
    pub major: u16,
    pub minor: u16,
}

impl FormatVersion {
    pub const CURRENT: Self = Self { major: 0, minor: 1 };
}

impl core::fmt::Display for FormatVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "v{}.{}", self.major, self.minor)
    }
}
