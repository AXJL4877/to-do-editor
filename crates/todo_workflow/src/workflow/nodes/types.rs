//! Shared port / param / node-spec types for the builtin catalog.

/// Wire type shared by ports (and by runtime JSON payloads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataKind {
    /// Raw oral script: `{ "text": string }`.
    ScriptText,
    /// Segmented oral: `{ "mode", "segments": [{ "id", "text", "chars", ... }] }`.
    Segments,
    /// One speech clock segment (ruler). Audio file is optional — voice media
    /// travels on a separate `media` port.
    /// `{ "id"?, "duration", "timeline": { "text", "duration", "chars", ... }, "file"? }`.
    SpeechClip,
    /// Batch of speech clock segments (one per upstream text block).
    /// `{ "clips": [ SpeechClip, … ] }` — timing only; wav/mp3 go via MediaBin.
    SpeechClips,
    /// Subtitle cues: `{ "cues": [{ "text", "start", "end" }] }`.
    SubtitleCues,
    /// Timeline items (`todo-items/1` array or `{ "items": [...] }`).
    Items,
    /// File container: `{ "files": [{ "path", "kind": "text"|"audio"|"video"|"image"|"json"|"other", "name"? }] }`.
    MediaBin,
    /// Workflow start pulse: `{ "trigger": true, "at"?: number }`.
    Trigger,
    /// LLM / multimodal text reply: `{ "text": string, "model"?: string }`.
    ChatReply,
    /// Generated or reference still: `{ "path": string, "mime"?: "image/png"|… }`.
    ImageAsset,
    /// Batch of stills (one per upstream text block): `{ "images": [ ImageAsset… ] }`.
    ImageAssets,
    /// Untyped (python nodes / transitional builtins).
    Any,
}

impl DataKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScriptText => "script_text",
            Self::Segments => "segments",
            Self::SpeechClip => "speech_clip",
            Self::SpeechClips => "speech_clips",
            Self::SubtitleCues => "subtitle_cues",
            Self::Items => "items",
            Self::MediaBin => "media_bin",
            Self::Trigger => "trigger",
            Self::ChatReply => "chat_reply",
            Self::ImageAsset => "image_asset",
            Self::ImageAssets => "image_assets",
            Self::Any => "any",
        }
    }

    pub fn display_label(self) -> &'static str {
        match self {
            Self::ScriptText => "txt",
            Self::Segments => "段",
            Self::SpeechClip => "尺",
            Self::SpeechClips => "尺",
            Self::SubtitleCues => "srt",
            Self::Items => "items",
            Self::MediaBin => "files",
            Self::Trigger => "go",
            Self::ChatReply => "reply",
            Self::ImageAsset => "img",
            Self::ImageAssets => "imgs",
            Self::Any => "any",
        }
    }

    pub fn compatible(self, other: Self) -> bool {
        self == other
            || self == Self::Any
            || other == Self::Any
            || (self == Self::ChatReply && other == Self::ScriptText)
            || (self == Self::ScriptText && other == Self::ChatReply)
            || (self == Self::Segments && other == Self::ScriptText)
            || (self == Self::ScriptText && other == Self::Segments)
            || (self == Self::SpeechClip && other == Self::SpeechClips)
            || (self == Self::SpeechClips && other == Self::SpeechClip)
            || (self == Self::ImageAsset && other == Self::ImageAssets)
            || (self == Self::ImageAssets && other == Self::ImageAsset)
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

/// Palette menu section (二级菜单分组). Order is display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeGroup {
    /// 触发、素材、中转、完成
    Basic,
    /// 对话 / 生图 / 多模态
    Ai,
    /// 口播文案与分段
    Script,
    /// TTS 与字幕
    Audio,
    /// 拆镜、标签、动效、逐镜画面
    Shots,
    /// emit / 入轨
    Timeline,
}

impl NodeGroup {
    pub const ALL: &[NodeGroup] = &[
        Self::Basic,
        Self::Ai,
        Self::Script,
        Self::Audio,
        Self::Shots,
        Self::Timeline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Basic => "基础",
            Self::Ai => "大模型",
            Self::Script => "文案",
            Self::Audio => "配音字幕",
            Self::Shots => "分镜画面",
            Self::Timeline => "入轨",
        }
    }

    pub fn order(self) -> u8 {
        match self {
            Self::Basic => 0,
            Self::Ai => 1,
            Self::Script => 2,
            Self::Audio => 3,
            Self::Shots => 4,
            Self::Timeline => 5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NodeSpec {
    pub type_name: &'static str,
    pub label: &'static str,
    /// Add-node menu category.
    pub group: NodeGroup,
    pub summary: &'static str,
    /// Longer help shown in the inspector.
    pub docs: &'static str,
    pub inputs: &'static [Port],
    pub outputs: &'static [Port],
    pub fields: &'static [ParamField],
}

