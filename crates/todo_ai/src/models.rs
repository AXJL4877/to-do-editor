//! Model discovery and capability classification for OpenAI-compatible gateways.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelCapability {
    Language,
    Image,
    Video,
    Multimodal,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub capabilities: Vec<ModelCapability>,
}

impl ModelInfo {
    pub fn supports(&self, capability: ModelCapability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// Parse the OpenAI-compatible `/v1/models` response and infer capabilities.
/// Gateways differ in whether they expose `capabilities` or `modalities`, so
/// ids remain the fallback signal. Audio and multimodal models are retained so
/// callers can bind them to the corresponding capability instead of making
/// provider-specific assumptions.
pub fn classify_models(body: &Value) -> Vec<ModelInfo> {
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .into_iter()
        .flatten();
    let mut models: Vec<_> = items
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())?;
            let capabilities =
                explicit_capabilities(item).unwrap_or_else(|| infer_capabilities(&item_text(item)));
            if capabilities.is_empty() {
                return None;
            }
            Some(ModelInfo {
                id: id.to_owned(),
                capabilities,
            })
        })
        .collect();
    models.sort_by_key(|model| model.id.to_ascii_lowercase());
    models.dedup_by(|a, b| a.id.eq_ignore_ascii_case(&b.id));
    models
}

fn item_text(item: &Value) -> String {
    ["id", "name", "object", "owned_by", "type"]
        .iter()
        .filter_map(|key| item.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn explicit_capabilities(item: &Value) -> Option<Vec<ModelCapability>> {
    let mut found = Vec::new();
    let add = |name: &str, found: &mut Vec<ModelCapability>| {
        let value = name.to_ascii_lowercase();
        let capability = if value.contains("video") {
            Some(ModelCapability::Video)
        } else if value.contains("multimodal")
            || value.contains("vision-language")
            || value == "vision"
        {
            Some(ModelCapability::Multimodal)
        } else if value.contains("image") || value.contains("画") {
            Some(ModelCapability::Image)
        } else if value.contains("audio") || value.contains("speech") || value.contains("tts") {
            Some(ModelCapability::Audio)
        } else if value.contains("text")
            || value.contains("chat")
            || value.contains("completion")
            || value.contains("language")
        {
            Some(ModelCapability::Language)
        } else {
            None
        };
        if let Some(capability) = capability {
            if !found.contains(&capability) {
                found.push(capability);
            }
        }
    };
    if let Some(values) = item.get("modalities").and_then(Value::as_array) {
        for value in values.iter().filter_map(Value::as_str) {
            add(value, &mut found);
        }
    }
    if let Some(values) = item.get("supported_modalities").and_then(Value::as_array) {
        for value in values.iter().filter_map(Value::as_str) {
            add(value, &mut found);
        }
    }
    if let Some(values) = item.get("capabilities").and_then(Value::as_object) {
        for (name, enabled) in values {
            if enabled.as_bool().unwrap_or(false) {
                add(name, &mut found);
            }
        }
    }
    if let Some(values) = item.get("capabilities").and_then(Value::as_array) {
        for value in values.iter().filter_map(Value::as_str) {
            add(value, &mut found);
        }
    }
    (!found.is_empty()).then_some(found)
}

fn infer_capabilities(name: &str) -> Vec<ModelCapability> {
    let mut found = Vec::new();
    let has = |terms: &[&str]| terms.iter().any(|term| name.contains(term));
    // Infrastructure-only models are not model capabilities.
    if has(&["embedding", "moderation", "realtime", "rerank", "safety"]) {
        return found;
    }
    if has(&[
        "audio",
        "music",
        "speech",
        "transcrib",
        "tts",
        "voice",
        "whisper",
    ]) {
        found.push(ModelCapability::Audio);
    }
    if has(&["multimodal", "vision-language", "vision", "omni", "vl-"]) {
        found.push(ModelCapability::Multimodal);
    }
    let video = has(&[
        "video", "sora", "veo", "kling", "runway", "hailuo", "seedance", "wan2", "pixverse", "vidu",
    ]);
    if video {
        found.push(ModelCapability::Video);
    }
    let image = has(&[
        "image",
        "dall-e",
        "dalle",
        "face-repaint",
        "flux",
        "sdxl",
        "stable-diffusion",
        "imagen",
        "seedream",
        "kolors",
    ]);
    if image {
        found.push(ModelCapability::Image);
    }
    // Media model names often contain family words such as `gpt`, `gemini`
    // and `pro`. Do not let those broad family signals leak image/video
    // models into the text list.
    if !image
        && !video
        && !found.contains(&ModelCapability::Multimodal)
        && !found.contains(&ModelCapability::Audio)
        && (found.is_empty()
            || has(&[
                "chat",
                "text",
                "instruct",
                "reasoning",
                "completion",
                "language",
                "llm",
                "gpt",
                "gemini",
                "claude",
                "qwen",
                "deepseek",
                "llama",
                "mistral",
                "glm",
                "grok",
                "minimax",
                "doubao",
                "ernie",
                "command",
                "flash",
            ]))
    {
        found.push(ModelCapability::Language);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_explicit_and_named_models() {
        let models = classify_models(&json!({
            "data": [
                {"id": "geminiflash3.1", "capabilities": {"text": true}},
                {"id": "seedream-4", "object": "image"},
                {"id": "sora-video", "modalities": ["video"]}
            ]
        }));
        assert!(models[0].supports(ModelCapability::Language));
        assert!(models
            .iter()
            .find(|m| m.id == "seedream-4")
            .unwrap()
            .supports(ModelCapability::Image));
        assert!(models
            .iter()
            .find(|m| m.id == "sora-video")
            .unwrap()
            .supports(ModelCapability::Video));
    }

    #[test]
    fn unknown_model_is_available_for_text() {
        let models = classify_models(&json!({"data": [{"id": "provider-custom"}]}));
        assert_eq!(models[0].capabilities, vec![ModelCapability::Language]);
    }

    #[test]
    fn media_and_audio_models_are_classified_without_leaking() {
        let models = classify_models(&json!({
            "data": [
                {"id": "gpt-image-2"},
                {"id": "gemini-3.1-pro-image"},
                {"id": "kling-v3"},
                {"id": "doubao-tts-2.0"},
                {"id": "whisper-1"}
            ]
        }));
        let image = models
            .iter()
            .find(|model| model.id == "gpt-image-2")
            .unwrap();
        assert!(image.supports(ModelCapability::Image));
        assert!(!image.supports(ModelCapability::Language));
        let video = models.iter().find(|model| model.id == "kling-v3").unwrap();
        assert_eq!(video.capabilities, vec![ModelCapability::Video]);
        assert!(models
            .iter()
            .any(|model| model.id.contains("tts") && model.supports(ModelCapability::Audio)));
        assert!(models
            .iter()
            .any(|model| model.id == "whisper-1" && model.supports(ModelCapability::Audio)));
    }
}
