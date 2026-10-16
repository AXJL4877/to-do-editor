use crate::MediaError;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub(crate) struct PendingFile {
    path: PathBuf,
}
impl PendingFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn publish(self, destination: &Path) -> Result<(), MediaError> {
        fs::rename(&self.path, destination).map_err(|error| MediaError::io(destination, error))
    }
}
impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
pub(crate) fn temporary_file(
    directory: &Path,
    extension: &str,
) -> Result<(PendingFile, File), MediaError> {
    let path = directory.join(format!(".{}.tmp.{extension}", Uuid::new_v4()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| MediaError::io(&path, error))?;
    Ok((PendingFile { path }, file))
}
