//! Key encoding for the metadata KV (spec v0.1 §10).
//!
//! Keys are ASCII strings. Wherever a hash appears in a key it's in
//! lowercase hex (64 chars) — matching `Hash::to_hex()`.

use atlas_core::Hash;

pub fn object(h: &Hash) -> String {
    format!("obj:{}", h.to_hex())
}

pub fn object_prefix() -> &'static str {
    "obj:"
}

pub fn refkey(path: &str) -> String {
    format!("ref:{}", path)
}

pub fn refkey_prefix() -> &'static str {
    "ref:"
}

pub fn branch(name: &str) -> String {
    format!("branch:{}", name)
}

pub fn branch_prefix() -> &'static str {
    "branch:"
}

pub fn commit(h: &Hash) -> String {
    format!("commit:{}", h.to_hex())
}

pub fn commit_prefix() -> &'static str {
    "commit:"
}

pub fn head() -> &'static str {
    "head"
}

pub fn xattr(h: &Hash, key: &str) -> String {
    format!("xattr:{}:{}", h.to_hex(), key)
}

pub fn config() -> &'static str {
    "config"
}
