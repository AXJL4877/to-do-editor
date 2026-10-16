mod process;
#[cfg(test)]
mod tests;
pub(crate) use process::{capture, execute};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MediaTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
impl MediaTools {
    pub fn discover(root: &Path) -> Self {
        Self {
            ffmpeg: locate(root, "ffmpeg", "TODO_FFMPEG"),
            ffprobe: locate(root, "ffprobe", "TODO_FFPROBE"),
        }
    }
}
fn locate(root: &Path, name: &str, variable: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
        return path.into();
    }
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let bundled = root.join("dependencies/ffmpeg/bin").join(&filename);
    if bundled.is_file() {
        return bundled;
    }
    if let Some(found) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(&filename))
            .find(|path| path.is_file())
    }) {
        return found;
    }
    bundled
}
