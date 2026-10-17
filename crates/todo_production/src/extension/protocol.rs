//! `todo-worker/1`: one JSON message per stdout line between host and worker.
use super::items::Item;
use super::manifest::Service;
use serde::{Deserialize, Serialize};

pub const WORKER_SPEC: &str = "todo-worker/1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// Host → worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostMessage {
    Hello {
        spec: String,
        host: String,
        /// The installed package directory (read-only).
        package: String,
    },
    /// Processes are reused across runs (and productions): the working
    /// directory and seed belong to each run.
    Run {
        id: String,
        op: String,
        input: serde_json::Value,
        params: serde_json::Value,
        canvas: Canvas,
        /// Every file the worker writes stays inside; item paths are relative to it.
        workdir: String,
        /// Seed for any randomness, so reruns are identical.
        seed: u64,
    },
    Reply {
        to: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ServiceError>,
    },
    Cancel {
        id: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceErrorClass {
    /// The request is malformed or not allowed.
    Invalid,
    /// Moderation refused it; a rewritten prompt may pass.
    ContentPolicy,
    /// The manifest budget for this service is spent.
    Budget,
    /// The host retried and gave up (network, provider outage, possibly billed).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceError {
    pub class: ServiceErrorClass,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunErrorClass {
    /// Rerunning cannot help.
    InvalidInput,
    /// Memory, disk or time; the host may retry once in a fresh process.
    Resource,
    Internal,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunError {
    pub class: RunErrorClass,
    pub message: String,
}

/// Worker → host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkerMessage {
    Ready {
        spec: String,
        ops: Vec<String>,
        version: String,
    },
    Call {
        id: String,
        run: String,
        service: Service,
        request: serde_json::Value,
    },
    Progress {
        run: String,
        phase: String,
        fraction: f64,
    },
    Emit {
        run: String,
        items: Vec<Item>,
    },
    Done {
        run: String,
        #[serde(default)]
        result: serde_json::Value,
    },
    Failed {
        run: String,
        error: RunError,
    },
}

/// Typed service requests (the `request` of a `call`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatCall {
    pub messages: Vec<crate::tasks::ChatMessage>,
    #[serde(default)]
    pub json: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageCall {
    pub prompt: String,
    pub size: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default)]
    pub transparent: bool,
    #[serde(default)]
    pub variant: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechCall {
    pub text: String,
    pub voice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubtitlesCall {
    pub script: String,
    /// The `file` of a speech result.
    pub speech: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProbeCall {
    pub file: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EncodeQuality {
    /// Editing quality (visually lossless at preview sizes).
    Standard,
    /// Archival quality for final export sources.
    High,
}

/// Frames `<frames>/000001.png`, `000002.png`, … at `fps`. With `alpha` the
/// host produces packed-alpha video, otherwise a short-GOP H.264.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EncodeCall {
    pub frames: String,
    pub fps: f64,
    #[serde(default)]
    pub alpha: bool,
    #[serde(default = "standard")]
    pub quality: EncodeQuality,
    /// Optional audio track to mux (a workdir file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
}

fn standard() -> EncodeQuality {
    EncodeQuality::Standard
}

/// A parsed `call`.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceCall {
    Chat(ChatCall),
    Image(ImageCall),
    Speech(SpeechCall),
    Subtitles(SubtitlesCall),
    Probe(ProbeCall),
    Encode(EncodeCall),
}

impl ServiceCall {
    pub fn parse(service: Service, request: &serde_json::Value) -> Result<Self, ServiceError> {
        let invalid = |e: serde_json::Error| ServiceError {
            class: ServiceErrorClass::Invalid,
            message: e.to_string(),
        };
        let value = request.clone();
        let call = match service {
            Service::Chat => Self::Chat(serde_json::from_value(value).map_err(invalid)?),
            Service::Image => Self::Image(serde_json::from_value(value).map_err(invalid)?),
            Service::Speech => Self::Speech(serde_json::from_value(value).map_err(invalid)?),
            Service::Subtitles => Self::Subtitles(serde_json::from_value(value).map_err(invalid)?),
            Service::Probe => Self::Probe(serde_json::from_value(value).map_err(invalid)?),
            Service::Encode => Self::Encode(serde_json::from_value(value).map_err(invalid)?),
        };
        let bad = |message: &str| {
            Err(ServiceError {
                class: ServiceErrorClass::Invalid,
                message: message.into(),
            })
        };
        match &call {
            Self::Chat(c) if c.messages.is_empty() => return bad("messages 不能为空"),
            Self::Chat(c) if c.temperature.is_some_and(|t| !(0.0..=1.0).contains(&t)) => {
                return bad("temperature 须在 0~1")
            }
            Self::Image(c) if c.prompt.trim().is_empty() || c.prompt.chars().count() > 4000 => {
                return bad("prompt 须 1~4000 字")
            }
            Self::Image(c) if !super::valid_size(&c.size) => return bad("size 须为 WxH"),
            Self::Speech(c) if c.text.trim().is_empty() || c.text.chars().count() > 1200 => {
                return bad("text 须 1~1200 字")
            }
            Self::Encode(c) if !(c.fps.is_finite() && (1.0..=120.0).contains(&c.fps)) => {
                return bad("fps 须在 1~120")
            }
            Self::Encode(c) if !super::manifest::valid_relative_path(&c.frames) => {
                return bad("frames 须为工作目录内的相对路径")
            }
            _ => {}
        }
        Ok(call)
    }
}
