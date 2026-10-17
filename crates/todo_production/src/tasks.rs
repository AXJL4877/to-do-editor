//! Requests to external AI services, as data. The planner produces them, the
//! application routes them as effects, and `todo_ai` executes them. A task is
//! identified by the fingerprint of its complete request: the same request is
//! one task, never paid for twice.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bumped when the meaning of a request changes, so old results are not reused.
pub const TASK_SPEC_VERSION: &str = "todo-task/1";

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
    Chat,
    Image,
    Speech,
}

impl Capability {
    /// Repeating the call after an unknown outcome may be billed again.
    pub fn is_paid_per_attempt(self) -> bool {
        matches!(self, Self::Image)
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
pub struct ChatRequest {
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechRequest {
    pub text: String,
    pub speaker: String,
    pub resource_id: String,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "camelCase")]
pub enum TaskRequest {
    Chat(ChatRequest),
    Image(ImageRequest),
    Speech(SpeechRequest),
}

impl TaskRequest {
    pub fn capability(&self) -> Capability {
        match self {
            Self::Chat(_) => Capability::Chat,
            Self::Image(_) => Capability::Image,
            Self::Speech(_) => Capability::Speech,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpec {
    pub provider: Provider,
    pub request: TaskRequest,
}

impl TaskSpec {
    pub fn capability(&self) -> Capability {
        self.request.capability()
    }

    /// Stable identity: SHA-256 over the versioned canonical JSON of the spec.
    pub fn key(&self) -> TaskKey {
        let body = serde_json::to_vec(self).expect("task specs always serialize");
        let mut hash = Sha256::new();
        hash.update(TASK_SPEC_VERSION.as_bytes());
        hash.update([0]);
        hash.update(&body);
        TaskKey(hex(&hash.finalize()))
    }
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
