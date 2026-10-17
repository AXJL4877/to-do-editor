//! Presentation-only AI settings model. It carries masked key hints and never
//! exposes a secret value to the UI layer.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Missing,
    Environment,
    Keychain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCapability {
    Text,
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableModelView {
    pub id: String,
    pub capabilities: Vec<ModelCapability>,
}

#[derive(Clone, PartialEq)]
pub struct AiSettingsView {
    pub huniu_base_url: String,
    pub huniu_chat_model: String,
    pub huniu_image_model: String,
    pub huniu_key: Option<String>,
    pub huniu_key_entry: String,
    pub huniu_clear_key: bool,
    pub huniu_key_source: KeySource,
    pub huniu_models: Vec<AvailableModelView>,
    pub huniu_model_status: Option<String>,
    pub volc_base_url: String,
    pub volc_speaker: String,
    pub volc_key: Option<String>,
    pub volc_key_entry: String,
    pub volc_clear_key: bool,
    pub volc_key_source: KeySource,
    pub proxy: ProxyChoice,
}

impl std::fmt::Debug for AiSettingsView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AiSettingsView")
            .field("huniu_base_url", &self.huniu_base_url)
            .field("huniu_chat_model", &self.huniu_chat_model)
            .field("huniu_image_model", &self.huniu_image_model)
            .field("huniu_key", &self.huniu_key)
            .field("huniu_key_entry", &"***")
            .field("huniu_key_source", &self.huniu_key_source)
            .field("huniu_models", &self.huniu_models)
            .field("huniu_model_status", &self.huniu_model_status)
            .field("volc_base_url", &self.volc_base_url)
            .field("volc_speaker", &self.volc_speaker)
            .field("volc_key", &self.volc_key)
            .field("volc_key_entry", &"***")
            .field("volc_key_source", &self.volc_key_source)
            .field("proxy", &self.proxy)
            .finish()
    }
}

impl Default for AiSettingsView {
    fn default() -> Self {
        Self {
            huniu_base_url: "https://huniuai.com".into(),
            huniu_chat_model: "geminiflash3.1".into(),
            huniu_image_model: "gpt-image-2".into(),
            huniu_key: None,
            huniu_key_entry: String::new(),
            huniu_clear_key: false,
            huniu_key_source: KeySource::Missing,
            huniu_models: Vec::new(),
            huniu_model_status: None,
            volc_base_url: "https://openspeech.bytedance.com".into(),
            volc_speaker: "zh_female_mizai_uranus_bigtts".into(),
            volc_key: None,
            volc_key_entry: String::new(),
            volc_clear_key: false,
            volc_key_source: KeySource::Missing,
            proxy: ProxyChoice::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyChoice {
    Auto,
    System,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSettingsError(pub String);

pub fn normalize_base_url(raw: &str) -> Result<String, AiSettingsError> {
    let mut value = raw.trim().trim_end_matches('/').to_owned();
    if value.to_ascii_lowercase().ends_with("/v1") {
        value.truncate(value.len() - 3);
    }
    value = value.trim_end_matches('/').to_owned();
    let host = value.split_once("://").map_or("", |(_, rest)| rest);
    if !(value.starts_with("http://") || value.starts_with("https://"))
        || host.is_empty()
        || host.chars().any(char::is_whitespace)
    {
        return Err(AiSettingsError(
            "服务地址须以 http:// 或 https:// 开头".into(),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_normalization_matches_service_contract() {
        assert_eq!(
            normalize_base_url(" https://gateway.test/v1/ ").unwrap(),
            "https://gateway.test"
        );
        assert!(normalize_base_url("gateway.test").is_err());
        assert!(normalize_base_url("https://").is_err());
    }

    #[test]
    fn debug_does_not_expose_entered_keys() {
        let view = AiSettingsView {
            huniu_key_entry: "secret-value".into(),
            ..AiSettingsView::default()
        };
        assert!(!format!("{view:?}").contains("secret-value"));
    }
}
