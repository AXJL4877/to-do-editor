use crate::{MediaError, MediaService};
use std::path::{Path, PathBuf};
use todo_core::MediaItem;
use uuid::Uuid;

impl MediaService {
    /// Register files already inside the repository without copying them.
    pub fn adopt_files(&self, files: &[PathBuf]) -> Result<Vec<MediaItem>, MediaError> {
        let root = self
            .root
            .canonicalize()
            .map_err(|e| MediaError::io(&self.root, e))?;
        let mut items = Vec::with_capacity(files.len());
        for file in files {
            // Relative paths are repository-relative, whatever the process cwd.
            let file = if file.is_relative() {
                self.root.join(file)
            } else {
                file.clone()
            };
            let file = &file;
            let canonical = file.canonicalize().map_err(|e| MediaError::io(file, e))?;
            if !canonical.starts_with(&root) || !canonical.is_file() {
                return Err(MediaError::Invalid(format!(
                    "adopt path is outside repository: {}",
                    file.display()
                )));
            }
            let metadata = self.probe_path(&canonical)?;
            let relative = canonical
                .strip_prefix(&root)
                .map_err(|_| MediaError::Invalid("adopt path escaped repository".into()))?
                .to_string_lossy()
                .replace('\\', "/");
            let name = canonical
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            items.push(MediaItem {
                id: Uuid::new_v4().to_string(),
                name,
                path: Some(relative),
                duration: metadata.duration,
                has_audio: Some(metadata.has_audio),
                width: metadata.width,
                height: metadata.height,
                kind: Some(metadata.kind),
                alpha: metadata.alpha.map(|alpha| match alpha {
                    crate::AlphaPacking::Stacked => todo_core::AlphaPacking::Stacked,
                    crate::AlphaPacking::Reveal => todo_core::AlphaPacking::Reveal,
                }),
                reveal: metadata.reveal.clone(),
                folder_id: None,
            });
        }
        Ok(items)
    }
}

#[allow(dead_code)]
fn _inside(_: &Path) {}
