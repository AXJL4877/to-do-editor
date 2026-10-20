//! Where the motion library lives on disk: `<repo>/motions/` for the user's
//! presets and `motions/` inside every installed extension.
use std::path::{Path, PathBuf};
use todo_motion::{MotionLibrary, MOTIONS_DIR};

/// Load the library; problems with single files come back as warnings.
pub fn load(repo: &Path) -> (MotionLibrary, Vec<String>) {
    let mut extensions = Vec::new();
    for base in ["extensions/builtin", "extensions/user"] {
        let Ok(entries) = std::fs::read_dir(repo.join(base)) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|dir| dir.join(MOTIONS_DIR).is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            extensions.push((extension_id(&dir), dir));
        }
    }
    MotionLibrary::load(&repo.join(MOTIONS_DIR), &extensions)
}

/// The manifest id, falling back to the folder name for a broken manifest
/// (the extension registry reports that problem; the motions still load).
fn extension_id(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("extension.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("id")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}
