use crate::{files::temporary_file, MediaError};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    fs,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

#[derive(Debug)]
pub(crate) struct MediaCache {
    directory: PathBuf,
    pending: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}
impl MediaCache {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            pending: Mutex::new(HashMap::new()),
        }
    }
    pub fn key(&self, full: &Path, kind: &str, parameter: u32) -> Result<String, MediaError> {
        let metadata = fs::metadata(full).map_err(|error| MediaError::io(full, error))?;
        let mut hash = DefaultHasher::new();
        (
            3,
            full,
            kind,
            parameter,
            metadata.len(),
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok()),
            metadata
                .created()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok()),
        )
            .hash(&mut hash);
        Ok(format!("{kind}-{:016x}", hash.finish()))
    }
    pub fn path(&self, key: &str, extension: &str) -> PathBuf {
        let kind = key.split('-').next().unwrap_or("other");
        self.directory.join(kind).join(format!("{key}.{extension}"))
    }
    pub fn with_key<T>(
        &self,
        key: &str,
        work: impl FnOnce() -> Result<T, MediaError>,
    ) -> Result<T, MediaError> {
        let gate = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(key.into())
            .or_default()
            .clone();
        let result = {
            let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
            work()
        };
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if Arc::strong_count(&gate) == 2 {
            pending.remove(key);
        }
        result
    }
    /// Cache failure does not invalidate a usable generated result.
    pub fn write(&self, path: &Path, bytes: &[u8]) {
        let save = || -> Result<(), MediaError> {
            let directory = path.parent().unwrap();
            fs::create_dir_all(directory).map_err(|error| MediaError::io(directory, error))?;
            let (pending, mut file) = temporary_file(directory, "cache")?;
            file.write_all(bytes)
                .map_err(|error| MediaError::io(pending.path(), error))?;
            drop(file);
            pending.publish(path)
        };
        let _ = save();
    }
}
