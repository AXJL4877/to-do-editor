//! Persist image_gen outputs under the extension package so unchanged
//! prompts/params reuse files on the next run.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use todo_production::tasks::fingerprint;

const ROOT: &str = "assets/node-cache";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    fingerprint: String,
    images: Vec<Value>,
}

pub fn image_gen_fingerprint(
    blocks: &[(String, String)],
    style: &str,
    size: &str,
    quality: Option<&str>,
    transparent: bool,
    variant: u32,
    model: Option<&str>,
    references: &[String],
    // Bumped by「清除缓存」so package cache and AI journal keys both miss.
    epoch: u32,
) -> String {
    fingerprint(&json!({
        "blocks": blocks.iter().map(|(id, text)| json!({"id": id, "text": text})).collect::<Vec<_>>(),
        "style": style,
        "size": size,
        "quality": quality,
        "transparent": transparent,
        "variant": variant,
        "model": model,
        "references": references,
        "epoch": epoch,
    }))
}

fn manifest_path(package_dir: &Path, node_id: &str) -> PathBuf {
    package_dir.join(ROOT).join(format!("{node_id}.json"))
}

fn epoch_path(package_dir: &Path, node_id: &str) -> PathBuf {
    package_dir.join(ROOT).join(format!("{node_id}.epoch"))
}

/// How many times the user has cleared this node's cache (0 = never).
pub fn epoch(package_dir: &Path, node_id: &str) -> u32 {
    let text = std::fs::read_to_string(epoch_path(package_dir, node_id)).unwrap_or_default();
    text.trim().parse().unwrap_or(0)
}

fn write_epoch(package_dir: &Path, node_id: &str, value: u32) -> Result<(), String> {
    let path = epoch_path(package_dir, node_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let partial = path.with_extension("epoch.part");
    std::fs::write(&partial, value.to_string().as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(())
}

fn cache_dir(package_dir: &Path, node_id: &str, fp: &str) -> PathBuf {
    let short: String = fp.chars().take(16).collect();
    package_dir.join(ROOT).join(node_id).join(short)
}

fn resolve_source(package_dir: &Path, workdir: Option<&Path>, rel: &str) -> Option<PathBuf> {
    if rel.contains("..") || rel.starts_with('/') || rel.contains('\\') {
        return None;
    }
    if let Some(workdir) = workdir {
        let in_work = workdir.join(rel);
        if in_work.is_file() {
            return Some(in_work);
        }
    }
    let in_pkg = package_dir.join(rel);
    in_pkg.is_file().then_some(in_pkg)
}

/// Load cache if fingerprint matches and every image file still exists.
/// When `workdir` is set, files are copied into `services/cache-…` for this run.
pub fn try_load(
    package_dir: &Path,
    workdir: Option<&Path>,
    node_id: &str,
    want_fp: &str,
) -> Option<Vec<Value>> {
    let text = std::fs::read_to_string(manifest_path(package_dir, node_id)).ok()?;
    let cache: CacheFile = serde_json::from_str(&text).ok()?;
    if cache.fingerprint != want_fp || cache.images.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(cache.images.len());
    for (i, asset) in cache.images.iter().enumerate() {
        let rel = asset.get("path").and_then(Value::as_str)?;
        let src = resolve_source(package_dir, None, rel)?;
        let mut staged = asset.clone();
        if let Some(workdir) = workdir {
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png");
            let dest_rel = format!("services/cache-{node_id}-{i}.{ext}");
            let dest = workdir.join(&dest_rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok()?;
            }
            std::fs::copy(&src, &dest).ok()?;
            staged["path"] = json!(dest_rel);
            if let Some(obj) = staged.as_object_mut() {
                obj.remove("file");
            }
        }
        out.push(staged);
    }
    Some(out)
}

/// Copy finished images into the package cache and write the manifest.
pub fn save(
    package_dir: &Path,
    workdir: Option<&Path>,
    node_id: &str,
    want_fp: &str,
    images: &[Value],
) -> Result<(), String> {
    if images.is_empty() {
        return Ok(());
    }
    let dir = cache_dir(package_dir, node_id, want_fp);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut stored = Vec::with_capacity(images.len());
    for (i, asset) in images.iter().enumerate() {
        let src_rel = asset
            .get("path")
            .or_else(|| asset.get("file"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("缓存第 {i} 张缺 path"))?;
        let src = resolve_source(package_dir, workdir, src_rel)
            .ok_or_else(|| format!("找不到生图文件：{src_rel}"))?;
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image.png");
        let dest = dir.join(format!("{i}-{name}"));
        std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
        let rel = dest
            .strip_prefix(package_dir)
            .map_err(|_| "缓存文件不在扩展包内".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let mut entry = asset.clone();
        entry["path"] = json!(rel);
        if let Some(obj) = entry.as_object_mut() {
            obj.remove("file");
        }
        stored.push(entry);
    }
    let manifest = CacheFile {
        fingerprint: want_fp.to_owned(),
        images: stored,
    };
    let path = manifest_path(package_dir, node_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    let partial = path.with_extension("json.part");
    std::fs::write(&partial, body.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove manifest + cached image files, and bump epoch so the next run
/// cannot reuse the AI journal entry for the same prompt either.
pub fn clear(package_dir: &Path, node_id: &str) -> Result<(), String> {
    let manifest = manifest_path(package_dir, node_id);
    if manifest.is_file() {
        std::fs::remove_file(&manifest).map_err(|e| format!("无法删除缓存清单：{e}"))?;
    }
    let dir = package_dir.join(ROOT).join(node_id);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("无法删除缓存目录：{e}"))?;
    }
    let next = epoch(package_dir, node_id).saturating_add(1);
    write_epoch(package_dir, node_id, next)?;
    Ok(())
}

pub fn has_cache(package_dir: &Path, node_id: &str) -> bool {
    manifest_path(package_dir, node_id).is_file()
}
