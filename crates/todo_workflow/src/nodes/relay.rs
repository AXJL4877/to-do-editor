//! Execute `relay`: file mid-store + text-block mid-store (from 口播分段).

use super::media_bin::kind_of;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use crate::workflow::graph::GraphNode;

pub(crate) fn step(
    node: &GraphNode,
    inputs: &BTreeMap<String, Value>,
    base_dir: &Path,
    logs: &mut Vec<String>,
) -> Result<BTreeMap<String, Value>, String> {
    let mut files: Vec<Value> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push_file = |entry: Value| {
        let Some(path) = entry.get("path").and_then(Value::as_str).map(str::to_owned) else {
            return;
        };
        if path.is_empty() || !seen.insert(path) {
            return;
        }
        files.push(entry);
    };

    // Start from previously stored text blocks (params.blocks).
    let mut blocks = blocks_from_params(node);
    let mut mode = node
        .params
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("semantic")
        .to_owned();
    let mut full_text = String::new();

    if let Some(list) = node.params.get("files").and_then(Value::as_array) {
        for entry in list {
            if let Some(normalized) = normalize_entry(entry, base_dir, true)? {
                push_file(normalized);
            }
        }
    }

    if let Some(bin) = inputs.get("files") {
        for entry in bin
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(normalized) = normalize_entry(entry, base_dir, false)? {
                push_file(normalized);
            }
        }
    }

    if let Some(image) = inputs.get("image") {
        push_media_payload(image, &mut push_file);
    }

    let mut passthrough: Option<Value> = None;
    if let Some(any) = inputs.get("in") {
        match classify_payload(any) {
            InKind::MediaBin => {
                for entry in any
                    .get("files")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(normalized) = normalize_entry(entry, base_dir, false)? {
                        push_file(normalized);
                    }
                }
            }
            InKind::ImageAssets => {
                push_media_payload(any, &mut push_file);
            }
            InKind::ImageAsset => {
                push_media_payload(any, &mut push_file);
            }
            InKind::Segments => {
                // Upstream 口播分段 → replace stored text blocks.
                mode = any
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("semantic")
                    .to_owned();
                full_text = any
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| join_segment_texts(any));
                blocks = normalize_blocks(
                    any.get("segments")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                );
                passthrough = Some(any.clone());
            }
            InKind::ScriptText => {
                let text = any
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                if !text.trim().is_empty() {
                    full_text = text.clone();
                    blocks = vec![json!({
                        "id": "s1",
                        "index": 0,
                        "text": text,
                        "chars": text.chars().count(),
                    })];
                    mode = "length".into();
                }
                passthrough = Some(any.clone());
            }
            InKind::Other => {
                passthrough = Some(any.clone());
                if let Some(text) = any.get("text").and_then(Value::as_str) {
                    full_text = text.to_owned();
                }
            }
        }
    }

    if full_text.is_empty() && !blocks.is_empty() {
        full_text = blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
    }

    logs.push(format!(
        "中转：{} 个文件 · {} 个文本块",
        files.len(),
        blocks.len()
    ));

    let mut out = BTreeMap::new();
    out.insert("files".into(), json!({ "files": files.clone() }));

    if !blocks.is_empty() {
        out.insert(
            "segments".into(),
            json!({
                "mode": mode,
                "text": full_text,
                "segments": blocks,
            }),
        );
    }

    let media_images: Vec<Value> = files
        .iter()
        .filter(|f| {
            matches!(
                f.get("kind").and_then(Value::as_str),
                Some("image") | Some("video")
            )
        })
        .cloned()
        .collect();
    if let Some(first) = media_images.first() {
        if let Some(path) = first.get("path").cloned() {
            let mut asset = json!({ "path": path });
            if let Some(id) = first.get("id") {
                asset["id"] = id.clone();
            }
            copy_visual_caption(first, &mut asset);
            out.insert("image".into(), asset);
        }
    }
    if media_images
        .iter()
        .any(|f| f.get("kind").and_then(Value::as_str) == Some("image"))
    {
        let images: Vec<Value> = media_images
            .iter()
            .filter(|f| f.get("kind").and_then(Value::as_str) == Some("image"))
            .map(|f| {
                let mut asset = json!({ "path": f.get("path").cloned().unwrap_or(Value::Null) });
                if let Some(id) = f.get("id") {
                    asset["id"] = id.clone();
                }
                copy_visual_caption(f, &mut asset);
                asset
            })
            .collect();
        if !images.is_empty() {
            out.insert("images".into(), json!({ "images": images }));
        }
    }

    let mut text_parts = Vec::new();
    if !full_text.trim().is_empty() {
        text_parts.push(full_text.clone());
    }
    for entry in &files {
        let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "text" | "json") {
            continue;
        }
        let Some(path) = entry.get("path").and_then(Value::as_str) else {
            continue;
        };
        if let Ok(raw) = std::fs::read_to_string(base_dir.join(path)) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                text_parts.push(trimmed.to_owned());
            }
        }
    }
    if !text_parts.is_empty() {
        let mut text = json!({ "text": text_parts.join("\n\n") });
        // Keep segmentation on the text port as well: TTS accepts both ports.
        // File text appended above must remain audible, so include it as extra blocks.
        if !blocks.is_empty() {
            let mut text_blocks = blocks.clone();
            for (index, part) in text_parts.iter().skip(1).enumerate() {
                text_blocks.push(json!({ "id": format!("file-{index}"), "text": part }));
            }
            text["segments"] = json!(text_blocks);
        }
        out.insert(
            "text".into(),
            text,
        );
    }

    if let Some(json_file) = files
        .iter()
        .find(|f| f.get("kind").and_then(Value::as_str) == Some("json"))
    {
        if let Some(path) = json_file.get("path").and_then(Value::as_str) {
            if let Ok(raw) = std::fs::read_to_string(base_dir.join(path)) {
                if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                    out.insert("json".into(), value);
                } else {
                    out.insert("json".into(), json!({ "raw": raw }));
                }
            }
        }
    } else if let Some(value) = passthrough {
        out.insert("json".into(), value);
    } else if !blocks.is_empty() {
        out.insert(
            "json".into(),
            json!({ "mode": mode, "text": full_text, "segments": blocks }),
        );
    }

    Ok(out)
}

fn blocks_from_params(node: &GraphNode) -> Vec<Value> {
    let Some(list) = node.params.get("blocks").and_then(Value::as_array) else {
        return Vec::new();
    };
    normalize_blocks(list.clone())
}

fn normalize_blocks(list: Vec<Value>) -> Vec<Value> {
    list.into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())?
                .to_owned();
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("s{}", index + 1));
            let chars = item
                .get("chars")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| text.chars().count() as u64);
            Some(json!({
                "id": id,
                "index": item.get("index").and_then(Value::as_u64).unwrap_or(index as u64),
                "text": text,
                "chars": chars,
            }))
        })
        .collect()
}

enum InKind {
    MediaBin,
    ImageAssets,
    ImageAsset,
    ScriptText,
    Segments,
    Other,
}

fn classify_payload(value: &Value) -> InKind {
    if value.get("files").and_then(Value::as_array).is_some() {
        return InKind::MediaBin;
    }
    if value.get("images").and_then(Value::as_array).is_some() {
        return InKind::ImageAssets;
    }
    if value.get("segments").and_then(Value::as_array).is_some() {
        return InKind::Segments;
    }
    if value.get("path").and_then(Value::as_str).is_some()
        && value.get("text").is_none()
        && value.get("trigger").is_none()
    {
        return InKind::ImageAsset;
    }
    if value.get("text").and_then(Value::as_str).is_some() && value.get("segments").is_none()
    {
        return InKind::ScriptText;
    }
    InKind::Other
}

fn push_media_payload(value: &Value, push_file: &mut impl FnMut(Value)) {
    if let Some(list) = value.get("images").and_then(Value::as_array) {
        for (index, item) in list.iter().enumerate() {
            let Some(path) = item.get("path").and_then(Value::as_str) else {
                continue;
            };
            let detected = kind_of(Path::new(path));
            let kind = item
                .get("kind")
                .and_then(Value::as_str)
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if detected == "other" {
                        "image".into()
                    } else {
                        detected
                    }
                });
            let id = item
                .get("id")
                .cloned()
                .unwrap_or_else(|| json!(index.to_string()));
            let mut entry = json!({
                "path": path,
                "kind": kind,
                "name": PathBuf::from(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path),
                "id": id,
            });
            copy_visual_caption(item, &mut entry);
            push_file(entry);
        }
        return;
    }
    let Some(path) = value.get("path").and_then(Value::as_str) else {
        return;
    };
    let detected = kind_of(Path::new(path));
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
        .unwrap_or(detected);
    let mut entry = json!({
        "path": path,
        "kind": kind,
        "name": PathBuf::from(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path),
    });
    if let Some(id) = value.get("id") {
        entry["id"] = id.clone();
    }
    copy_visual_caption(value, &mut entry);
    push_file(entry);
}

/// Keep oral script (preferred) + optional model prompt for emit text-snap.
fn copy_visual_caption(from: &Value, into: &mut Value) {
    let script = ["script", "sourceText", "text", "caption", "label"]
        .iter()
        .find_map(|key| {
            from.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });
    if let Some(script) = script {
        into["script"] = json!(script.clone());
        into["text"] = json!(script);
    }
    if let Some(prompt) = from
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        into["prompt"] = json!(prompt);
        if into.get("text").is_none() {
            into["text"] = json!(prompt);
        }
    }
}

fn join_segment_texts(value: &Value) -> String {
    value
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|seg| seg.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn normalize_entry(
    entry: &Value,
    base_dir: &Path,
    require_file: bool,
) -> Result<Option<Value>, String> {
    let Some(path) = entry.get("path").and_then(Value::as_str) else {
        return Ok(None);
    };
    if path.is_empty() || path.contains("..") || path.starts_with('/') || path.contains('\\') {
        return Err(format!("非法路径：{path}"));
    }
    let absolute = base_dir.join(path);
    if require_file && !absolute.is_file() {
        return Err(format!("找不到文件：{path}"));
    }
    let kind = entry
        .get("kind")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if absolute.is_file() {
                kind_of(&absolute)
            } else {
                kind_of(Path::new(path))
            }
        });
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
    let mut out = json!({
        "path": path,
        "kind": kind,
        "name": name,
    });
    if let Some(id) = entry.get("id") {
        out["id"] = id.clone();
    }
    copy_visual_caption(entry, &mut out);
    Ok(Some(out))
}
