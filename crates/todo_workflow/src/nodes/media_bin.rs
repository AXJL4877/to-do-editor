//! Execute `media_bin`: local files → MediaBin + optional ScriptText.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use crate::workflow::graph::GraphNode;

pub(crate) fn step(
    node: &GraphNode,
    base_dir: &Path,
    logs: &mut Vec<String>,
) -> Result<BTreeMap<String, Value>, String> {
    let (files, text) = read_files(node, base_dir)?;
    logs.push(format!(
        "读素材 {} 个文件，抽出文本 {} 字",
        files.len(),
        text.chars().count()
    ));
    let mut out = BTreeMap::new();
    out.insert("files".into(), json!({ "files": files.clone() }));
    if !text.is_empty() {
        out.insert("text".into(), json!({ "text": text }));
    }
    let image_files: Vec<&Value> = files
        .iter()
        .filter(|f| f.get("kind").and_then(Value::as_str) == Some("image"))
        .collect();
    if let Some(first) = image_files.first() {
        if let Some(path) = first.get("path").cloned() {
            let mut asset = json!({ "path": path });
            if let Some(mime) = mime_for(first) {
                asset["mime"] = json!(mime);
            }
            out.insert("image".into(), asset);
        }
    }
    if !image_files.is_empty() {
        let images: Vec<Value> = image_files
            .iter()
            .filter_map(|f| {
                let path = f.get("path").cloned()?;
                let mut asset = json!({ "path": path });
                if let Some(mime) = mime_for(f) {
                    asset["mime"] = json!(mime);
                }
                Some(asset)
            })
            .collect();
        if !images.is_empty() {
            out.insert("images".into(), json!({ "images": images }));
        }
    }
    Ok(out)
}

fn mime_for(file: &Value) -> Option<&'static str> {
    let path = file.get("path").and_then(Value::as_str)?;
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn read_files(node: &GraphNode, base_dir: &Path) -> Result<(Vec<Value>, String), String> {
    let empty = Vec::new();
    let list = node
        .params
        .get("files")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut files = Vec::new();
    let mut texts = Vec::new();
    for entry in list {
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "media_bin.files[].path 须为字符串".to_string())?;
        if path.contains("..") || path.starts_with('/') || path.contains('\\') {
            return Err(format!("非法路径：{path}"));
        }
        let absolute = base_dir.join(path);
        if !absolute.is_file() {
            return Err(format!("找不到文件：{path}"));
        }
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let kind = if kind.is_empty() {
            kind_of(&absolute)
        } else {
            kind
        };
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                PathBuf::from(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_owned()
            });
        if matches!(kind.as_str(), "text" | "json") {
            if let Ok(raw) = std::fs::read_to_string(&absolute) {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    texts.push(trimmed.to_owned());
                }
            }
        }
        files.push(json!({
            "path": path,
            "kind": kind,
            "name": name,
        }));
    }
    Ok((files, texts.join("\n\n")))
}

pub(crate) fn kind_of(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "md" | "csv" | "srt" | "ass" => "text".into(),
        "json" => "json".into(),
        "mp3" | "wav" | "m4a" | "ogg" | "flac" | "aac" => "audio".into(),
        "mp4" | "mov" | "webm" | "mkv" | "avi" => "video".into(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" => "image".into(),
        _ => "other".into(),
    }
}
