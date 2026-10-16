mod temporary;
use crate::MediaError;
use std::{
    fs,
    path::{Path, PathBuf},
};
pub(crate) use temporary::temporary_file;

/// Absolute legacy references are readable; relative references stay in the workspace.
pub(crate) fn resolve(root: &Path, stored: &Path) -> Result<PathBuf, MediaError> {
    let root = root
        .canonicalize()
        .map_err(|error| MediaError::io(root, error))?;
    let joined = if stored.is_absolute() {
        stored.into()
    } else {
        root.join(stored)
    };
    let full = joined
        .canonicalize()
        .map_err(|error| MediaError::io(&joined, error))?;
    if !stored.is_absolute() && !full.starts_with(&root) {
        return Err(MediaError::Invalid(
            "relative path escapes the workspace".into(),
        ));
    }
    if !fs::metadata(&full)
        .map_err(|error| MediaError::io(&full, error))?
        .is_file()
    {
        return Err(MediaError::Invalid("media must be a regular file".into()));
    }
    Ok(full)
}
