//! Installed extensions: built-in (`extensions/builtin`, read-only, shipped)
//! and user (`extensions/user`). A user extension with the same id replaces
//! the built-in one. Invalid packages are listed with their problems and are
//! never runnable.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use todo_production::extension::manifest::{Manifest, Problem};

pub const MANIFEST_FILE: &str = "extension.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    User,
}

#[derive(Debug, Clone)]
pub struct Extension {
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub origin: Origin,
}

impl Extension {
    /// Package text file by relative path (prompts, templates).
    pub fn read(&self, relative: &str) -> Option<String> {
        if !todo_production::extension::manifest::valid_relative_path(relative) {
            return None;
        }
        std::fs::read_to_string(self.dir.join(relative)).ok()
    }

    /// `id@version`: processes are reused only for the same package build.
    pub fn build_key(&self) -> String {
        format!("{}@{}", self.manifest.id, self.manifest.version)
    }
}

#[derive(Debug, Clone)]
pub struct Rejected {
    pub dir: PathBuf,
    pub origin: Origin,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    extensions: BTreeMap<String, Arc<Extension>>,
    pub rejected: Vec<Rejected>,
}

/// Parse and validate one package directory.
pub fn load(dir: &Path, origin: Origin) -> Result<Extension, Vec<Problem>> {
    let problem = |message: String| {
        vec![Problem {
            path: MANIFEST_FILE.into(),
            message,
        }]
    };
    let text =
        std::fs::read_to_string(dir.join(MANIFEST_FILE)).map_err(|e| problem(e.to_string()))?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| problem(e.to_string()))?;
    let extension = Extension {
        manifest,
        dir: dir.to_path_buf(),
        origin,
    };
    let mut problems = extension
        .manifest
        .validate(&|relative| extension.read(relative));
    if let Some(worker) = &extension.manifest.worker {
        if !dir.join(&worker.entry).is_file() {
            problems.push(Problem {
                path: "worker.entry".into(),
                message: format!("包内没有 {}", worker.entry),
            });
        }
    }
    if problems.is_empty() {
        Ok(extension)
    } else {
        Err(problems)
    }
}

impl Registry {
    pub fn scan(builtin: &Path, user: &Path) -> Self {
        let mut registry = Self::default();
        for (root, origin) in [(builtin, Origin::Builtin), (user, Origin::User)] {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue;
            };
            let mut dirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && !p
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .collect();
            dirs.sort();
            for dir in dirs {
                if !dir.join(MANIFEST_FILE).is_file() {
                    continue;
                }
                match load(&dir, origin) {
                    Ok(extension) => {
                        // User packages are scanned last and replace built-in ones.
                        registry
                            .extensions
                            .insert(extension.manifest.id.clone(), Arc::new(extension));
                    }
                    Err(problems) => registry.rejected.push(Rejected {
                        dir,
                        origin,
                        problems,
                    }),
                }
            }
        }
        registry
    }

    pub fn get(&self, id: &str) -> Option<Arc<Extension>> {
        self.extensions.get(id).cloned()
    }

    pub fn list(&self) -> impl Iterator<Item = &Arc<Extension>> {
        self.extensions.values()
    }
}
