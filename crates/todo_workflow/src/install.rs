//! `.todoext` packages: zip archives of an extension directory. Installing
//! checks every entry and the manifest from memory before anything is written,
//! extracts into a temporary directory and swaps it in. Nothing in the package
//! runs during installation.
use crate::registry::{load, Extension, Origin, MANIFEST_FILE};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use crate::workflow::manifest::{valid_relative_path, Manifest, Problem};

pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 4000;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("扩展包无法读取：{0}")]
    Archive(String),
    #[error("扩展包内容不合法：{0}")]
    Unsafe(String),
    #[error("扩展清单有 {} 处问题：{}", .0.len(), .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("；"))]
    Invalid(Vec<Problem>),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn io(path: &Path, source: std::io::Error) -> InstallError {
    InstallError::Io {
        path: path.into(),
        source,
    }
}

/// Read and check every entry of an archive into memory.
fn read_archive(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, InstallError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| InstallError::Archive(e.to_string()))?;
    if archive.len() > MAX_FILES {
        return Err(InstallError::Unsafe(format!("文件数超过 {MAX_FILES}")));
    }
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| InstallError::Archive(e.to_string()))?;
        let name = entry
            .name()
            .map_err(|e| InstallError::Archive(e.to_string()))?
            .into_owned();
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() || !valid_relative_path(&name) {
            return Err(InstallError::Unsafe(format!("路径 {name:?} 不允许")));
        }
        if entry.size() > MAX_FILE_BYTES {
            return Err(InstallError::Unsafe(format!("{name} 超过 256 MiB")));
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        // Never trust the declared size: cap what is actually inflated.
        (&mut entry)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| InstallError::Archive(e.to_string()))?;
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(InstallError::Unsafe(format!("{name} 超过 256 MiB")));
        }
        total += data.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(InstallError::Unsafe("解压后超过 1 GiB".into()));
        }
        if files.insert(name.clone(), data).is_some() {
            return Err(InstallError::Unsafe(format!("{name} 重复")));
        }
    }
    Ok(files)
}

/// Validate a `.todoext` in memory without writing (Agent dry-run).
pub fn validate_package(archive: &[u8]) -> Result<Manifest, InstallError> {
    let files = read_archive(archive)?;
    let manifest_bytes = files
        .get(MANIFEST_FILE)
        .ok_or_else(|| InstallError::Unsafe("缺少 extension.json".into()))?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes).map_err(|e| {
        InstallError::Invalid(vec![Problem {
            path: MANIFEST_FILE.into(),
            message: e.to_string(),
        }])
    })?;
    let read = |path: &str| {
        files
            .get(path)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    };
    let mut problems = manifest.validate(&read);
    if let Some(worker) = &manifest.worker {
        if !files.contains_key(&worker.entry) {
            problems.push(Problem {
                path: "worker.entry".into(),
                message: format!("包内没有 {}", worker.entry),
            });
        }
    }
    if !problems.is_empty() {
        return Err(InstallError::Invalid(problems));
    }
    Ok(manifest)
}

/// Install a `.todoext` into `user_dir/<id>`, replacing an older version.
pub fn install(archive: &[u8], user_dir: &Path) -> Result<Extension, InstallError> {
    let files = read_archive(archive)?;
    let manifest = validate_package(archive)?;
    std::fs::create_dir_all(user_dir).map_err(|e| io(user_dir, e))?;
    let staging = user_dir.join(format!(".install-{}", uuid::Uuid::new_v4()));
    let write = || -> Result<(), InstallError> {
        for (name, data) in &files {
            let path = staging.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| io(&path, e))?;
            let mut file = std::fs::File::create(&path).map_err(|e| io(&path, e))?;
            file.write_all(data).map_err(|e| io(&path, e))?;
            file.sync_all().map_err(|e| io(&path, e))?;
        }
        Ok(())
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let target = user_dir.join(&manifest.id);
    let retired = user_dir.join(format!(".old-{}", uuid::Uuid::new_v4()));
    if target.exists() {
        std::fs::rename(&target, &retired).map_err(|e| io(&target, e))?;
    }
    if let Err(e) = std::fs::rename(&staging, &target) {
        // Put the previous version back.
        let _ = std::fs::rename(&retired, &target);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(&target, e));
    }
    let _ = std::fs::remove_dir_all(&retired);
    load(&target, Origin::User).map_err(InstallError::Invalid)
}

/// Remove a user extension (built-in ones cannot be removed).
pub fn uninstall(user_dir: &Path, id: &str) -> Result<bool, InstallError> {
    if !crate::workflow::manifest::valid_id(id) {
        return Ok(false);
    }
    let target = user_dir.join(id);
    if !target.join(MANIFEST_FILE).is_file() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&target).map_err(|e| io(&target, e))?;
    Ok(true)
}

/// Zip an extension directory (for authors); hidden files and `__pycache__` are skipped.
pub fn pack(dir: &Path) -> Result<Vec<u8>, InstallError> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for relative in files {
        let data = std::fs::read(dir.join(&relative)).map_err(|e| io(&dir.join(&relative), e))?;
        out.start_file(relative.as_str(), options)
            .map_err(|e| InstallError::Archive(e.to_string()))?;
        out.write_all(&data).map_err(|e| io(dir, e))?;
    }
    let cursor = out
        .finish()
        .map_err(|e| InstallError::Archive(e.to_string()))?;
    Ok(cursor.into_inner())
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), InstallError> {
    for entry in std::fs::read_dir(dir).map_err(|e| io(dir, e))?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "__pycache__" {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push(relative);
        }
    }
    Ok(())
}
