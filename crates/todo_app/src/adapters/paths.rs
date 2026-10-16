use std::path::{Component, Path, PathBuf};

pub fn repo_root() -> PathBuf {
    if let Some(root) = env_dir("TODO_REPO_ROOT") {
        return root;
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            if directory.join("preview_host.exe").is_file() {
                return directory.to_path_buf();
            }
        }
    }
    // crates/todo_app -> repo root
    let compiled = normalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    if looks_like_repo(&compiled) {
        return compiled;
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            if looks_like_repo(ancestor) {
                return ancestor.to_path_buf();
            }
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or(compiled)
}

pub fn preview_host_exe(root: &Path) -> PathBuf {
    if let Ok(raw) = std::env::var("TODO_PREVIEW_HOST") {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return path;
        }
    }
    let packaged = root.join("preview_host.exe");
    if packaged.is_file() {
        return packaged;
    }
    root.join("native")
        .join("build")
        .join("bin")
        .join("preview_host.exe")
}

/// Project JSON stores paths relative to the repo root (`data/...`).
/// Absolute paths are left unchanged.
pub fn resolve_media_path(root: &Path, stored: &str) -> PathBuf {
    let path = PathBuf::from(stored);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn env_dir(key: &str) -> Option<PathBuf> {
    let raw = std::env::var(key).ok()?;
    let path = PathBuf::from(raw.trim());
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(normalize(path))
}

fn looks_like_repo(path: &Path) -> bool {
    path.join("Cargo.toml").is_file()
        && path
            .join("crates")
            .join("todo_app")
            .join("Cargo.toml")
            .is_file()
}

fn normalize(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_media_joins_repo_root() {
        let root = PathBuf::from(r"d:\Desktop\to_do");
        let got = resolve_media_path(&root, "data/untitled/media/a.mp4");
        assert_eq!(got, root.join("data/untitled/media/a.mp4"));
    }

    #[test]
    fn absolute_media_is_unchanged() {
        let root = PathBuf::from(r"d:\Desktop\to_do");
        let stored = r"d:\clips\a.mp4";
        assert_eq!(resolve_media_path(&root, stored), PathBuf::from(stored));
    }
}
