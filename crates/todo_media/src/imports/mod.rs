use crate::{files::temporary_file, MediaError, MediaService};
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};
use todo_core::MediaItem;
use todo_storage::ProjectStore;
use uuid::Uuid;

impl MediaService {
    /// Stream-copy and probe privately; publish only to an existing project.
    pub fn import_file(&self, project: &str, source: &Path) -> Result<MediaItem, MediaError> {
        let mut input = File::open(source).map_err(|error| MediaError::io(source, error))?;
        let info = input
            .metadata()
            .map_err(|error| MediaError::io(source, error))?;
        if !info.is_file() || info.len() == 0 {
            return Err(MediaError::Invalid(
                "source must be a nonempty regular file".into(),
            ));
        }
        let root = self
            .root
            .canonicalize()
            .map_err(|error| MediaError::io(&self.root, error))?;
        let directory = ProjectStore::new(&root).media_directory(project)?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 16
                    && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .unwrap_or("bin")
            .to_ascii_lowercase();
        let (pending, mut output) = temporary_file(&directory, &extension)?;
        let copied =
            io::copy(&mut input, &mut output).map_err(|error| MediaError::io(source, error))?;
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|error| MediaError::io(pending.path(), error))?;
        drop(output);
        if copied == 0 {
            return Err(MediaError::Invalid(
                "source became empty during import".into(),
            ));
        }
        let metadata = self.probe_path(pending.path())?;
        let id = Uuid::new_v4().to_string();
        let destination = directory.join(format!("{id}.{extension}"));
        pending.publish(&destination)?;
        let stored = destination
            .strip_prefix(&root)
            .map_err(|_| MediaError::Invalid("media destination escaped workspace".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        Ok(MediaItem {
            id,
            name: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: Some(stored),
            duration: metadata.duration,
            has_audio: Some(metadata.has_audio),
            width: metadata.width,
            height: metadata.height,
            kind: Some(metadata.kind),
        })
    }
}
