//! ATLAS FUSE adapter.
//!
//! - **Default build (any OS)**: exposes `mount()` returning
//!   `MountError::NotImplemented` so the workspace compiles everywhere.
//! - **`linux-fuse` feature on Linux**: pulls in `fuser` and provides a
//!   read-write filesystem implementation covering `getattr`, `readdir`,
//!   `lookup`, `open`, `read`, `write`, `create`, `mkdir`, `unlink`,
//!   `rmdir`, and `rename` — enough to use a mounted ATLAS store like a
//!   regular Linux directory.

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
        MountOption::RW,
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
    //! Read-write FUSE filesystem.
    //!
    //! Inode strategy: inode 1 is root; every other object gets a fresh u64.
    //! Write path: accumulate in a HashMap<ino, Vec<u8>>, flush on release.

    use atlas_core::ObjectKind;
    use atlas_fs::Fs;
    use fuser::{
        FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
        ReplyEmpty, ReplyEntry, ReplyWrite, Request,
    };
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::sync::Mutex;
    use std::time::{Duration, UNIX_EPOCH};

    const TTL: Duration = Duration::from_secs(1);
    const ROOT_INO: u64 = 1;

    pub struct AtlasFuse {
        fs: Fs,
        ino_to_path: Mutex<HashMap<u64, String>>,
        path_to_ino: Mutex<HashMap<String, u64>>,
        next_ino: Mutex<u64>,
        write_buf: Mutex<HashMap<u64, Vec<u8>>>,
    }

    impl AtlasFuse {
        pub fn new(fs: Fs) -> Self {
            let mut i2p = HashMap::new();
            let mut p2i = HashMap::new();
            i2p.insert(ROOT_INO, "/".to_string());
            p2i.insert("/".to_string(), ROOT_INO);
            Self {
                fs,
                ino_to_path: Mutex::new(i2p),
                path_to_ino: Mutex::new(p2i),
                next_ino: Mutex::new(ROOT_INO + 1),
                write_buf: Mutex::new(HashMap::new()),
            }
        }

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

        fn forget_path(&self, path: &str) {
            let mut p2i = self.path_to_ino.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ino) = p2i.remove(path) {
                self.ino_to_path.lock().unwrap_or_else(|e| e.into_inner()).remove(&ino);
            }
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
                atime: UNIX_EPOCH, mtime: UNIX_EPOCH, ctime: UNIX_EPOCH, crtime: UNIX_EPOCH,
                kind,
                perm: if matches!(kind, FileType::Directory) { 0o755 } else { 0o644 },
                nlink: 1,
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                rdev: 0, blksize: 4096, flags: 0,
            })
        }
    }

    fn join(parent: &str, name: &str) -> String {
        if parent == "/" { format!("/{name}") } else { format!("{parent}/{name}") }
    }

    impl Filesystem for AtlasFuse {
        fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
            let Some(parent_path) = self.path_of(parent) else { reply.error(libc::ENOENT); return; };
            let Some(name) = name.to_str() else { reply.error(libc::EINVAL); return; };
            let path = join(&parent_path, name);
            let Some(ino) = self.intern(&path) else { reply.error(libc::EIO); return; };
            match self.attr_for(ino, &path) {
                Some(attr) => reply.entry(&TTL, &attr, 0),
                None => reply.error(libc::ENOENT),
            }
        }

        fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
            let Some(path) = self.path_of(ino) else { reply.error(libc::ENOENT); return; };
            match self.attr_for(ino, &path) {
                Some(attr) => reply.attr(&TTL, &attr),
                None => reply.error(libc::ENOENT),
            }
        }

        fn open(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
            if self.path_of(ino).is_some() { reply.opened(0, 0); } else { reply.error(libc::ENOENT); }
        }

        fn read(
            &mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, size: u32,
            _flags: i32, _lock: Option<u64>, reply: ReplyData,
        ) {
            let Some(path) = self.path_of(ino) else { reply.error(libc::ENOENT); return; };
            let buffered = self.write_buf.lock().ok().and_then(|b| b.get(&ino).cloned());
            let bytes = if let Some(buf) = buffered { buf } else {
                match self.fs.read(&path) {
                    Ok(b) => b,
                    Err(_) => { reply.error(libc::ENOENT); return; }
                }
            };
            let start = (offset.max(0) as usize).min(bytes.len());
            let end = (start + size as usize).min(bytes.len());
            reply.data(&bytes[start..end]);
        }

        fn readdir(
            &mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory,
        ) {
            let Some(path) = self.path_of(ino) else { reply.error(libc::ENOENT); return; };
            let entries = match self.fs.list(&path) {
                Ok(v) => v,
                Err(_) => { reply.error(libc::ENOENT); return; }
            };
            let mut listing: Vec<(u64, FileType, String)> = Vec::with_capacity(entries.len() + 2);
            listing.push((ino, FileType::Directory, ".".into()));
            listing.push((ino, FileType::Directory, "..".into()));
            for e in entries {
                let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
                let child_ino = match self.intern(&e.path) {
                    Some(i) => i, None => { reply.error(libc::EIO); return; }
                };
                let kind = match e.kind { ObjectKind::Dir => FileType::Directory, _ => FileType::RegularFile };
                listing.push((child_ino, kind, name));
            }
            for (i, (cino, kind, name)) in listing.into_iter().enumerate().skip(offset as usize) {
                if reply.add(cino, (i + 1) as i64, kind, name) { break; }
            }
            reply.ok();
        }

        fn create(
            &mut self, _req: &Request<'_>, parent: u64, name: &OsStr,
            _mode: u32, _umask: u32, _flags: i32, reply: ReplyCreate,
        ) {
            let Some(parent_path) = self.path_of(parent) else { reply.error(libc::ENOENT); return; };
            let Some(name_str) = name.to_str() else { reply.error(libc::EINVAL); return; };
            let path = join(&parent_path, name_str);
            match self.fs.write(&path, &[]) {
                Ok(_) => {
                    let ino = match self.intern(&path) {
                        Some(i) => i, None => { reply.error(libc::EIO); return; }
                    };
                    if let Ok(mut buf) = self.write_buf.lock() { buf.insert(ino, Vec::new()); }
                    let attr = FileAttr {
                        ino, size: 0, blocks: 0,
                        atime: UNIX_EPOCH, mtime: UNIX_EPOCH, ctime: UNIX_EPOCH, crtime: UNIX_EPOCH,
                        kind: FileType::RegularFile, perm: 0o644, nlink: 1,
                        uid: unsafe { libc::getuid() }, gid: unsafe { libc::getgid() },
                        rdev: 0, blksize: 4096, flags: 0,
                    };
                    reply.created(&TTL, &attr, 0, 0, 0);
                }
                Err(_) => reply.error(libc::EIO),
            }
        }

        fn write(
            &mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, data: &[u8],
            _write_flags: u32, _flags: i32, _lock_owner: Option<u64>, reply: ReplyWrite,
        ) {
            let Some(path) = self.path_of(ino) else { reply.error(libc::ENOENT); return; };
            let off = offset.max(0) as usize;
            let mut bufs = match self.write_buf.lock() {
                Ok(b) => b, Err(_) => { reply.error(libc::EIO); return; }
            };
            let buf = bufs.entry(ino).or_insert_with(|| self.fs.read(&path).unwrap_or_default());
            let needed = off + data.len();
            if buf.len() < needed { buf.resize(needed, 0); }
            buf[off..off + data.len()].copy_from_slice(data);
            reply.written(data.len() as u32);
        }

        fn flush(&mut self, _req: &Request<'_>, ino: u64, _fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
            let Some(path) = self.path_of(ino) else { reply.ok(); return; };
            if let Some(data) = self.write_buf.lock().ok().and_then(|mut b| b.remove(&ino)) {
                match self.fs.write(&path, &data) {
                    Ok(_) => reply.ok(), Err(_) => reply.error(libc::EIO),
                }
            } else {
                reply.ok();
            }
        }

        fn release(
            &mut self, _req: &Request<'_>, ino: u64, _fh: u64, _flags: i32,
            _lock_owner: Option<u64>, _flush: bool, reply: ReplyEmpty,
        ) {
            let Some(path) = self.path_of(ino) else { reply.ok(); return; };
            if let Some(data) = self.write_buf.lock().ok().and_then(|mut b| b.remove(&ino)) {
                match self.fs.write(&path, &data) {
                    Ok(_) => reply.ok(), Err(_) => reply.error(libc::EIO),
                }
            } else {
                reply.ok();
            }
        }

        fn mkdir(
            &mut self, _req: &Request<'_>, parent: u64, name: &OsStr,
            _mode: u32, _umask: u32, reply: ReplyEntry,
        ) {
            let Some(parent_path) = self.path_of(parent) else { reply.error(libc::ENOENT); return; };
            let Some(name_str) = name.to_str() else { reply.error(libc::EINVAL); return; };
            let path = join(&parent_path, name_str);
            match self.fs.mkdir(&path) {
                Ok(_) => {
                    let ino = match self.intern(&path) {
                        Some(i) => i, None => { reply.error(libc::EIO); return; }
                    };
                    let attr = FileAttr {
                        ino, size: 0, blocks: 0,
                        atime: UNIX_EPOCH, mtime: UNIX_EPOCH, ctime: UNIX_EPOCH, crtime: UNIX_EPOCH,
                        kind: FileType::Directory, perm: 0o755, nlink: 2,
                        uid: unsafe { libc::getuid() }, gid: unsafe { libc::getgid() },
                        rdev: 0, blksize: 4096, flags: 0,
                    };
                    reply.entry(&TTL, &attr, 0);
                }
                Err(_) => reply.error(libc::EIO),
            }
        }

        fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            let Some(parent_path) = self.path_of(parent) else { reply.error(libc::ENOENT); return; };
            let Some(name_str) = name.to_str() else { reply.error(libc::EINVAL); return; };
            let path = join(&parent_path, name_str);
            match self.fs.delete(&path) {
                Ok(_) => { self.forget_path(&path); reply.ok(); }
                Err(atlas_core::Error::NotFound(_)) => reply.error(libc::ENOENT),
                Err(_) => reply.error(libc::EIO),
            }
        }

        fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            let Some(parent_path) = self.path_of(parent) else { reply.error(libc::ENOENT); return; };
            let Some(name_str) = name.to_str() else { reply.error(libc::EINVAL); return; };
            let path = join(&parent_path, name_str);
            match self.fs.delete(&path) {
                Ok(_) => { self.forget_path(&path); reply.ok(); }
                Err(atlas_core::Error::NotFound(_)) => reply.error(libc::ENOENT),
                Err(atlas_core::Error::Invalid(_)) => reply.error(libc::ENOTEMPTY),
                Err(_) => reply.error(libc::EIO),
            }
        }

        fn rename(
            &mut self, _req: &Request<'_>, parent: u64, name: &OsStr,
            new_parent: u64, new_name: &OsStr, _flags: u32, reply: ReplyEmpty,
        ) {
            let (Some(pp), Some(npp)) = (self.path_of(parent), self.path_of(new_parent)) else {
                reply.error(libc::ENOENT); return;
            };
            let (Some(ns), Some(nns)) = (name.to_str(), new_name.to_str()) else {
                reply.error(libc::EINVAL); return;
            };
            let from = join(&pp, ns);
            let to = join(&npp, nns);
            match self.fs.rename(&from, &to) {
                Ok(_) => {
                    let mut p2i = self.path_to_ino.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(ino) = p2i.remove(&from) {
                        p2i.insert(to.clone(), ino);
                        self.ino_to_path.lock().unwrap_or_else(|e| e.into_inner()).insert(ino, to);
                    }
                    reply.ok();
                }
                Err(atlas_core::Error::NotFound(_)) => reply.error(libc::ENOENT),
                Err(_) => reply.error(libc::EIO),
            }
        }

        fn setattr(
            &mut self, _req: &Request<'_>, ino: u64, _mode: Option<u32>, _uid: Option<u32>,
            _gid: Option<u32>, size: Option<u64>, _atime: Option<fuser::TimeOrNow>,
            _mtime: Option<fuser::TimeOrNow>, _ctime: Option<std::time::SystemTime>,
            _fh: Option<u64>, _crtime: Option<std::time::SystemTime>,
            _chgtime: Option<std::time::SystemTime>, _bkuptime: Option<std::time::SystemTime>,
            _flags: Option<u32>, reply: ReplyAttr,
        ) {
            let Some(path) = self.path_of(ino) else { reply.error(libc::ENOENT); return; };
            if let Some(new_size) = size {
                let mut bufs = match self.write_buf.lock() {
                    Ok(b) => b, Err(_) => { reply.error(libc::EIO); return; }
                };
                let buf = bufs.entry(ino).or_insert_with(|| self.fs.read(&path).unwrap_or_default());
                buf.resize(new_size as usize, 0);
                if new_size == 0 {
                    let data = buf.clone();
                    drop(bufs);
                    let _ = self.fs.write(&path, &data);
                }
            }
            match self.attr_for(ino, &path) {
                Some(attr) => reply.attr(&TTL, &attr),
                None => reply.error(libc::ENOENT),
            }
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
        #[cfg(not(all(feature = "linux-fuse", target_os = "linux")))]
        assert!(matches!(mount(fs, &mountpoint), Err(MountError::NotImplemented)));
        #[cfg(all(feature = "linux-fuse", target_os = "linux"))]
        let _ = (fs, mountpoint);
    }
}
