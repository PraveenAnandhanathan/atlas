//! ATLAS FUSE adapter.
//!
//! - **Default build (any OS)**: exposes `mount()` returning
//!   `MountError::NotImplemented` so the workspace compiles everywhere.
//! - **`linux-fuse` feature on Linux**: pulls in `fuser` and provides a
//!   read-only filesystem implementation covering `getattr`, `readdir`,
//!   `lookup`, `open`, and `read` — enough to `cat` files from a mounted
//!   ATLAS store.
//!
//! Read-only is deliberate for this milestone. Write paths through FUSE
//! interact with `Fs::write` and need careful inode lifetime / dirty
//! page handling that's outside the scope of T0.7's "make a mount
//! browseable" goal.

use atlas_fs::Fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum MountError {
    #[error("FUSE mount is only available with --features linux-fuse on Linux")]
    NotImplemented,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("atlas: {0}")]
    Atlas(#[from] atlas_core::Error),
}

#[cfg(all(feature = "linux-fuse", target_os = "linux"))]
pub fn mount(fs: Fs, mountpoint: impl AsRef<Path>) -> Result<(), MountError> {
    use fuser::MountOption;
    let opts = vec![
        MountOption::RO,
        MountOption::FSName("atlas".into()),
        MountOption::AutoUnmount,
        MountOption::AllowOther,
    ];
    let inner = imp::AtlasFuse::new(fs);
    fuser::mount2(inner, mountpoint.as_ref(), &opts)?;
    Ok(())
}

#[cfg(not(all(feature = "linux-fuse", target_os = "linux")))]
pub fn mount(_fs: Fs, _mountpoint: impl AsRef<Path>) -> Result<(), MountError> {
    Err(MountError::NotImplemented)
}

#[cfg(all(feature = "linux-fuse", target_os = "linux"))]
mod imp {
    //! Real FUSE filesystem. Read-only.
    //!
    //! Inode strategy: inode `1` is the root directory; every other
    //! object is allocated a fresh u64 on first lookup, paired with the
    //! ATLAS path it represents. We never reuse inodes (so the kernel
    //! cache stays consistent) but we do clear the table on unmount.

    use atlas_core::ObjectKind;
    use atlas_fs::Fs;
    use fuser::{
        FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
    };
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::sync::Mutex;
    use std::time::{Duration, UNIX_EPOCH};

    const TTL: Duration = Duration::from_secs(1);
    const ROOT_INO: u64 = 1;

    pub struct AtlasFuse {
        fs: Fs,
        // ino -> "/atlas/path"
        ino_to_path: Mutex<HashMap<u64, String>>,
        path_to_ino: Mutex<HashMap<String, u64>>,
        next_ino: Mutex<u64>,
    }

    impl AtlasFuse {
        pub fn new(fs: Fs) -> Self {
            let mut ino_to_path = HashMap::new();
            let mut path_to_ino = HashMap::new();
            ino_to_path.insert(ROOT_INO, "/".to_string());
            path_to_ino.insert("/".to_string(), ROOT_INO);
            Self {
                fs,
                ino_to_path: Mutex::new(ino_to_path),
                path_to_ino: Mutex::new(path_to_ino),
                next_ino: Mutex::new(ROOT_INO + 1),
            }
        }

        /// Intern `path` → inode, allocating a fresh inode if needed.
        /// Returns `None` if a mutex is poisoned (mount stays alive; caller
        /// replies `EIO` to the kernel).
        fn intern(&self, path: &str) -> Option<u64> {
            let mut p2i = self.path_to_ino.lock().ok()?;
            if let Some(&ino) = p2i.get(path) {
                return Some(ino);
            }
            let mut next = self.next_ino.lock().ok()?;
            let ino = *next;
            *next += 1;
            p2i.insert(path.to_string(), ino);
            self.ino_to_path.lock().ok()?.insert(ino, path.to_string());
            Some(ino)
        }

        fn path_of(&self, ino: u64) -> Option<String> {
            self.ino_to_path.lock().ok()?.get(&ino).cloned()
        }

        fn attr_for(&self, ino: u64, path: &str) -> Option<FileAttr> {
            let entry = self.fs.stat(path).ok()?;
            let (kind, size) = match entry.kind {
                ObjectKind::Dir => (FileType::Directory, 0),
                ObjectKind::File => (FileType::RegularFile, entry.size),
                _ => return None,
            };
            Some(FileAttr {
                ino,
                size,
                blocks: size.div_ceil(512),
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
                crtime: UNIX_EPOCH,
                kind,
                perm: if matches!(kind, FileType::Directory) {
                    0o555
                } else {
                    0o444
                },
                nlink: 1,
                uid: 0,
                gid: 0,
                rdev: 0,
                blksize: 4096,
                flags: 0,
            })
        }
    }

    fn join(parent: &str, name: &str) -> String {
        if parent == "/" {
            format!("/{name}")
        } else {
            format!("{parent}/{name}")
        }
    }

    impl Filesystem for AtlasFuse {
        fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
            let Some(parent_path) = self.path_of(parent) else {
                reply.error(libc::ENOENT);
                return;
            };
            let Some(name) = name.to_str() else {
                reply.error(libc::EINVAL);
                return;
            };
            let path = join(&parent_path, name);
            let Some(ino) = self.intern(&path) else {
                reply.error(libc::EIO);
                return;
            };
            match self.attr_for(ino, &path) {
                Some(attr) => reply.entry(&TTL, &attr, 0),
                None => reply.error(libc::ENOENT),
            }
        }

        fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
            let Some(path) = self.path_of(ino) else {
                reply.error(libc::ENOENT);
                return;
            };
            match self.attr_for(ino, &path) {
                Some(attr) => reply.attr(&TTL, &attr),
                None => reply.error(libc::ENOENT),
            }
        }

        fn read(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _fh: u64,
            offset: i64,
            size: u32,
            _flags: i32,
            _lock: Option<u64>,
            reply: ReplyData,
        ) {
            let Some(path) = self.path_of(ino) else {
                reply.error(libc::ENOENT);
                return;
            };
            match self.fs.read(&path) {
                Ok(bytes) => {
                    let start = offset.max(0) as usize;
                    let end = (start + size as usize).min(bytes.len());
                    if start >= bytes.len() {
                        reply.data(&[]);
                    } else {
                        reply.data(&bytes[start..end]);
                    }
                }
                Err(_) => reply.error(libc::ENOENT),
            }
        }

        fn readdir(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _fh: u64,
            offset: i64,
            mut reply: ReplyDirectory,
        ) {
            let Some(path) = self.path_of(ino) else {
                reply.error(libc::ENOENT);
                return;
            };
            let entries = match self.fs.list(&path) {
                Ok(v) => v,
                Err(_) => {
                    reply.error(libc::ENOENT);
                    return;
                }
            };

            let mut listing: Vec<(u64, FileType, String)> = Vec::with_capacity(entries.len() + 2);
            listing.push((ino, FileType::Directory, ".".into()));
            listing.push((ino, FileType::Directory, "..".into()));
            for e in entries {
                // `Entry::path` is the absolute logical path; for readdir we
                // need only the basename.
                let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
                let child_ino = match self.intern(&e.path) {
                    Some(i) => i,
                    None => { reply.error(libc::EIO); return; }
                };
                let kind = match e.kind {
                    ObjectKind::Dir => FileType::Directory,
                    _ => FileType::RegularFile,
                };
                listing.push((child_ino, kind, name));
            }
            for (i, (cino, kind, name)) in listing.into_iter().enumerate().skip(offset as usize) {
                if reply.add(cino, (i + 1) as i64, kind, name) {
                    break;
                }
            }
            reply.ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_not_implemented_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        let mountpoint = dir.path().join("mnt");
        std::fs::create_dir_all(&mountpoint).unwrap();
        // On non-Linux or without the feature the stub returns NotImplemented.
        // On Linux + feature, mount() blocks; we can't unit-test that here.
        #[cfg(not(all(feature = "linux-fuse", target_os = "linux")))]
        assert!(matches!(
            mount(fs, &mountpoint),
            Err(MountError::NotImplemented)
        ));
        #[cfg(all(feature = "linux-fuse", target_os = "linux"))]
        let _ = (fs, mountpoint);
    }
}
