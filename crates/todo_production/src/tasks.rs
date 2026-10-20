//! Requests to external AI services, as data. The planner produces them, the
//! application routes them as effects, and `todo_ai` executes them. A task is
//! identified by the fingerprint of its complete request: the same request is
//! one task, never paid for twice.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bumped when the meaning of a request changes, so old results are not reused.
pub const TASK_SPEC_VERSION: &str = "todo-ai-capability/2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    /// huniuai.com, OpenAI-compatible chat and image generation.
    Huniu,
    /// Volcengine (Doubao) speech synthesis with word timestamps.
    Volc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    Language,
    Image,
    Video,
    Multimodal,
    Audio,
}

impl Capability {
    /// Repeating the call after an unknown outcome may be billed again.
    pub fn is_paid_per_attempt(self) -> bool {
        matches!(self, Self::Image | Self::Video)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f64,
    /// Ask for a single JSON object answer.
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRequest {
    pub model: String,
    pub prompt: String,
    /// `WIDTHxHEIGHT`, e.g. `1024x1024`.
    pub size: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    pub transparent: bool,
    /// Distinguishes deliberate extra candidates of the same prompt.
    pub variant: u32,
    /// Pictures the result must stay consistent with (characters, props):
    /// sent as reference images; empty means plain generation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    /// Content digests of `references`, filled in by the router so the task
    /// fingerprint follows the pictures, not only their paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_digests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioRequest {
    /// The selected audio model or synthesis resource.
    pub model: String,
    pub text: String,
    /// Builtin voice id (`*_bigtts`) or a cloned voice id (`S_...`).
    pub speaker: String,
    /// `seed-tts-2.0` for builtin voices, `seed-icl-2.0` for cloned ones.
    pub resource_id: String,
    pub sample_rate: u32,
    /// Percent speed change (−50..=100), the toolkit's `speech_rate`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_rate: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    /// Desired duration in seconds. Providers may quantize this value.
    pub duration: f64,
    /// `WIDTHxHEIGHT`, e.g. `1920x1080`.
    pub size: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Distinguishes deliberate extra candidates of the same prompt.
    pub variant: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultimodalRequest {
    pub model: String,
    pub prompt: String,
    /// Local or content-addressed image/video references supplied to the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<String>,
    #[serde(default)]
    pub json: bool,
    /// Content digests of `media`, filled in by the router, so the task
    /// fingerprint changes when a picture changes under the same path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub digests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "camelCase")]
pub enum TaskRequest {
    Language(LanguageRequest),
    Image(ImageRequest),
    Video(VideoRequest),
    Multimodal(MultimodalRequest),
    Audio(AudioRequest),
}

impl TaskRequest {
    pub fn capability(&self) -> Capability {
        match self {
            Self::Language(_) => Capability::Language,
            Self::Image(_) => Capability::Image,
            Self::Video(_) => Capability::Video,
            Self::Multimodal(_) => Capability::Multimodal,
            Self::Audio(_) => Capability::Audio,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpec {
    pub request: TaskRequest,
}

impl TaskSpec {
    pub fn capability(&self) -> Capability {
        self.request.capability()
    }

    /// Stable identity: SHA-256 over the versioned canonical JSON of the spec.
    /// Pictures covered by content digests count by content, not by path:
    /// the same reference picture in another working directory is the same
    /// request (and is never paid twice).
    pub fn key(&self) -> TaskKey {
        let mut identity = self.clone();
        match &mut identity.request {
            TaskRequest::Image(value)
                if !value.references.is_empty()
                    && value.reference_digests.len() == value.references.len() =>
            {
                value.references.clear();
            }
            TaskRequest::Multimodal(value)
                if !value.media.is_empty() && value.digests.len() == value.media.len() =>
            {
                value.media.clear();
            }
            _ => {}
        }
        spec_key(&identity)
    }

    /// The key journals used until 2026-10-19 (paths included), so records
    /// written then can be found and moved to [`TaskSpec::key`].
    pub fn legacy_key(&self) -> TaskKey {
        spec_key(self)
    }
}

fn spec_key(spec: &TaskSpec) -> TaskKey {
    let body = serde_json::to_vec(spec).expect("task specs always serialize");
    let mut hash = Sha256::new();
    hash.update(TASK_SPEC_VERSION.as_bytes());
    hash.update([0]);
    hash.update(&body);
    TaskKey(hex(&hash.finalize()))
}

/// Lowercase hex SHA-256 of a [`TaskSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskKey(pub String);

impl TaskKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TaskKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// SHA-256 of a value's JSON, for inputs that are not a request themselves.
pub fn fingerprint(value: &impl Serialize) -> String {
    let body = serde_json::to_vec(value).expect("fingerprinted values serialize");
    hex(&Sha256::digest(&body))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}
