//! ATLAS versioning — commits, branches, log, diff, checkout.
//!
//! Sits on top of [`atlas_fs::Fs`] and operates on whole-tree commits.
//! Every commit records the working-root tree at a point in time and
//! advances the current branch.
//!
//! Phase 1 scope: linear history, single parent commits, log, diff,
//! checkout, branch create/list/delete. Merge and rebase land in
//! a later milestone.

use atlas_core::{time::now_millis, Author, Error, Hash, ObjectKind, Result};
use atlas_fs::Fs;
use atlas_meta::MetaStore;
use atlas_object::{
    codec::seal, Branch, BranchProtection, Commit, DirectoryManifest, HeadState,
};
use std::collections::HashSet;

/// One change between two trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added { path: String, hash: Hash, kind: ObjectKind },
    Removed { path: String, hash: Hash, kind: ObjectKind },
    Modified { path: String, from: Hash, to: Hash, kind: ObjectKind },
}

/// Versioning operations on top of an [`Fs`].
pub struct Version<'a> {
    fs: &'a Fs,
}

impl<'a> Version<'a> {
    pub fn new(fs: &'a Fs) -> Self {
        Self { fs }
    }

    // -- Commit -------------------------------------------------------

    /// Snapshot the current working root and advance the current branch.
    ///
    /// Fails if HEAD is detached. (Use `branch_create` first.)
    pub fn commit(&self, author: Author, message: impl Into<String>) -> Result<Hash> {
        let head = self.head()?;
        let branch_name = match head {
            HeadState::Branch(b) => b,
            HeadState::DetachedCommit(_) => {
                return Err(Error::Invalid(
                    "HEAD is detached; create a branch first".into(),
                ));
            }
        };
        let mut branch = self
            .fs
            .meta()
            .get_branch(&branch_name)?
            .ok_or_else(|| Error::NotFound(format!("branch {branch_name}")))?;

        let tree_hash = self.fs.working_root()?;

        let mut commit = Commit {
            hash: Hash::ZERO,
            tree_hash,
            parents: vec![branch.head],
            author,
            timestamp: now_millis(),
            message: message.into(),
            signature: None,
        };
        let (commit_hash, _) = seal(&mut commit)?;
        self.fs.meta().put_commit(&commit)?;

        branch.head = commit_hash;
        self.fs.meta().put_branch(&branch)?;
        Ok(commit_hash)
    }

    // -- Branches -----------------------------------------------------

    /// Create a new branch at the given commit (defaults to current HEAD's commit).
    pub fn branch_create(&self, name: impl Into<String>, base: Option<Hash>) -> Result<Branch> {
        let name: String = name.into();
        if self.fs.meta().get_branch(&name)?.is_some() {
            return Err(Error::AlreadyExists(format!("branch {name}")));
        }
        let head_commit = match base {
            Some(h) => h,
            None => self.head_commit()?,
        };
        let b = Branch {
            name: name.clone(),
            head: head_commit,
            protection: BranchProtection::default(),
        };
        self.fs.meta().put_branch(&b)?;
        Ok(b)
    }

    pub fn branch_list(&self) -> Result<Vec<Branch>> {
        self.fs.meta().list_branches()
    }

    pub fn branch_delete(&self, name: &str) -> Result<()> {
        // Refuse to delete the branch HEAD currently points at — protects
        // the user from a foot-gun. Detach first if you really want to.
        if let HeadState::Branch(current) = self.head()? {
            if current == name {
                return Err(Error::Invalid(format!(
                    "cannot delete branch '{name}' while HEAD points at it"
                )));
            }
        }
        if self.fs.meta().get_branch(name)?.is_none() {
            return Err(Error::NotFound(format!("branch {name}")));
        }
        self.fs.meta().delete(&atlas_meta::keys::branch(name))
    }

    // -- HEAD ---------------------------------------------------------

    pub fn head(&self) -> Result<HeadState> {
        self.fs
            .meta()
            .get_head()?
            .ok_or_else(|| Error::NotFound("HEAD".into()))
    }

    /// Resolve HEAD to a commit hash regardless of attached/detached.
    pub fn head_commit(&self) -> Result<Hash> {
        match self.head()? {
            HeadState::Branch(name) => {
                let b = self
                    .fs
                    .meta()
                    .get_branch(&name)?
                    .ok_or_else(|| Error::NotFound(format!("branch {name}")))?;
                Ok(b.head)
            }
            HeadState::DetachedCommit(h) => Ok(h),
        }
    }

    // -- Checkout -----------------------------------------------------

    /// Switch HEAD to the named branch (working root becomes that branch's tree).
    pub fn checkout_branch(&self, name: &str) -> Result<()> {
        let b = self
            .fs
            .meta()
            .get_branch(name)?
            .ok_or_else(|| Error::NotFound(format!("branch {name}")))?;
        let commit = self
            .fs
            .meta()
            .get_commit(&b.head)?
            .ok_or_else(|| Error::NotFound(format!("commit {}", b.head.short())))?;
        self.fs.set_working_root(commit.tree_hash)?;
        self.fs.meta().put_head(&HeadState::Branch(name.to_string()))?;
        Ok(())
    }

    /// Detached checkout of a specific commit.
    pub fn checkout_commit(&self, commit: Hash) -> Result<()> {
        let c = self
            .fs
            .meta()
            .get_commit(&commit)?
            .ok_or_else(|| Error::NotFound(format!("commit {}", commit.short())))?;
        self.fs.set_working_root(c.tree_hash)?;
        self.fs.meta().put_head(&HeadState::DetachedCommit(commit))?;
        Ok(())
    }

    // -- Log / diff ---------------------------------------------------

    /// Walk back from `start` (or HEAD if None) and yield commits in
    /// child-before-parent order, up to `limit` entries.
    pub fn log(&self, start: Option<Hash>, limit: usize) -> Result<Vec<Commit>> {
        let mut out = Vec::new();
        let mut seen: HashSet<Hash> = HashSet::new();
        let mut cursor = match start {
            Some(h) => Some(h),
            None => Some(self.head_commit()?),
        };
        while let Some(h) = cursor {
            if out.len() >= limit {
                break;
            }
            if !seen.insert(h) {
                break;
            }
            let c = self
                .fs
                .meta()
                .get_commit(&h)?
                .ok_or_else(|| Error::NotFound(format!("commit {}", h.short())))?;
            cursor = c.parents.first().copied();
            out.push(c);
        }
        Ok(out)
    }

    /// Diff two trees. Returns adds/removes/modifies sorted by path.
    pub fn diff_trees(&self, from: Hash, to: Hash) -> Result<Vec<Change>> {
        let mut changes = Vec::new();
        self.diff_dirs("", from, to, &mut changes)?;
        changes.sort_by(|a, b| change_path(a).cmp(change_path(b)));
        Ok(changes)
    }

    /// Diff two commits.
    pub fn diff_commits(&self, from: Hash, to: Hash) -> Result<Vec<Change>> {
        let from_c = self
            .fs
            .meta()
            .get_commit(&from)?
            .ok_or_else(|| Error::NotFound(format!("commit {}", from.short())))?;
        let to_c = self
            .fs
            .meta()
            .get_commit(&to)?
            .ok_or_else(|| Error::NotFound(format!("commit {}", to.short())))?;
        self.diff_trees(from_c.tree_hash, to_c.tree_hash)
    }

    fn diff_dirs(
        &self,
        prefix: &str,
        from: Hash,
        to: Hash,
        out: &mut Vec<Change>,
    ) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let from_dir: DirectoryManifest = self
            .fs
            .meta()
            .get_dir_manifest(&from)?
            .ok_or_else(|| Error::NotFound(format!("dir {}", from.short())))?;
        let to_dir: DirectoryManifest = self
            .fs
            .meta()
            .get_dir_manifest(&to)?
            .ok_or_else(|| Error::NotFound(format!("dir {}", to.short())))?;

        let mut i = 0usize;
        let mut j = 0usize;
        while i < from_dir.entries.len() && j < to_dir.entries.len() {
            let a = &from_dir.entries[i];
            let b = &to_dir.entries[j];
            match a.name.cmp(&b.name) {
                std::cmp::Ordering::Equal => {
                    if a.object_hash != b.object_hash || a.kind != b.kind {
                        let path = join(prefix, &a.name);
                        if a.kind == ObjectKind::Dir && b.kind == ObjectKind::Dir {
                            self.diff_dirs(&path, a.object_hash, b.object_hash, out)?;
                        } else {
                            out.push(Change::Modified {
                                path,
                                from: a.object_hash,
                                to: b.object_hash,
                                kind: b.kind,
                            });
                        }
                    }
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => {
                    self.collect_subtree(prefix, a, /*added=*/ false, out)?;
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    self.collect_subtree(prefix, b, /*added=*/ true, out)?;
                    j += 1;
                }
            }
        }
        while i < from_dir.entries.len() {
            self.collect_subtree(prefix, &from_dir.entries[i], false, out)?;
            i += 1;
        }
        while j < to_dir.entries.len() {
            self.collect_subtree(prefix, &to_dir.entries[j], true, out)?;
            j += 1;
        }
        Ok(())
    }

    fn collect_subtree(
        &self,
        prefix: &str,
        entry: &atlas_object::DirEntry,
        added: bool,
        out: &mut Vec<Change>,
    ) -> Result<()> {
        let path = join(prefix, &entry.name);
        match entry.kind {
            ObjectKind::Dir => {
                let dir = self
                    .fs
                    .meta()
                    .get_dir_manifest(&entry.object_hash)?
                    .ok_or_else(|| {
                        Error::NotFound(format!("dir {}", entry.object_hash.short()))
                    })?;
                for child in &dir.entries {
                    self.collect_subtree(&path, child, added, out)?;
                }
            }
            _ => {
                if added {
                    out.push(Change::Added {
                        path,
                        hash: entry.object_hash,
                        kind: entry.kind,
                    });
                } else {
                    out.push(Change::Removed {
                        path,
                        hash: entry.object_hash,
                        kind: entry.kind,
                    });
                }
            }
        }
        Ok(())
    }
}

fn change_path(c: &Change) -> &str {
    match c {
        Change::Added { path, .. } => path,
        Change::Removed { path, .. } => path,
        Change::Modified { path, .. } => path,
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        format!("/{name}")
    } else {
        format!("{prefix}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Fs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        (dir, fs)
    }

    #[test]
    fn commit_advances_head() {
        let (_d, fs) = fixture();
        fs.write("/a", b"hi").unwrap();
        let v = Version::new(&fs);
        let before = v.head_commit().unwrap();
        let c = v.commit(Author::new("u", "u@x"), "first").unwrap();
        assert_ne!(c, before);
        assert_eq!(v.head_commit().unwrap(), c);
    }

    #[test]
    fn log_returns_history_youngest_first() {
        let (_d, fs) = fixture();
        let v = Version::new(&fs);
        fs.write("/a", b"1").unwrap();
        let c1 = v.commit(Author::new("u", "u@x"), "one").unwrap();
        fs.write("/a", b"2").unwrap();
        let c2 = v.commit(Author::new("u", "u@x"), "two").unwrap();
        let log = v.log(None, 10).unwrap();
        assert_eq!(log[0].hash, c2);
        assert_eq!(log[1].hash, c1);
    }

    #[test]
    fn branch_create_and_checkout() {
        let (_d, fs) = fixture();
        let v = Version::new(&fs);
        fs.write("/a", b"1").unwrap();
        let c1 = v.commit(Author::new("u", "u@x"), "one").unwrap();
        v.branch_create("feature", None).unwrap();

        // Modify on main.
        fs.write("/a", b"main-only").unwrap();
        let _c2 = v.commit(Author::new("u", "u@x"), "main edit").unwrap();

        // Switch to feature: should see /a == "1".
        v.checkout_branch("feature").unwrap();
        assert_eq!(fs.read("/a").unwrap(), b"1".to_vec());
        assert_eq!(v.head_commit().unwrap(), c1);
    }

    #[test]
    fn diff_picks_up_adds_removes_modifies() {
        let (_d, fs) = fixture();
        let v = Version::new(&fs);
        fs.write("/keep", b"k").unwrap();
        fs.write("/gone", b"g").unwrap();
        fs.write("/edit", b"v1").unwrap();
        let c1 = v.commit(Author::new("u", "u@x"), "one").unwrap();
        fs.delete("/gone").unwrap();
        fs.write("/edit", b"v2").unwrap();
        fs.write("/added", b"a").unwrap();
        let c2 = v.commit(Author::new("u", "u@x"), "two").unwrap();
        let changes = v.diff_commits(c1, c2).unwrap();
        let kinds: Vec<&'static str> = changes
            .iter()
            .map(|c| match c {
                Change::Added { .. } => "add",
                Change::Removed { .. } => "del",
                Change::Modified { .. } => "mod",
            })
            .collect();
        assert!(kinds.contains(&"add"));
        assert!(kinds.contains(&"del"));
        assert!(kinds.contains(&"mod"));
    }

    #[test]
    fn cannot_delete_current_branch() {
        let (_d, fs) = fixture();
        let v = Version::new(&fs);
        assert!(v.branch_delete("main").is_err());
    }
}
