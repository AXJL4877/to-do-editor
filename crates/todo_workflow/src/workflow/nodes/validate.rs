//! Param and payload validation for builtin nodes.
use serde_json::Value;
use std::collections::BTreeMap;

use super::types::{DataKind, Port};
use super::lookup;
use crate::workflow::manifest::Problem;

/// Install-time params for a node instance.
pub fn validate_params(node_type: &str, params: &BTreeMap<String, Value>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let path = "params";
    match canonical_type(node_type) {
        "segment_script" => {
            if let Some(mode) = params.get("mode") {
                let ok = mode.as_str().is_some_and(|m| {
                    matches!(m, "semantic" | "length" | "paragraph" | "sentence")
                });
                if !ok {
                    problems.push(Problem {
                        path: format!("{path}.mode"),
                        message: "须为 semantic | length | paragraph | sentence".into(),
                    });
                }
            }
            let target = params.get("target").and_then(Value::as_u64);
            let max = params.get("max").and_then(Value::as_u64);
            if let Some(t) = target {
                if !(20..=1000).contains(&t) {
                    problems.push(Problem {
                        path: format!("{path}.target"),
                        message: "20~1000".into(),
                    });
                }
            }
            if let Some(m) = max {
                if !(20..=1200).contains(&m) {
                    problems.push(Problem {
                        path: format!("{path}.max"),
                        message: "20~1200".into(),
                    });
                }
            }
            if let (Some(t), Some(m)) = (target, max) {
                if m < t {
                    problems.push(Problem {
                        path: format!("{path}.max"),
                        message: "须 ≥ target".into(),
                    });
                }
            }
            for key in params.keys() {
                if !matches!(key.as_str(), "mode" | "target" | "max" | "model" | "title") {
                    problems.push(Problem {
                        path: format!("{path}.{key}"),
                        message: "未知参数".into(),
                    });
                }
            }
        }
        "tts" | "speech" => {
            // Voice is optional on the canvas; filled before run (or left empty for host default).
            if let Some(voice) = params.get("voice") {
                let ok = match voice.as_str() {
                    Some("") => true,
                    Some(v) => v.len() <= 64,
                    None => false,
                };
                if !ok {
                    problems.push(Problem {
                        path: format!("{path}.voice"),
                        message: "音色 id 须为字符串，≤ 64".into(),
                    });
                }
            }
            if let Some(rate) = params.get("rate") {
                let ok = rate.as_i64().is_some_and(|r| (-50..=100).contains(&r))
                    || rate
                        .as_f64()
                        .is_some_and(|r| r.is_finite() && (-50.0..=100.0).contains(&r));
                if !ok {
                    problems.push(Problem {
                        path: format!("{path}.rate"),
                        message: "语速 -50~100".into(),
                    });
                }
            }
            if let Some(handoff) = params.get("handoff") {
                if !handoff
                    .as_str()
                    .is_some_and(|v| matches!(v, "wait" | "stream"))
                {
                    problems.push(Problem {
                        path: format!("{path}.handoff"),
                        message: "出流方式须为 wait | stream".into(),
                    });
                }
            }
            for key in params.keys() {
                if !matches!(key.as_str(), "voice" | "rate" | "handoff") {
                    problems.push(Problem {
                        path: format!("{path}.{key}"),
                        message: "未知参数".into(),
                    });
                }
            }
        }
        "script" => {
            for key in params.keys() {
                if key.chars().count() > 32 {
                    problems.push(Problem {
                        path: format!("{path}.{key}"),
                        message: "参数名过长".into(),
                    });
                }
            }
        }
        "media_bin" | "relay" => {
            if let Some(accept) = params.get("accept") {
                if !accept.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 200) {
                    problems.push(Problem {
                        path: format!("{path}.accept"),
                        message: "accept 须为扩展名列表字符串".into(),
                    });
                }
            }
            if let Some(note) = params.get("note") {
                if note.as_str().is_none_or(|s| s.chars().count() > 200) {
                    problems.push(Problem {
                        path: format!("{path}.note"),
                        message: "备注 ≤ 200 字".into(),
                    });
                }
            }
            if let Some(files) = params.get("files") {
                if let Err(e) = validate_media_bin(&serde_json::json!({ "files": files })) {
                    problems.push(Problem {
                        path: format!("{path}.files"),
                        message: e,
                    });
                }
            }
            if node_type == "relay" {
                if let Some(blocks) = params.get("blocks") {
                    if let Err(e) = validate_relay_blocks(blocks) {
                        problems.push(Problem {
                            path: format!("{path}.blocks"),
                            message: e,
                        });
                    }
                }
            }
            for key in params.keys() {
                let allowed = if node_type == "relay" {
                    matches!(key.as_str(), "accept" | "note" | "files" | "blocks" | "mode")
                } else {
                    matches!(key.as_str(), "accept" | "note" | "files")
                };
                if !allowed {
                    problems.push(Problem {
                        path: format!("{path}.{key}"),
                        message: "未知参数".into(),
                    });
                }
            }
        }
        _ => {
            // Other builtins: allow free params for now (notes / transitional).
        }
    }
    problems
}

fn canonical_type(node_type: &str) -> &str {
    if node_type == "speech" {
        "tts"
    } else {
        node_type
    }
}

/// Runtime check that `value` matches a port's data kind.
pub fn validate_payload(kind: DataKind, value: &Value) -> Result<(), String> {
    match kind {
        DataKind::Any => Ok(()),
        DataKind::ScriptText | DataKind::ChatReply => validate_script_text(value),
        DataKind::Segments => validate_segments(value),
        DataKind::SpeechClip => validate_speech_clip(value),
        DataKind::SpeechClips => validate_speech_clips(value),
        DataKind::SubtitleCues => validate_subtitle_cues(value),
        DataKind::Items => validate_items_payload(value),
        DataKind::MediaBin => validate_media_bin(value),
        DataKind::Trigger => validate_trigger(value),
        DataKind::ImageAsset => validate_image_asset(value),
        DataKind::ImageAssets => validate_image_assets(value),
    }
}

fn validate_trigger(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("trigger 须为对象")?;
    if obj.get("trigger").and_then(Value::as_bool) != Some(true) {
        return Err("trigger.trigger 须为 true".into());
    }
    Ok(())
}

fn validate_image_asset(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("image_asset 须为对象")?;
    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .ok_or("image_asset.path 须为字符串")?;
    if path.is_empty() || path.contains("..") || path.starts_with('/') || path.contains('\\') {
        return Err("image_asset.path 非法".into());
    }
    Ok(())
}

fn validate_image_assets(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("image_assets 须为对象")?;
    let list = obj
        .get("images")
        .and_then(Value::as_array)
        .ok_or("image_assets.images 须为数组")?;
    if list.is_empty() {
        return Err("image_assets.images 不能为空".into());
    }
    if list.len() > 256 {
        return Err("image_assets.images 最多 256 张".into());
    }
    for (i, item) in list.iter().enumerate() {
        validate_image_asset(item).map_err(|e| format!("images[{i}]: {e}"))?;
    }
    Ok(())
}

fn validate_media_bin(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("media_bin 须为对象")?;
    let files = obj
        .get("files")
        .and_then(Value::as_array)
        .ok_or("media_bin.files 须为数组")?;
    // Align with image_assets / segments / speech_clips: relay hubs often mirror
    // one file per parallel image_gen slot (tens to low hundreds).
    if files.len() > 256 {
        return Err("media_bin.files 最多 256 个".into());
    }
    for (i, file) in files.iter().enumerate() {
        let f = file
            .as_object()
            .ok_or_else(|| format!("files[{i}] 须为对象"))?;
        let path = f
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("files[{i}].path 须为字符串"))?;
        if path.is_empty() || path.contains("..") || path.starts_with('/') || path.contains('\\')
        {
            return Err(format!("files[{i}].path 非法"));
        }
        let kind = f
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("files[{i}].kind 须为 text|audio|video|image|json|other")
            })?;
        if !matches!(kind, "text" | "audio" | "video" | "image" | "json" | "other") {
            return Err(format!(
                "files[{i}].kind 须为 text|audio|video|image|json|other"
            ));
        }
        let lower = path.to_ascii_lowercase();
        let ok_ext = match kind {
            "text" => {
                lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".json")
            }
            "json" => lower.ends_with(".json"),
            "audio" => {
                lower.ends_with(".mp3")
                    || lower.ends_with(".wav")
                    || lower.ends_with(".m4a")
                    || lower.ends_with(".ogg")
            }
            "video" => {
                lower.ends_with(".mp4")
                    || lower.ends_with(".mov")
                    || lower.ends_with(".webm")
            }
            "image" => {
                lower.ends_with(".png")
                    || lower.ends_with(".jpg")
                    || lower.ends_with(".jpeg")
                    || lower.ends_with(".webp")
            }
            "other" => true,
            _ => false,
        };
        if !ok_ext {
            return Err(format!("files[{i}] 扩展名与 kind={kind} 不匹配"));
        }
    }
    Ok(())
}

fn validate_relay_blocks(value: &Value) -> Result<(), String> {
    let list = value
        .as_array()
        .ok_or("blocks 须为数组")?;
    if list.len() > 256 {
        return Err("blocks 最多 256 块".into());
    }
    for (i, seg) in list.iter().enumerate() {
        let s = seg
            .as_object()
            .ok_or_else(|| format!("blocks[{i}] 须为对象"))?;
        let text = s
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("blocks[{i}].text 须为字符串"))?;
        if text.trim().is_empty() {
            return Err(format!("blocks[{i}].text 不能为空"));
        }
        if text.chars().count() > 2000 {
            return Err(format!("blocks[{i}].text 最多 2000 字"));
        }
    }
    Ok(())
}

fn validate_script_text(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("script_text 须为对象")?;
    let text = obj
        .get("text")
        .and_then(Value::as_str)
        .ok_or("script_text.text 须为字符串")?;
    let n = text.chars().count();
    if text.trim().is_empty() {
        return Err("script_text.text 不能为空".into());
    }
    if n > 12000 {
        return Err("script_text.text 最多 12000 字".into());
    }
    Ok(())
}

fn validate_segments(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("segments 须为对象")?;
    let mode = obj
        .get("mode")
        .and_then(Value::as_str)
        .ok_or("segments.mode 须为字符串")?;
    if !matches!(
        mode,
        "semantic" | "length" | "paragraph" | "sentence" | "edit" | "image_prompt"
    ) {
        return Err(
            "segments.mode 须为 semantic | length | paragraph | sentence | edit | image_prompt"
                .into(),
        );
    }
    let list = obj
        .get("segments")
        .and_then(Value::as_array)
        .ok_or("segments.segments 须为数组")?;
    if list.is_empty() {
        return Err("segments.segments 不能为空".into());
    }
    if list.len() > 256 {
        return Err("segments.segments 最多 256 段".into());
    }
    for (i, seg) in list.iter().enumerate() {
        let s = seg
            .as_object()
            .ok_or_else(|| format!("segments.segments[{i}] 须为对象"))?;
        let text = s
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("segments.segments[{i}].text 须为字符串"))?;
        if text.trim().is_empty() {
            return Err(format!("segments.segments[{i}].text 不能为空"));
        }
        if text.chars().count() > 2000 {
            return Err(format!("segments.segments[{i}].text 最多 2000 字"));
        }
        if let Some(chars) = s.get("chars") {
            let n = chars
                .as_u64()
                .ok_or_else(|| format!("segments.segments[{i}].chars 须为整数"))?;
            if n == 0 || n as usize != text.chars().count() {
                return Err(format!(
                    "segments.segments[{i}].chars 须等于 text 字数"
                ));
            }
        }
        if let Some(id) = s.get("id") {
            if id.as_str().map(|x| x.is_empty()).unwrap_or(true)
                && id.as_u64().is_none()
            {
                return Err(format!("segments.segments[{i}].id 须为非空字符串或整数"));
            }
        }
    }
    Ok(())
}

fn validate_speech_clip(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("speech_clip 须为对象")?;
    // `file` is optional: clips are the clock; voice wav rides on MediaBin `media`.
    if let Some(file) = obj.get("file").and_then(Value::as_str) {
        if file.is_empty() || file.contains("..") || file.starts_with('/') || file.contains('\\') {
            return Err("speech_clip.file 非法路径".into());
        }
        let lower = file.to_ascii_lowercase();
        if !(lower.ends_with(".wav")
            || lower.ends_with(".mp3")
            || lower.ends_with(".m4a")
            || lower.ends_with(".ogg"))
        {
            return Err("speech_clip.file 须为 wav/mp3/m4a/ogg".into());
        }
    }
    let duration = obj
        .get("duration")
        .and_then(Value::as_f64)
        .ok_or("speech_clip.duration 须为数字")?;
    if !duration.is_finite() || duration < 0.0 || duration > 3600.0 {
        return Err("speech_clip.duration 须为 0~3600 秒".into());
    }
    let timeline = obj
        .get("timeline")
        .and_then(Value::as_object)
        .ok_or("speech_clip.timeline 须为对象")?;
    let tl_dur = timeline
        .get("duration")
        .and_then(Value::as_f64)
        .unwrap_or(duration);
    if !tl_dur.is_finite() || tl_dur < 0.0 {
        return Err("speech_clip.timeline.duration 非法".into());
    }
    // Char timestamps are relative to this clip (当段).
    let chars = timeline
        .get("chars")
        .and_then(Value::as_array)
        .ok_or("speech_clip.timeline.chars 须为数组（逐字时间戳）")?;
    if chars.is_empty() {
        return Err("speech_clip.timeline.chars 不能为空".into());
    }
    let eps = 0.05;
    let mut prev_end = 0.0_f64;
    for (i, ch) in chars.iter().enumerate() {
        let c = ch
            .as_object()
            .ok_or_else(|| format!("timeline.chars[{i}] 须为对象"))?;
        let text = c
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("timeline.chars[{i}].text 须为字符串"))?;
        if text.is_empty() {
            return Err(format!("timeline.chars[{i}].text 不能为空"));
        }
        let start = c
            .get("start")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("timeline.chars[{i}].start 须为数字"))?;
        let end = c
            .get("end")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("timeline.chars[{i}].end 须为数字"))?;
        if !start.is_finite() || !end.is_finite() || start < -eps || end < start - eps {
            return Err(format!("timeline.chars[{i}] 起止时间非法（相对本段音频）"));
        }
        if end > duration + 0.5 || end > tl_dur + 0.5 {
            return Err(format!(
                "timeline.chars[{i}].end 超出本段 duration（须为当段时间）"
            ));
        }
        let _ = start;
        prev_end = end.max(prev_end);
    }
    Ok(())
}

fn validate_speech_clips(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("speech_clips 须为对象")?;
    let clips = obj
        .get("clips")
        .and_then(Value::as_array)
        .ok_or("speech_clips.clips 须为数组")?;
    if clips.is_empty() {
        return Err("speech_clips.clips 不能为空".into());
    }
    if clips.len() > 256 {
        return Err("speech_clips.clips 最多 256 段".into());
    }
    for (i, clip) in clips.iter().enumerate() {
        validate_speech_clip(clip).map_err(|e| format!("clips[{i}]: {e}"))?;
    }
    Ok(())
}

fn validate_subtitle_cues(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("subtitle_cues 须为对象")?;
    let cues = obj
        .get("cues")
        .and_then(Value::as_array)
        .ok_or("subtitle_cues.cues 须为数组")?;
    for (i, cue) in cues.iter().enumerate() {
        let c = cue
            .as_object()
            .ok_or_else(|| format!("cues[{i}] 须为对象"))?;
        if c.get("text").and_then(Value::as_str).is_none_or(|t| t.is_empty()) {
            return Err(format!("cues[{i}].text 须为非空字符串"));
        }
        let start = c
            .get("start")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("cues[{i}].start 须为数字"))?;
        let end = c
            .get("end")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("cues[{i}].end 须为数字"))?;
        if !start.is_finite() || !end.is_finite() || end < start {
            return Err(format!("cues[{i}] 时间非法"));
        }
    }
    Ok(())
}

fn validate_items_payload(value: &Value) -> Result<(), String> {
    if let Some(arr) = value.as_array() {
        if arr.len() > 10_000 {
            return Err("items 过多".into());
        }
        return Ok(());
    }
    if let Some(obj) = value.as_object() {
        if let Some(arr) = obj.get("items").and_then(Value::as_array) {
            if arr.len() > 10_000 {
                return Err("items 过多".into());
            }
            return Ok(());
        }
    }
    Err("items 须为数组或 {{ items: [...] }}".into())
}

/// Resolve which output/input ports an edge uses; check kind compatibility.
pub fn resolve_edge_ports(
    from_type: &str,
    to_type: &str,
    out: Option<&str>,
    input: Option<&str>,
) -> Result<(Port, Port), String> {
    let from = lookup(from_type).ok_or_else(|| format!("未知源类型 {from_type}"))?;
    let to = lookup(to_type).ok_or_else(|| format!("未知目标类型 {to_type}"))?;

    let out_port = match out {
        Some(name) => *from
            .outputs
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("源节点没有输出端口 {name}"))?,
        None if from.outputs.is_empty() => {
            return Err("源节点没有输出端口".into());
        }
        None if from.outputs.len() == 1 => from.outputs[0],
        None => {
            return Err("源节点有多个输出，请指定 out".into());
        }
    };

    let in_port = match input {
        Some(name) => *to
            .inputs
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("目标节点没有输入端口 {name}"))?,
        None => {
            let compatible: Vec<&Port> = to
                .inputs
                .iter()
                .filter(|p| out_port.kind.compatible(p.kind))
                .collect();
            // Prefer exact kind (e.g. speech_clips→clips) over Any sinks
            // so tts.clips can auto-wire to emit without naming `in`.
            let exact: Vec<&Port> = compatible
                .iter()
                .copied()
                .filter(|p| p.kind == out_port.kind)
                .collect();
            let candidates = if exact.len() == 1 {
                exact
            } else {
                compatible
            };
            match candidates.as_slice() {
                [only] => **only,
                [] => {
                    return Err(format!(
                        "没有与 {} 兼容的输入端口",
                        out_port.kind.as_str()
                    ));
                }
                _ => return Err("多个兼容输入端口，请指定 in".into()),
            }
        }
    };

    if !out_port.kind.compatible(in_port.kind) {
        return Err(format!(
            "类型不匹配：{} → {}（{} ↛ {}）",
            out_port.name,
            in_port.name,
            out_port.kind.as_str(),
            in_port.kind.as_str()
        ));
    }
    Ok((out_port, in_port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn segment_and_tts_payloads_round_trip_rules() {
        validate_payload(
            DataKind::ScriptText,
            &json!({"text": "今天讲沉没成本。"}),
        )
        .unwrap();
        validate_payload(
            DataKind::Segments,
            &json!({
                "mode": "length",
                "segments": [
                    {"id": "0", "text": "今天讲沉没成本。", "chars": 8}
                ]
            }),
        )
        .unwrap();
        validate_payload(
            DataKind::SpeechClip,
            &json!({
                "file": "services/seg0.mp3",
                "duration": 1.2,
                "timeline": {
                    "text": "今天",
                    "duration": 1.2,
                    "chars": [
                        {"id": 0, "text": "今", "start": 0.0, "end": 0.4},
                        {"id": 1, "text": "天", "start": 0.4, "end": 0.9}
                    ]
                }
            }),
        )
        .unwrap();
        assert!(validate_payload(
            DataKind::SpeechClip,
            &json!({
                "file": "out/a.mp3",
                "duration": 1.0,
                "timeline": {
                    "duration": 1.0,
                    "chars": [{"id": 0, "text": "啊", "start": 0.0, "end": 2.0}]
                }
            }),
        )
        .unwrap_err()
        .contains("当段"));
    }

    #[test]
    fn script_to_segment_to_tts_edges_typecheck() {
        let (o, i) =
            resolve_edge_ports("script", "segment_script", Some("script"), Some("text")).unwrap();
        assert_eq!(o.kind, DataKind::ScriptText);
        assert_eq!(i.kind, DataKind::ScriptText);
        // Segments can feed TTS directly. The runner preserves the original
        // narration text from the bundle while exposing the segment metadata
        // to downstream shot planning.
        let (o, i) = resolve_edge_ports("segment_script", "tts", None, None).unwrap();
        assert_eq!(o.kind, DataKind::Segments);
        assert_eq!(i.kind, DataKind::ScriptText);
        let (o, i) = resolve_edge_ports("script", "tts", None, None).unwrap();
        assert_eq!(o.name, "script");
        assert_eq!(i.name, "text");
        let (o, i) = resolve_edge_ports("tts", "emit_sequence", Some("clips"), None).unwrap();
        assert_eq!(o.name, "clips");
        assert_eq!(o.kind, DataKind::SpeechClips);
        assert_eq!(i.kind, DataKind::SpeechClips);
        assert!(
            resolve_edge_ports("tts", "emit_sequence", Some("audio"), None).is_err(),
            "tts 不再有 audio 口"
        );
        let (o, i) = resolve_edge_ports("tts", "emit_visuals", Some("clips"), None).unwrap();
        assert_eq!((o.name, i.name), ("clips", "clips"));
        let (o, i) = resolve_edge_ports("tts", "emit_sequence", Some("media"), Some("media")).unwrap();
        assert_eq!((o.name, i.name), ("media", "media"));
        assert_eq!(o.kind, DataKind::MediaBin);
        let (o, i) =
            resolve_edge_ports("relay", "emit_visuals", Some("images"), Some("visuals")).unwrap();
        assert_eq!((o.name, i.name), ("images", "visuals"));
        let (o, i) =
            resolve_edge_ports("image_prompt", "image_gen", Some("prompt"), Some("prompt")).unwrap();
        assert_eq!(o.kind, DataKind::Segments);
        assert_eq!(i.kind, DataKind::ScriptText);
        assert!(DataKind::Segments.compatible(DataKind::ScriptText));
    }

    #[test]
    fn media_bin_files_allow_parallel_image_hub_sizes() {
        let files: Vec<_> = (0..67)
            .map(|i| json!({ "path": format!("assets/a{i}.png"), "kind": "image" }))
            .collect();
        validate_payload(DataKind::MediaBin, &json!({ "files": files })).unwrap();

        let too_many: Vec<_> = (0..257)
            .map(|i| json!({ "path": format!("assets/b{i}.png"), "kind": "image" }))
            .collect();
        let err = validate_payload(DataKind::MediaBin, &json!({ "files": too_many })).unwrap_err();
        assert!(err.contains("最多 256"), "{err}");
    }

    #[test]
    fn segment_params_reject_bad_mode() {
        let mut params = BTreeMap::new();
        params.insert("mode".into(), json!("random"));
        let found = validate_params("segment_script", &params);
        assert!(found.iter().any(|p| p.path.contains("mode")), "{found:?}");
    }

    #[test]
    fn tts_allows_empty_voice_on_canvas() {
        let mut params = BTreeMap::new();
        params.insert("voice".into(), json!(""));
        params.insert("rate".into(), json!(0));
        assert!(
            validate_params("tts", &params).is_empty(),
            "{:?}",
            validate_params("tts", &params)
        );
        params.insert("voice".into(), json!("S_demo"));
        assert!(validate_params("tts", &params).is_empty());
    }
}
