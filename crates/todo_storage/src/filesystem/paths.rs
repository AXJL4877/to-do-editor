use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::StorageError;

pub(crate) fn normalize_name(name: &str) -> Result<String, StorageError> {
    let mut normalized = name;
    // Repeated suffixes are collapsed too, making normalization idempotent.
    while normalized
        .get(normalized.len().saturating_sub(5)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".json"))
    {
        normalized = &normalized[..normalized.len() - 5];
    }
    let invalid = normalized.trim().is_empty()
        || matches!(normalized, "." | "..")
        || normalized.eq_ignore_ascii_case(".trash")
        || normalized.ends_with(['.', ' '])
        || normalized
            .chars()
            .any(|c| c <= '\u{1f}' || "<>:\"/\\|?*".contains(c))
        || normalized.encode_utf16().count() > 255
        || is_reserved_device(normalized);
    if invalid {
        return Err(StorageError::InvalidName(name.to_owned()));
    }
    Ok(normalized.to_owned())
}

fn is_reserved_device(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|tail| {
            matches!(
                tail,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

pub(crate) fn data_dir(repo_root: &Path, create: bool) -> Result<Option<PathBuf>, StorageError> {
    let repo_root = repo_root
        .canonicalize()
        .map_err(|error| StorageError::io(repo_root, error))?;
    let path = repo_root.join("data");
    if create {
        ensure_directory(&path, &repo_root)?;
        Ok(Some(path))
    } else if checked_directory(&path, &repo_root)? {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

pub(crate) fn ensure_directory(path: &Path, boundary: &Path) -> Result<(), StorageError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(StorageError::io(path, error)),
    }
    if !checked_directory(path, boundary)? {
        return Err(StorageError::UnsafePath(path.to_owned()));
    }
    Ok(())
}

pub(crate) fn checked_directory(path: &Path, boundary: &Path) -> Result<bool, StorageError> {
    checked_entry(path, boundary, true)
}

pub(crate) fn checked_file(path: &Path, boundary: &Path) -> Result<bool, StorageError> {
    checked_entry(path, boundary, false)
}

fn checked_entry(path: &Path, boundary: &Path, directory: bool) -> Result<bool, StorageError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(StorageError::io(path, error)),
    };
    if is_reparse_point(&metadata) {
        return Err(StorageError::UnsafePath(path.to_owned()));
    }
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Ok(false);
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| StorageError::io(path, error))?;
    if !canonical.starts_with(boundary) || canonical == boundary {
        return Err(StorageError::UnsafePath(path.to_owned()));
    }
    Ok(true)
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
