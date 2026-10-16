//! Local project documents and recoverable filesystem persistence.

mod error;
mod filesystem;
mod project_document;
mod projects;

pub use error::StorageError;
pub use projects::ProjectStore;
