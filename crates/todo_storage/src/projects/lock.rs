//! One writer per project across processes. The editor and command-line
//! tools share `data/`: without this, a second process could resume the same
//! productions, re-register their files as media and save over the first one.
//!
//! The lock is an OS file lock on `data/.locks/<hash>.lock` (outside the
//! project directory, so deleting a project can still move it), released when
//! the guard drops or the process dies. The holder writes its process id to
//! `<hash>.owner`, so the same process may keep saving while it holds the lock.
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{filesystem::paths, StorageError};

/// Exclusive use of one project by this process.
#[derive(Debug)]
pub struct ProjectLock {
    name: String,
    owner: PathBuf,
    _file: File,
}

impl ProjectLock {
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.owner);
    }
}

/// Who may write a project now.
pub(crate) enum Holder {
    Nobody,
    ThisProcess,
    Other(String),
}

fn files(data: &Path, name: &str) -> (PathBuf, PathBuf) {
    // FNV-1a: project names may be 255 characters, file names may not.
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let dir = data.join(".locks");
    (dir.join(format!("{hash:016x}.lock")), dir.join(format!("{hash:016x}.owner")))
}

fn open(lock: &Path) -> Result<File, StorageError> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock)
        .map_err(|error| StorageError::io(lock, error))
}

pub(crate) fn acquire(data: &Path, name: &str) -> Result<ProjectLock, StorageError> {
    paths::ensure_directory(&data.join(".locks"), data)?;
    let (lock, owner) = files(data, name);
    let file = open(&lock)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(StorageError::Busy(name.to_owned(), describe(&owner)))
        }
        Err(TryLockError::Error(error)) => return Err(StorageError::io(&lock, error)),
    }
    let mut stamp = File::create(&owner).map_err(|error| StorageError::io(&owner, error))?;
    let _ = writeln!(stamp, "{}", std::process::id());
    let _ = writeln!(stamp, "{}", std::env::args().next().unwrap_or_default());
    Ok(ProjectLock {
        name: name.to_owned(),
        owner,
        _file: file,
    })
}

pub(crate) fn holder(data: &Path, name: &str) -> Result<Holder, StorageError> {
    let (lock, owner) = files(data, name);
    if !lock.is_file() {
        return Ok(Holder::Nobody);
    }
    let file = open(&lock)?;
    match file.try_lock() {
        // Free (the probe's own lock goes away with `file`).
        Ok(()) => Ok(Holder::Nobody),
        Err(TryLockError::WouldBlock) => {
            let text = fs::read_to_string(&owner).unwrap_or_default();
            if text.lines().next().and_then(|pid| pid.trim().parse::<u32>().ok())
                == Some(std::process::id())
            {
                Ok(Holder::ThisProcess)
            } else {
                Ok(Holder::Other(describe(&owner)))
            }
        }
        Err(TryLockError::Error(error)) => Err(StorageError::io(&lock, error)),
    }
}

fn describe(owner: &Path) -> String {
    let text = fs::read_to_string(owner).unwrap_or_default();
    let mut lines = text.lines();
    let pid = lines.next().unwrap_or("?").trim().to_owned();
    let program = lines
        .next()
        .map(|path| {
            Path::new(path.trim())
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    if program.is_empty() {
        format!("进程 {pid}")
    } else {
        format!("{program}（进程 {pid}）")
    }
}
