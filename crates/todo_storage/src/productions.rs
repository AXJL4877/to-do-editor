//! Atomic storage for `todo-production/2` documents and their worker files.
use crate::{
    filesystem::{atomic, paths},
    StorageError,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use todo_production::{Production, PRODUCTION_SPEC};
use uuid::Uuid;

static SAVE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionSummary {
    pub id: String,
    pub title: String,
    pub rev: u64,
}

#[derive(Debug, Clone)]
pub struct ProductionStore {
    repo_root: PathBuf,
}

impl ProductionStore {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    pub fn list(&self, project: &str) -> Result<Vec<ProductionSummary>, StorageError> {
        let root = self.productions_dir(project, false)?;
        let Some(root) = root else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in fs::read_dir(&root).map_err(|e| StorageError::io(&root, e))? {
            let entry = entry.map_err(|e| StorageError::io(&root, e))?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !matches!(paths::normalize_name(&id), Ok(normalized) if normalized == id)
                || id == ".trash"
            {
                continue;
            }
            let dir = entry.path();
            if !paths::checked_directory(&dir, &root)? {
                continue;
            }
            let file = dir.join("production.json");
            if !paths::checked_file(&file, &root)? {
                continue;
            }
            let doc = self.load_file(&file)?;
            out.push(ProductionSummary {
                id,
                title: doc.title,
                rev: doc.rev,
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub fn create(&self, project: &str, document: &Production) -> Result<(), StorageError> {
        let project_dir = self.project_dir(project)?;
        let id = paths::normalize_name(&document.id)?;
        let root = project_dir.join("productions");
        paths::ensure_directory(&root, &project_dir)?;
        let directory = root.join(&id);
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StorageError::AlreadyExists(id.clone())
            } else {
                StorageError::io(&directory, error)
            }
        })?;
        let result = (|| {
            paths::ensure_directory(&directory.join("work"), &root)?;
            self.write_document(&directory.join("production.json"), document, 0)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&directory);
        }
        result
    }

    pub fn load(&self, project: &str, id: &str) -> Result<Production, StorageError> {
        let path = self.production_dir(project, id)?.join("production.json");
        self.load_file(&path)
    }

    /// Save an existing production. The stored revision always increases.
    pub fn save(&self, project: &str, document: &Production) -> Result<(), StorageError> {
        let _guard = SAVE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let directory = self.production_dir(project, &document.id)?;
        let path = directory.join("production.json");
        let current = self.load_file(&path)?;
        let rev = document.rev.max(current.rev.saturating_add(1));
        self.write_document(&path, document, rev)
    }

    pub fn workdir(&self, project: &str, id: &str) -> Result<PathBuf, StorageError> {
        let production = self.production_dir(project, id)?;
        let work = production.join("work");
        paths::ensure_directory(&work, &production)?;
        Ok(work)
    }

    pub fn delete(&self, project: &str, id: &str) -> Result<(), StorageError> {
        let production = self.production_dir(project, id)?;
        let root = production
            .parent()
            .ok_or_else(|| StorageError::UnsafePath(production.clone()))?;
        let trash = root.join(".trash");
        paths::ensure_directory(&trash, root)?;
        let destination = trash.join(Uuid::new_v4().to_string());
        fs::rename(&production, &destination)
            .map_err(|error| StorageError::io(&production, error))?;
        atomic::sync_directory(&trash)?;
        atomic::sync_directory(root)
    }

    fn load_file(&self, path: &Path) -> Result<Production, StorageError> {
        let bytes = fs::read(path).map_err(|error| StorageError::io(path, error))?;
        let document: Production = serde_json::from_slice(&bytes)?;
        if document.spec != PRODUCTION_SPEC {
            return Err(StorageError::InvalidProject(format!(
                "unsupported production spec {}",
                document.spec
            )));
        }
        if document.id.trim().is_empty() {
            return Err(StorageError::InvalidProject(
                "invalid production identity".into(),
            ));
        }
        Ok(document)
    }

    fn write_document(
        &self,
        path: &Path,
        document: &Production,
        rev: u64,
    ) -> Result<(), StorageError> {
        if document.spec != PRODUCTION_SPEC {
            return Err(StorageError::InvalidProject(format!(
                "unsupported production spec {}",
                document.spec
            )));
        }
        let mut stored = document.clone();
        stored.rev = rev;
        let bytes = serde_json::to_vec_pretty(&stored)?;
        atomic::write(path, &bytes)
    }

    fn project_dir(&self, project: &str) -> Result<PathBuf, StorageError> {
        let project = paths::normalize_name(project)?;
        let data = paths::data_dir(&self.repo_root, false)?
            .ok_or_else(|| StorageError::NotFound(project.clone()))?;
        let directory = data.join(project);
        if !paths::checked_directory(&directory, &data)? {
            return Err(StorageError::NotFound(directory.display().to_string()));
        }
        Ok(directory)
    }

    fn productions_dir(
        &self,
        project: &str,
        create: bool,
    ) -> Result<Option<PathBuf>, StorageError> {
        let project_dir = self.project_dir(project)?;
        let root = project_dir.join("productions");
        if create {
            paths::ensure_directory(&root, &project_dir)?;
            Ok(Some(root))
        } else if paths::checked_directory(&root, &project_dir)? {
            Ok(Some(root))
        } else {
            Ok(None)
        }
    }

    fn production_dir(&self, project: &str, id: &str) -> Result<PathBuf, StorageError> {
        let id = paths::normalize_name(id)?;
        let root = self
            .productions_dir(project, false)?
            .ok_or_else(|| StorageError::NotFound(id.clone()))?;
        let directory = root.join(id.clone());
        if !paths::checked_directory(&directory, &root)?
            || !paths::checked_file(&directory.join("production.json"), &root)?
        {
            return Err(StorageError::NotFound(id));
        }
        Ok(directory)
    }
}
