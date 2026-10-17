//! Content-addressed results: `<root>/<sha[0..2]>/<sha256>.<ext>`. Bytes land in
//! a resumable `.part` file first and are published by rename only after a
//! format check, so a truncated download can never be mistaken for a result.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    pub sha256: String,
    pub ext: String,
    pub bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The bytes are not a complete file of the expected kind.
    #[error("invalid result: {0}")]
    Invalid(String),
}

fn io(path: &Path, source: std::io::Error) -> ArtifactError {
    ArtifactError::Io {
        path: path.into(),
        source,
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path(&self, artifact: &ArtifactRef) -> PathBuf {
        self.root
            .join(&artifact.sha256[..2])
            .join(format!("{}.{}", artifact.sha256, artifact.ext))
    }

    /// The file exists with the recorded size.
    pub fn contains(&self, artifact: &ArtifactRef) -> bool {
        fs::metadata(self.path(artifact)).is_ok_and(|m| m.is_file() && m.len() == artifact.bytes)
    }

    /// A partial result named after its owner (a task key), kept across restarts
    /// so downloads resume with HTTP `Range` instead of starting over.
    pub fn part(&self, owner: &str) -> Result<Part, ArtifactError> {
        let directory = self.root.join(".parts");
        fs::create_dir_all(&directory).map_err(|e| io(&directory, e))?;
        Ok(Part {
            path: directory.join(format!("{owner}.part")),
        })
    }

    /// Publish a complete in-memory result.
    pub fn put(
        &self,
        owner: &str,
        bytes: &[u8],
        ext: &str,
        verify: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<ArtifactRef, ArtifactError> {
        let part = self.part(owner)?;
        part.reset()?;
        part.append()?
            .write_all(bytes)
            .map_err(|e| io(&part.path, e))?;
        self.publish(part, ext, verify)
    }

    /// Verify, hash and move a finished part into place. A failed check
    /// deletes the part so the next attempt starts clean.
    pub fn publish(
        &self,
        part: Part,
        ext: &str,
        verify: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<ArtifactRef, ArtifactError> {
        // FlushFileBuffers needs a writable handle on Windows.
        OpenOptions::new()
            .write(true)
            .open(&part.path)
            .and_then(|file| file.sync_all())
            .map_err(|e| io(&part.path, e))?;
        if let Err(reason) = verify(&part.path) {
            let _ = fs::remove_file(&part.path);
            return Err(ArtifactError::Invalid(reason));
        }
        let (sha256, bytes) = hash_file(&part.path)?;
        let artifact = ArtifactRef {
            sha256,
            ext: ext.to_owned(),
            bytes,
        };
        let destination = self.path(&artifact);
        if self.contains(&artifact) {
            let _ = fs::remove_file(&part.path);
            return Ok(artifact);
        }
        let directory = destination.parent().expect("artifact paths have a parent");
        fs::create_dir_all(directory).map_err(|e| io(directory, e))?;
        fs::rename(&part.path, &destination).map_err(|e| io(&destination, e))?;
        Ok(artifact)
    }
}

/// A resumable partial file.
#[derive(Debug)]
pub struct Part {
    path: PathBuf,
}

impl Part {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Bytes already received.
    pub fn len(&self) -> u64 {
        fs::metadata(&self.path).map_or(0, |m| m.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn append(&self) -> Result<File, ArtifactError> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| io(&self.path, e))
    }
    /// Drop received bytes (the server ignored `Range`, or a check failed).
    pub fn reset(&self) -> Result<(), ArtifactError> {
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(&self.path, e)),
            _ => Ok(()),
        }
    }
}

fn hash_file(path: &Path) -> Result<(String, u64), ArtifactError> {
    let mut file = File::open(path).map_err(|e| io(path, e))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|e| io(path, e))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        total += count as u64;
    }
    let digest = hash.finalize();
    Ok((digest.iter().map(|b| format!("{b:02x}")).collect(), total))
}

/// Complete PNG/JPEG/WebP that decodes to at least `min` pixels per side.
pub fn verify_image(path: &Path, min: (u32, u32)) -> Result<(u32, u32), String> {
    let image = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| format!("image does not decode: {e}"))?;
    let size = (image.width(), image.height());
    if size.0 < min.0 || size.1 < min.1 {
        return Err(format!(
            "image is {}x{}, expected at least {}x{}",
            size.0, size.1, min.0, min.1
        ));
    }
    Ok(size)
}

/// RIFF/WAVE with a data chunk whose declared size matches the file.
pub fn verify_wav(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut position = 12;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size =
            u32::from_le_bytes(bytes[position + 4..position + 8].try_into().unwrap()) as usize;
        if id == b"data" {
            let available = bytes.len() - (position + 8);
            if size == 0 || size > available {
                return Err(format!(
                    "wav data chunk declares {size} bytes, file has {available}"
                ));
            }
            return Ok(());
        }
        position += 8 + size + (size & 1);
    }
    Err("wav has no data chunk".into())
}
