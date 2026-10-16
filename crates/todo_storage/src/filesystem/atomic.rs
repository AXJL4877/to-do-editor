use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::StorageError;

/// Publish a fully flushed document by replacing its directory entry. Never
/// remove the destination first: an unsuccessful replacement keeps the old file.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path
        .parent()
        .ok_or_else(|| StorageError::UnsafePath(path.to_owned()))?;
    let staging_path = parent.join(format!(".project-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staging_path)
        .map_err(|error| StorageError::io(&staging_path, error))?;
    let staging = StagingFile(staging_path);
    file.write_all(bytes)
        .map_err(|error| StorageError::io(&staging.0, error))?;
    file.flush()
        .map_err(|error| StorageError::io(&staging.0, error))?;
    file.sync_all()
        .map_err(|error| StorageError::io(&staging.0, error))?;
    drop(file);
    replace(&staging.0, path).map_err(|error| StorageError::io(path, error))?;
    sync_directory(parent)?;
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| StorageError::io(path, error))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Both NUL-terminated buffers remain alive for the call. Staging is in the
    // destination directory, so this cannot become a cross-volume copy/delete.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

struct StagingFile(PathBuf);

impl Drop for StagingFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
