//! Resolve an installed extension into a sanitized worker command.
//!
//! This module only prepares a command. Starting and supervising the child is
//! the responsibility of the worker pool, which keeps process lifecycle out of
//! the registry and manifest validation layers.
use crate::Extension;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use todo_production::extension::manifest::{Runtime as ManifestRuntime, RuntimeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub app_root: PathBuf,
    pub python_override: Option<PathBuf>,
}

impl RuntimePaths {
    pub fn new(app_root: PathBuf) -> Self {
        Self {
            app_root,
            python_override: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("扩展没有工作程序")]
    WorkerMissing,
    #[error("工作程序入口不合法：{0}")]
    InvalidEntry(String),
    #[error("找不到 Python 运行时")]
    PythonMissing,
    #[error("原生工作程序必须是包内的 .exe：{0}")]
    NativeMissing(PathBuf),
}

/// Build the command used to run an extension worker.
///
/// The returned command inherits ordinary non-secret environment variables,
/// but all names containing credential-like words are removed before the
/// worker-specific variables are added.
pub fn worker_command(ext: &Extension, paths: &RuntimePaths) -> Result<Command, RuntimeError> {
    let worker = ext
        .manifest
        .worker
        .as_ref()
        .ok_or(RuntimeError::WorkerMissing)?;
    let entry = package_entry(&ext.dir, &worker.entry)?;
    let runtime = ext
        .manifest
        .requires
        .runtime
        .as_ref()
        .ok_or_else(|| RuntimeError::InvalidEntry("缺少 requires.runtime".into()))?;
    let mut command = match runtime {
        ManifestRuntime {
            kind: RuntimeKind::Python,
            ..
        } => {
            let python = python_executable(paths).ok_or(RuntimeError::PythonMissing)?;
            let mut command = Command::new(python);
            command.args(["-X", "utf8", "-B", "-s", "-u"]);
            let sdk = paths
                .app_root
                .join("extensions/sdk/python")
                .canonicalize()
                .unwrap_or_else(|_| paths.app_root.join("extensions/sdk/python"));
            let sdk = plain_windows_path(&sdk);
            let entry = plain_windows_path(&entry);
            command.args([
                "-c",
                "import os,runpy,sys; sys.path[:0]=[sys.argv[1],os.path.dirname(sys.argv[2])]; sys.argv[0]=sys.argv[2]; runpy.run_path(sys.argv[2],run_name='__main__')",
            ]);
            command.arg(sdk);
            command.arg(entry);
            command
        }
        ManifestRuntime {
            kind: RuntimeKind::Native,
            ..
        } => {
            if entry
                .extension()
                .and_then(OsStr::to_str)
                .map(|s| s.to_ascii_lowercase())
                != Some("exe".into())
            {
                return Err(RuntimeError::NativeMissing(entry));
            }
            Command::new(entry)
        }
    };
    sanitize_environment(&mut command, &paths.app_root, &ext.manifest.id);
    command.current_dir(&ext.dir);
    Ok(command)
}

fn package_entry(package: &Path, relative: &str) -> Result<PathBuf, RuntimeError> {
    if relative.trim().is_empty()
        || Path::new(relative).is_absolute()
        || relative.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(RuntimeError::InvalidEntry(relative.into()));
    }
    let path = package.join(relative);
    let canonical_package = package
        .canonicalize()
        .map_err(|_| RuntimeError::InvalidEntry(relative.into()))?;
    let canonical = path
        .canonicalize()
        .map_err(|_| RuntimeError::InvalidEntry(relative.into()))?;
    if !canonical.starts_with(&canonical_package) || !canonical.is_file() {
        return Err(RuntimeError::InvalidEntry(relative.into()));
    }
    Ok(canonical)
}

fn plain_windows_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map_or_else(|| path.to_owned(), PathBuf::from)
}

fn python_executable(paths: &RuntimePaths) -> Option<PathBuf> {
    if let Some(path) = paths.python_override.clone().filter(|p| p.is_file()) {
        return Some(path);
    }
    if let Some(path) = std::env::var_os("TODO_PYTHON")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(path);
    }
    let bundled = paths.app_root.join("dependencies/python/python.exe");
    if bundled.is_file() {
        return Some(bundled);
    }
    if cfg!(debug_assertions) {
        // Development fallback: a real interpreter on PATH. The Microsoft
        // Store alias in WindowsApps hangs when its stdin is a pipe.
        let name = if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        };
        return std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .filter(|dir| !dir.to_string_lossy().contains("WindowsApps"))
                .map(|dir| dir.join(name))
                .find(|candidate| candidate.is_file())
        });
    }
    None
}

fn sanitize_environment(command: &mut Command, app_root: &Path, extension_id: &str) {
    let sdk = app_root
        .join("extensions/sdk/python")
        .canonicalize()
        .unwrap_or_else(|_| app_root.join("extensions/sdk/python"));
    let sdk_text = sdk.to_string_lossy().replace(r"\\?\", "");
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if ["KEY", "TOKEN", "SECRET", "PASSWORD"]
            .iter()
            .any(|word| upper.contains(word))
        {
            command.env_remove(&name);
        }
    }
    command
        .env_remove("TODO_HUNIU_API_KEY")
        .env_remove("TODO_VOLC_API_KEY")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONSTARTUP")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONPATH", sdk_text)
        .env("TODO_EXTENSION_ID", extension_id)
        .env("TODO_HOST_VERSION", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Origin;
    use todo_production::extension::manifest::{Manifest, Requires, Runtime, RuntimeKind, Worker};

    fn extension(dir: &Path, runtime: RuntimeKind, entry: &str) -> Extension {
        Extension {
            manifest: Manifest {
                spec: "todo-extension/1".into(),
                id: "test.worker".into(),
                name: "测试".into(),
                version: "1.0.0".into(),
                description: String::new(),
                kind: todo_production::extension::manifest::Kind::Workflow,
                input: Default::default(),
                params: Vec::new(),
                quick: Vec::new(),
                requires: Requires {
                    services: Default::default(),
                    runtime: Some(Runtime {
                        kind: runtime,
                        version: String::new(),
                    }),
                },
                worker: Some(Worker {
                    entry: entry.into(),
                    ops: vec!["run".into()],
                    timeout_sec: 30,
                    max_processes: 1,
                    memory_mib: 128,
                }),
                narration: None,
                graph: None,
                budget: Default::default(),
                previews: Vec::new(),
            },
            dir: dir.into(),
            origin: Origin::User,
        }
    }

    #[test]
    fn python_command_strips_credentials_and_sets_worker_environment() {
        let root = test_root("runtime");
        std::fs::create_dir_all(root.join("extensions/sdk/python")).unwrap();
        std::fs::create_dir_all(root.join("pkg/worker")).unwrap();
        let entry = root.join("pkg/worker/main.py");
        std::fs::write(&entry, b"pass\n").unwrap();
        let python = root.join("python.exe");
        std::fs::write(&python, b"placeholder").unwrap();
        let ext = extension(&root.join("pkg"), RuntimeKind::Python, "worker/main.py");
        let mut paths = RuntimePaths::new(root.clone());
        paths.python_override = Some(python.clone());
        let command = worker_command(&ext, &paths).unwrap();
        assert_eq!(command.get_program(), python.as_os_str());
        assert!(command.get_args().any(|arg| arg == OsStr::new("-u")));
        assert_eq!(command.get_current_dir(), Some(ext.dir.as_path()));
        assert!(command
            .get_envs()
            .any(|(name, _)| name == OsStr::new("PYTHONPATH")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_entry_must_be_an_exe_inside_the_package() {
        let root = test_root("native");
        std::fs::create_dir_all(root.join("pkg/worker")).unwrap();
        std::fs::write(root.join("pkg/worker/main.bin"), b"x").unwrap();
        let ext = extension(&root.join("pkg"), RuntimeKind::Native, "worker/main.bin");
        assert!(matches!(
            worker_command(&ext, &RuntimePaths::new(root.clone())),
            Err(RuntimeError::NativeMissing(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn path_escape_is_rejected_before_command_creation() {
        let root = test_root("escape");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        let ext = extension(&root.join("pkg"), RuntimeKind::Python, "../secret.py");
        assert!(matches!(
            worker_command(&ext, &RuntimePaths::new(root.clone())),
            Err(RuntimeError::InvalidEntry(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/extension-runtime-tests")
            .join(format!("{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
