//! Model discovery and capability classification for OpenAI-compatible gateways.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelCapability {
    Text,
    Image,
    Video,
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
/// ids remain the fallback signal. Unknown models are treated as text models,
/// which is the least surprising default for the existing chat pipeline.
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
            Some(ModelInfo {
                id: id.to_owned(),
                capabilities,
            })
        })
        .collect();
    models.sort_by(|a, b| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()));
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
        } else if value.contains("image") || value.contains("vision") || value.contains("画") {
            Some(ModelCapability::Image)
        } else if value.contains("text")
            || value.contains("chat")
            || value.contains("completion")
            || value.contains("language")
        {
            Some(ModelCapability::Text)
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
    if has(&[
        "video", "sora", "veo", "kling", "runway", "hailuo", "seedance", "wan2",
    ]) {
        found.push(ModelCapability::Video);
    }
    if has(&[
        "image",
        "dall-e",
        "dalle",
        "flux",
        "sdxl",
        "stable-diffusion",
        "imagen",
        "seedream",
        "kolors",
    ]) {
        found.push(ModelCapability::Image);
    }
    if found.is_empty()
        || has(&[
            "chat",
            "text",
            "instruct",
            "reasoning",
            "gpt",
            "gemini",
            "claude",
            "qwen",
            "deepseek",
            "llama",
            "mistral",
            "flash",
            "pro",
            "turbo",
        ])
    {
        found.push(ModelCapability::Text);
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
        assert!(models[0].supports(ModelCapability::Text));
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
        assert_eq!(models[0].capabilities, vec![ModelCapability::Text]);
    }
}
