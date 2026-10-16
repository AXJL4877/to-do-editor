use std::{io, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid project JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid project name: {0}")]
    InvalidName(String),
    #[error("project already exists: {0}")]
    AlreadyExists(String),
    #[error("project not found: {0}")]
    NotFound(String),
    #[error("unsupported project version: {0}")]
    UnsupportedVersion(u64),
    #[error("invalid project: {0}")]
    InvalidProject(String),
    #[error("unsafe project path: {0}")]
    UnsafePath(PathBuf),
}

impl StorageError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
