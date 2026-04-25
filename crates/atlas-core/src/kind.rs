//! Object kind enum (spec v0.1 §6).

use serde::{Deserialize, Serialize};

/// Kind of object a directory entry points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ObjectKind {
    File = 1,
    Dir = 2,
    Symlink = 3,
    /// Reserved: a ref-spec entry that points at a branch/ref rather than
    /// a concrete object. Unused in v0.1.
    Refspec = 4,
}

impl ObjectKind {
    pub fn is_dir(self) -> bool {
        matches!(self, Self::Dir)
    }

    pub fn is_file(self) -> bool {
        matches!(self, Self::File)
    }
}

impl core::fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Symlink => "symlink",
            Self::Refspec => "refspec",
        };
        f.write_str(s)
    }
}
