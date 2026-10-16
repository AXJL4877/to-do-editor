use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media job cancelled")]
    Cancelled,
    #[error("file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("project: {0}")]
    Project(#[from] todo_storage::StorageError),
    #[error("invalid media: {0}")]
    Invalid(String),
    #[error("media tool {tool}: {message}")]
    Tool { tool: PathBuf, message: String },
    #[error("media tool timed out: {0}")]
    Timeout(PathBuf),
    #[error("text: {0}")]
    Text(#[from] todo_text::TextError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
impl MediaError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
