//! Policy-aware ingest filter (T3.8).
//!
//! The full governor (T4.4) evaluates capability tokens and xattr ACLs.
//! Phase 3 ships the *hook point*: a `PolicyFilter` trait that callers
//! implement to gate what gets indexed.
//!
//! Two built-in implementations ship now:
//! - `AllowAll`  — permits every document (default).
//! - `XattrDenyList` — blocks documents whose xattrs match a deny set.

use atlas_indexer::Document;
use std::collections::HashMap;

/// Return `true` if the document is allowed to be indexed.
pub trait PolicyFilter: Send + Sync {
    fn allow(&self, doc: &Document) -> bool;
}

/// Permits every document. Use when governance is not yet enabled.
pub struct AllowAll;

impl PolicyFilter for AllowAll {
    fn allow(&self, _doc: &Document) -> bool {
        true
    }
}

/// Blocks documents whose xattrs contain any (key, value) in the deny map.
///
/// Example: `XattrDenyList::new([("classification", "top-secret")])` will
/// prevent top-secret files from being indexed.
pub struct XattrDenyList {
    deny: HashMap<String, String>,
}

impl XattrDenyList {
    pub fn new(pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        Self {
            deny: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

impl PolicyFilter for XattrDenyList {
    fn allow(&self, doc: &Document) -> bool {
        !self.deny.iter().any(|(k, v)| doc.xattrs.get(k) == Some(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::Hash;

    fn doc_with_xattr(key: &str, value: &str) -> Document {
        Document {
            file_hash: Hash::ZERO,
            path: "/test".into(),
            text: "test".into(),
            embedding: vec![],
            xattrs: [(key.into(), value.into())].into(),
            model_version: String::new(),
        }
    }

    #[test]
    fn allow_all_passes_everything() {
        let doc = doc_with_xattr("classification", "secret");
        assert!(AllowAll.allow(&doc));
    }

    #[test]
    fn deny_list_blocks_matching_xattr() {
        let filter = XattrDenyList::new([("classification", "secret")]);
        let blocked = doc_with_xattr("classification", "secret");
        let ok = doc_with_xattr("classification", "public");
        assert!(!filter.allow(&blocked));
        assert!(filter.allow(&ok));
    }
}
