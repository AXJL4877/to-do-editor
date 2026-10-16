//! Version dispatch and validation for the existing camelCase project document.

use todo_core::{validate_project, Project};

use crate::StorageError;

const CURRENT_VERSION: u64 = 1;

pub(crate) fn decode(bytes: &[u8]) -> Result<Project, StorageError> {
    let document: serde_json::Value = serde_json::from_slice(bytes)?;
    let version = document
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            StorageError::InvalidProject("version must be an unsigned integer".into())
        })?;
    if version != CURRENT_VERSION {
        return Err(StorageError::UnsupportedVersion(version));
    }
    // Version 1 needs no migration. Serde defaults fill only missing legacy fields;
    // reconstructing through create_project would change persisted optional values.
    let project: Project = serde_json::from_value(document)?;
    validate(&project)?;
    Ok(project)
}

pub(crate) fn encode(project: &Project) -> Result<Vec<u8>, StorageError> {
    validate(project)?;
    let mut bytes = serde_json::to_vec_pretty(project)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn validate(project: &Project) -> Result<(), StorageError> {
    if u64::from(project.version) != CURRENT_VERSION {
        return Err(StorageError::UnsupportedVersion(u64::from(project.version)));
    }
    validate_project(project).map_err(|error| StorageError::InvalidProject(error.to_string()))
}
