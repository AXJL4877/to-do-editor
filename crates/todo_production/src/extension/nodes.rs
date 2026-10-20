//! Built-in workflow node catalog: ports, params, and payload validation.
//! Graph install checks types/params/edges; runners call [`validate_payload`]
//! on each node's inputs and outputs.
use serde_json::Value;
use std::collections::BTreeMap;

use super::manifest::Problem;

/// Wire type shared by ports (and by runtime JSON payloads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataKind {
    /// Raw oral script: `{ "text": string }`.
    ScriptText,
    /// Segmented oral: `{ "mode", "segments": [{ "id", "text", "chars", ... }] }`.
    Segments,
    /// One TTS clip: audio file + per-clip char timestamps.
    /// `{ "file", "duration", "timeline": { "text", "duration", "chars", ... } }`.
    SpeechClip,
    /// Subtitle cues: `{ "cues": [{ "text", "start", "end" }] }`.
    SubtitleCues,
    /// Timeline items (`todo-items/1` array or `{ "items": [...] }`).
    Items,
    /// File container: `{ "files": [{ "path", "kind": "text"|"audio"|"video"|"image", "name"? }] }`.
    MediaBin,
    /// Untyped (python nodes / transitional builtins).
    Any,
}

impl DataKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScriptText => "script_text",
            Self::Segments => "segments",
            Self::SpeechClip => "speech_clip",
            Self::SubtitleCues => "subtitle_cues",
            Self::Items => "items",
            Self::MediaBin => "media_bin",
            Self::Any => "any",
        }
    }

    pub fn compatible(self, other: Self) -> bool {
        self == other || self == Self::Any || other == Self::Any
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Port {
    pub name: &'static str,
    pub kind: DataKind,
    /// Required inputs must be wired by an incoming edge.
    pub required: bool,
}

/// Inspector field (n8n-style node settings).
#[derive(Debug, Clone, Copy)]
pub enum ParamField {
    Select {
        id: &'static str,
        label: &'static str,
        hint: &'static str,
        options: &'static [(&'static str, &'static str)],
        default: &'static str,
    },
    Number {
        id: &'static str,
        label: &'static str,
        hint: &'static str,
        min: f64,
        max: f64,
        step: f64,
        default: f64,
        unit: Option<&'static str>,
    },
    Text {
        id: &'static str,
        label: &'static str,
        hint: &'static str,
        default: &'static str,
        placeholder: &'static str,
    },
    /// Accepted file kinds for a media_bin (display + validation hint).
    FileKinds {
        id: &'static str,
        label: &'static str,
        hint: &'static str,
        accept: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct NodeSpec {
    pub type_name: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    /// Longer help shown in the inspector.
    pub docs: &'static str,
    pub inputs: &'static [Port],
    pub outputs: &'static [Port],
    pub fields: &'static [ParamField],
}

const SEGMENT_FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "mode",
        label: "分段方式",
        hint: "semantic=语意切分；length=按字数；paragraph=按空行段落",
        options: &[
            ("semantic", "按语意"),
            ("length", "按字数"),
            ("paragraph", "按段落"),
        ],
        default: "semantic",
    },
    ParamField::Number {
        id: "target",
        label: "目标字数",
        hint: "每段期望字数（length / semantic 参考）",
        min: 20.0,
        max: 1000.0,
        step: 10.0,
        default: 120.0,
        unit: Some("字"),
    },
    ParamField::Number {
        id: "max",
        label: "上限字数",
        hint: "单段不得超过；须 ≥ 目标字数",
        min: 20.0,
        max: 1200.0,
        step: 10.0,
        default: 200.0,
        unit: Some("字"),
    },
];

const MEDIA_BIN_FIELDS: &[ParamField] = &[
    ParamField::FileKinds {
        id: "accept",
        label: "可放入的文件",
        hint: "容器可挂口播文案、配音参考、成片素材等，供下游读取",
        accept: ".txt,.md,.mp3,.wav,.m4a,.mp4,.mov,.png,.jpg,.webp",
    },
    ParamField::Text {
        id: "note",
        label: "备注",
        hint: "仅说明，不影响运行",
        default: "",
        placeholder: "例如：本集口播稿 + BGM",
    },
];

const TTS_FIELDS: &[ParamField] = &[
    ParamField::Text {
        id: "voice",
        label: "音色",
        hint: "宿主音色 id（克隆音 S_…）",
        default: "",
        placeholder: "S_…",
    },
    ParamField::Number {
        id: "rate",
        label: "语速",
        hint: "相对默认语速的百分比偏移",
        min: -50.0,
        max: 100.0,
        step: 1.0,
        default: 0.0,
        unit: Some("%"),
    },
];

/// Canonical builtins (aliases like `speech` → `tts` resolved via [`lookup`]).
pub const CATALOG: &[NodeSpec] = &[
    NodeSpec {
        type_name: "media_bin",
        label: "素材容器",
        summary: "起始节点：挂载 txt / mp3 / mp4 等输入文件。",
        docs: "工作流入口容器。可放入口播文案（txt/md）、音频（mp3/wav）、视频（mp4）与参考图。输出 files 给下游；若含文本文件，也可经 text 口输出 script_text。",
        inputs: &[],
        outputs: &[
            Port {
                name: "files",
                kind: DataKind::MediaBin,
                required: false,
            },
            Port {
                name: "text",
                kind: DataKind::ScriptText,
                required: false,
            },
        ],
        fields: MEDIA_BIN_FIELDS,
    },
    NodeSpec {
        type_name: "script",
        label: "原始口播",
        summary: "读取文案或按主题生成，输出整篇口播文本。",
        docs: "优先用上游素材容器的 text；否则用成片输入框文案；仍为空则按主题 write_script。",
        inputs: &[Port {
            name: "text",
            kind: DataKind::ScriptText,
            required: false,
        }],
        outputs: &[Port {
            name: "script",
            kind: DataKind::ScriptText,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "segment_script",
        label: "口播分段",
        summary: "按语意 / 字数 / 段落切开整篇口播，不改写原文。",
        docs: "输入整篇 script_text，输出 segments。拼接各段 text 须还原原文。语意模式会调 chat 只选切点；字数/段落为纯规则。下游若要逐段 TTS，需 map 节点（尚未落地）对每段调用 tts。",
        inputs: &[Port {
            name: "text",
            kind: DataKind::ScriptText,
            required: true,
        }],
        outputs: &[Port {
            name: "segments",
            kind: DataKind::Segments,
            required: false,
        }],
        fields: SEGMENT_FIELDS,
    },
    NodeSpec {
        type_name: "tts",
        label: "口播转 TTS",
        summary: "一段口播 → 音频文件 + 该段内逐字时间戳。",
        docs: "一次处理一段文本。输出 speech_clip：file（wav/mp3）+ timeline.chars（相对本段的逐字时间）。",
        inputs: &[Port {
            name: "text",
            kind: DataKind::ScriptText,
            required: true,
        }],
        outputs: &[Port {
            name: "audio",
            kind: DataKind::SpeechClip,
            required: false,
        }],
        fields: TTS_FIELDS,
    },
    NodeSpec {
        type_name: "subtitles",
        label: "字幕",
        summary: "口播文案 + 配音时间轴 → 字幕 cue。",
        docs: "对齐 script 与 speech_clip，切出字幕条。",
        inputs: &[
            Port {
                name: "script",
                kind: DataKind::ScriptText,
                required: true,
            },
            Port {
                name: "speech",
                kind: DataKind::SpeechClip,
                required: true,
            },
        ],
        outputs: &[Port {
            name: "cues",
            kind: DataKind::SubtitleCues,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "emit",
        label: "入轨",
        summary: "把条目推上时间轴（渐进 emit）。",
        docs: "将 payload 转为 timeline items 并 emit。",
        inputs: &[Port {
            name: "payload",
            kind: DataKind::Any,
            required: false,
        }],
        outputs: &[Port {
            name: "items",
            kind: DataKind::Items,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "split_shots",
        label: "拆镜",
        summary: "配音句轴 + 字幕断点 → 一句一镜。",
        docs: "短句合并、长句按字幕点拆开。",
        inputs: &[
            Port {
                name: "speech",
                kind: DataKind::SpeechClip,
                required: true,
            },
            Port {
                name: "cues",
                kind: DataKind::SubtitleCues,
                required: false,
            },
        ],
        outputs: &[Port {
            name: "shots",
            kind: DataKind::Any,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "label_shots",
        label: "镜头标签",
        summary: "为每镜打画面标签（LLM）。",
        docs: "story / sublime、元素与关系写入 plan。",
        inputs: &[Port {
            name: "shots",
            kind: DataKind::Any,
            required: true,
        }],
        outputs: &[Port {
            name: "plan",
            kind: DataKind::Any,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "choose_motions",
        label: "动效选择",
        summary: "白名单动效写入镜头计划。",
        docs: "种子随机 + 白名单。",
        inputs: &[Port {
            name: "plan",
            kind: DataKind::Any,
            required: true,
        }],
        outputs: &[Port {
            name: "plan",
            kind: DataKind::Any,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "ground_emit",
        label: "底板字幕入轨",
        summary: "背景板、字幕、升华字 emit。",
        docs: "分镜定稿后的第二批入轨。",
        inputs: &[Port {
            name: "plan",
            kind: DataKind::Any,
            required: true,
        }],
        outputs: &[Port {
            name: "items",
            kind: DataKind::Items,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "picture_batch",
        label: "逐镜画面",
        summary: "生图 / 元素 / 排版后按镜 emit。",
        docs: "齐图后再 shot_items emit。",
        inputs: &[Port {
            name: "plan",
            kind: DataKind::Any,
            required: true,
        }],
        outputs: &[Port {
            name: "items",
            kind: DataKind::Items,
            required: false,
        }],
        fields: &[],
    },
    NodeSpec {
        type_name: "done",
        label: "完成",
        summary: "收束并回传摘要。",
        docs: "工作流终点。",
        inputs: &[Port {
            name: "result",
            kind: DataKind::Any,
            required: false,
        }],
        outputs: &[],
        fields: &[],
    },
];

const PYTHON_SPEC: NodeSpec = NodeSpec {
    type_name: "python",
    label: "自定义 Python",
    summary: "调用扩展 worker 的单个 op。",
    docs: "python:<op>，op 须在 worker.ops 中声明。",
    inputs: &[Port {
        name: "in",
        kind: DataKind::Any,
        required: false,
    }],
    outputs: &[Port {
        name: "out",
        kind: DataKind::Any,
        required: false,
    }],
    fields: &[],
};

/// Resolve a node `type` string to its spec (`speech` aliases `tts`).
pub fn lookup(node_type: &str) -> Option<&'static NodeSpec> {
    if node_type == "speech" {
        return CATALOG.iter().find(|s| s.type_name == "tts");
    }
    if node_type.starts_with("python:") {
        return Some(&PYTHON_SPEC);
    }
    CATALOG.iter().find(|s| s.type_name == node_type)
}

pub fn is_known_type(node_type: &str) -> bool {
    lookup(node_type).is_some()
}

pub fn builtin_type_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = CATALOG.iter().map(|s| s.type_name).collect();
    names.push("speech"); // documented alias of tts
    names
}

/// Install-time params for a node instance.
pub fn validate_params(node_type: &str, params: &BTreeMap<String, Value>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let path = "params";
    match canonical_type(node_type) {
        "segment_script" => {
            if let Some(mode) = params.get("mode") {
                let ok = mode
                    .as_str()
                    .is_some_and(|m| matches!(m, "semantic" | "length" | "paragraph"));
                if !ok {
                    problems.push(Problem {
                        path: format!("{path}.mode"),
                        message: "须为 semantic | length | paragraph".into(),
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
                if !matches!(key.as_str(), "mode" | "target" | "max") {
                    problems.push(Problem {
                        path: format!("{path}.{key}"),
                        message: "未知参数".into(),
                    });
                }
            }
        }
        "tts" | "speech" => {
            if let Some(voice) = params.get("voice") {
                if !voice.as_str().is_some_and(|v| !v.is_empty() && v.len() <= 64) {
                    problems.push(Problem {
                        path: format!("{path}.voice"),
                        message: "非空音色 id，≤ 64".into(),
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
            for key in params.keys() {
                if !matches!(key.as_str(), "voice" | "rate") {
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
        "media_bin" => {
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
            for key in params.keys() {
                if !matches!(key.as_str(), "accept" | "note" | "files") {
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
        DataKind::ScriptText => validate_script_text(value),
        DataKind::Segments => validate_segments(value),
        DataKind::SpeechClip => validate_speech_clip(value),
        DataKind::SubtitleCues => validate_subtitle_cues(value),
        DataKind::Items => validate_items_payload(value),
        DataKind::MediaBin => validate_media_bin(value),
    }
}

fn validate_media_bin(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("media_bin 须为对象")?;
    let files = obj
        .get("files")
        .and_then(Value::as_array)
        .ok_or("media_bin.files 须为数组")?;
    if files.len() > 64 {
        return Err("media_bin.files 最多 64 个".into());
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
            .ok_or_else(|| format!("files[{i}].kind 须为 text|audio|video|image"))?;
        if !matches!(kind, "text" | "audio" | "video" | "image") {
            return Err(format!("files[{i}].kind 须为 text|audio|video|image"));
        }
        let lower = path.to_ascii_lowercase();
        let ok_ext = match kind {
            "text" => lower.ends_with(".txt") || lower.ends_with(".md"),
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
            _ => false,
        };
        if !ok_ext {
            return Err(format!("files[{i}] 扩展名与 kind={kind} 不匹配"));
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
    if !matches!(mode, "semantic" | "length" | "paragraph") {
        return Err("segments.mode 须为 semantic | length | paragraph".into());
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
    let file = obj
        .get("file")
        .and_then(Value::as_str)
        .ok_or("speech_clip.file 须为工作目录内相对路径")?;
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
        if end > duration + eps || end > tl_dur + eps {
            return Err(format!(
                "timeline.chars[{i}].end 超出本段 duration（须为当段时间）"
            ));
        }
        if start + eps < prev_end {
            // Allow tiny overlap; reject large backwards jumps.
            if prev_end - start > 0.25 {
                return Err(format!("timeline.chars[{i}] 时间轴回退过大"));
            }
        }
        prev_end = end.max(prev_end);
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
            match compatible.as_slice() {
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
        // segments → tts is NOT directly compatible (need per-segment text).
        assert!(resolve_edge_ports("segment_script", "tts", None, None).is_err());
        let (o, i) = resolve_edge_ports("script", "tts", None, None).unwrap();
        assert_eq!(o.name, "script");
        assert_eq!(i.name, "text");
    }

    #[test]
    fn segment_params_reject_bad_mode() {
        let mut params = BTreeMap::new();
        params.insert("mode".into(), json!("random"));
        let found = validate_params("segment_script", &params);
        assert!(found.iter().any(|p| p.path.contains("mode")), "{found:?}");
    }
}
