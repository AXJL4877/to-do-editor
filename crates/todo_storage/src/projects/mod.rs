mod lock;

use std::{fs, io, path::PathBuf};

use todo_core::{create_project, Project};
use uuid::Uuid;

use crate::{
    filesystem::{atomic, paths},
    project_document, StorageError,
};
pub use lock::ProjectLock;

#[derive(Debug, Clone)]
pub struct ProjectStore {
    repo_root: PathBuf,
}

impl ProjectStore {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    /// List addressable projects without opening or changing their documents.
    pub fn list(&self) -> Result<Vec<String>, StorageError> {
        let Some(data) = paths::data_dir(&self.repo_root, false)? else {
            return Ok(Vec::new());
        };
        let mut names = Vec::new();
        for entry in fs::read_dir(&data).map_err(|error| StorageError::io(&data, error))? {
            let entry = entry.map_err(|error| StorageError::io(&data, error))?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !matches!(paths::normalize_name(&name), Ok(normalized) if normalized == name) {
                continue;
            }
            let directory = entry.path();
            match paths::checked_directory(&directory, &data) {
                Ok(true) => {}
                Ok(false) | Err(StorageError::UnsafePath(_)) => continue,
                Err(error) => return Err(error),
            }
            match paths::checked_file(&directory.join("project.json"), &data) {
                Ok(true) => names.push(name),
                Ok(false) | Err(StorageError::UnsafePath(_)) => {}
                Err(error) => return Err(error),
            }
        }
        names.sort();
        Ok(names)
    }

    /// Create a fresh project; an existing directory is never overwritten.
    pub fn create(&self, name: &str) -> Result<Project, StorageError> {
        let name = paths::normalize_name(name)?;
        let project = create_project(None, None, None, None, None);
        let bytes = project_document::encode(&project)?;
        self.create_document(&name, &bytes)?;
        Ok(project)
    }

    pub fn load(&self, name: &str) -> Result<Project, StorageError> {
        let name = paths::normalize_name(name)?;
        let path = self.existing_project(&name)?.join("project.json");
        let bytes = fs::read(&path).map_err(|error| StorageError::io(&path, error))?;
        project_document::decode(&bytes)
    }

    /// Return media storage only for an existing, fenced project.
    pub fn media_directory(&self, name: &str) -> Result<PathBuf, StorageError> {
        let name = paths::normalize_name(name)?;
        let project = self.existing_project(&name)?;
        let media = project.join("media");
        paths::ensure_directory(&media, &project)?;
        Ok(media)
    }

    /// Take the project for this process: no other process may save or
    /// delete it until the lock drops. Fails with `Busy` while another
    /// process holds it.
    pub fn lock(&self, name: &str) -> Result<ProjectLock, StorageError> {
        let name = paths::normalize_name(name)?;
        let data = paths::data_dir(&self.repo_root, true)?
            .ok_or_else(|| StorageError::UnsafePath(self.repo_root.join("data")))?;
        lock::acquire(&data, &name)
    }

    /// Save an existing project. A late save cannot recreate a deleted project,
    /// and a project another process holds is never written.
    pub fn save(&self, name: &str, project: &Project) -> Result<(), StorageError> {
        let name = paths::normalize_name(name)?;
        let bytes = project_document::encode(project)?;
        let directory = self.existing_project(&name)?;
        self.writable(&name)?;
        atomic::write(&directory.join("project.json"), &bytes)
    }

    /// Retain the complete directory, including its media, under data/.trash.
    pub fn delete(&self, name: &str) -> Result<(), StorageError> {
        let name = paths::normalize_name(name)?;
        let project = self.existing_project(&name)?;
        self.writable(&name)?;
        let data = project
            .parent()
            .ok_or_else(|| StorageError::UnsafePath(project.clone()))?;
        let trash = data.join(".trash");
        paths::ensure_directory(&trash, data)?;
        // Keep the archive component bounded even for a 255-character name.
        let destination = trash.join(Uuid::new_v4().to_string());
        fs::rename(&project, &destination).map_err(|error| StorageError::io(&project, error))?;
        atomic::sync_directory(&trash)?;
        atomic::sync_directory(data)
    }

    /// Validate an imported version-1 document before creating any directory.
    /// Existing project names are rejected, exactly as with create.
    pub fn import_json(&self, name: &str, bytes: &[u8]) -> Result<Project, StorageError> {
        let name = paths::normalize_name(name)?;
        let project = project_document::decode(bytes)?;
        let bytes = project_document::encode(&project)?;
        self.create_document(&name, &bytes)?;
        Ok(project)
    }

    fn writable(&self, name: &str) -> Result<(), StorageError> {
        let Some(data) = paths::data_dir(&self.repo_root, false)? else {
            return Ok(());
        };
        match lock::holder(&data, name)? {
            lock::Holder::Other(who) => Err(StorageError::Busy(name.to_owned(), who)),
            lock::Holder::Nobody | lock::Holder::ThisProcess => Ok(()),
        }
    }

    fn existing_project(&self, name: &str) -> Result<PathBuf, StorageError> {
        let data = paths::data_dir(&self.repo_root, false)?
            .ok_or_else(|| StorageError::NotFound(name.to_owned()))?;
        let directory = data.join(name);
        if !paths::checked_directory(&directory, &data)?
            || !paths::checked_file(&directory.join("project.json"), &data)?
        {
            return Err(StorageError::NotFound(name.to_owned()));
        }
        Ok(directory)
    }

    fn create_document(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let data = paths::data_dir(&self.repo_root, true)?
            .ok_or_else(|| StorageError::UnsafePath(self.repo_root.join("data")))?;
        let directory = data.join(name);
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                StorageError::AlreadyExists(name.to_owned())
            } else {
                StorageError::io(&directory, error)
            }
        })?;
        let result = (|| {
            paths::ensure_directory(&directory.join("media"), &data)?;
            atomic::write(&directory.join("project.json"), bytes)
        })();
        if result.is_err() {
            // Remove only directories we just made, and only while empty. Never
            // recursively erase anything another operation might have written.
            let _ = fs::remove_dir(directory.join("media"));
            let _ = fs::remove_dir(&directory);
        }
        result
    }
}
