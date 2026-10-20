//! Non-secret service settings, owned by the app: `<root>/config/ai.json`
//! (written by the AI settings panel) over built-in defaults. Keys are not
//! here; see [`crate::secrets`].
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const CONFIG_RELATIVE: &str = "config/ai.json";
pub const HUNIU_DEFAULT_URL: &str = "https://huniuai.com";
pub const VOLC_DEFAULT_URL: &str = "https://openspeech.bytedance.com";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

/// `config/ai.json`. Every field is optional; unknown fields are rejected so a
/// typo never silently falls back to a default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default, skip_serializing_if = "HuniuFile::is_empty")]
    pub huniu: HuniuFile,
    /// Optional dedicated endpoint for the language capability (e.g. DeepSeek).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<LanguageFile>,
    #[serde(default, skip_serializing_if = "VolcFile::is_empty")]
    pub volc: VolcFile,
    #[serde(default, skip_serializing_if = "LimitsFile::is_empty")]
    pub limits: LimitsFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyMode>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HuniuFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multimodal_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_size: Option<String>,
    /// Huniu channel routing, for example `XUE` or `price_first`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LanguageFile {
    /// Origin without `/v1`, e.g. `https://api.deepseek.com`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl HuniuFile {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolcFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

impl VolcFile {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LimitsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_inflight: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_inflight: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_rate_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_inflight: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_paid_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_resubmit_sec: Option<u64>,
}

impl LimitsFile {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    /// System proxy, except a loopback one (Clash and similar) → direct.
    Auto,
    System,
    Direct,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HuniuSettings {
    /// Origin without `/v1`, e.g. `https://huniuai.com`.
    pub base_url: String,
    pub language_model: String,
    pub image_model: String,
    pub video_model: String,
    pub multimodal_model: String,
    pub image_size: String,
    pub strategy: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolcSettings {
    pub base_url: String,
    pub audio_model: String,
    pub speaker: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub chat_inflight: usize,
    pub image_inflight: usize,
    pub image_rate_per_second: f64,
    pub speech_inflight: usize,
    /// Extra attempts allowed after an unknown outcome of a billed image request.
    pub image_paid_retries: u32,
    /// An image request still unanswered after this long is sent again (the
    /// earlier send keeps running; the first image back wins).
    pub image_resubmit_after: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiConfig {
    pub huniu: HuniuSettings,
    /// Direct language endpoint; `None` keeps everything on huniu.
    pub language_base_url: Option<String>,
    pub volc: VolcSettings,
    pub limits: Limits,
    pub proxy: ProxyMode,
}

impl AiConfig {
    /// Load `<root>/config/ai.json`; a missing file means defaults.
    pub fn load(root: &Path) -> Result<Self, ConfigError> {
        Self::resolve(Self::load_file(root)?)
    }

    pub fn load_file(root: &Path) -> Result<ConfigFile, ConfigError> {
        let path = root.join(CONFIG_RELATIVE);
        match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|source| ConfigError::Parse { path, source })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
            Err(source) => Err(ConfigError::Read { path, source }),
        }
    }

    /// Validate and write the settings file atomically.
    pub fn save(root: &Path, file: &ConfigFile) -> Result<Self, ConfigError> {
        let resolved = Self::resolve(file.clone())?;
        let path = root.join(CONFIG_RELATIVE);
        let directory = path.parent().expect("config path has a parent");
        let io = |source| ConfigError::Read {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(directory).map_err(io)?;
        let temporary = directory.join(format!(".ai.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(file).expect("config serializes");
        std::fs::write(&temporary, bytes).map_err(io)?;
        std::fs::rename(&temporary, &path).map_err(io)?;
        Ok(resolved)
    }

    pub fn resolve(file: ConfigFile) -> Result<Self, ConfigError> {
        let pick = |explicit: Option<String>, default: &str| {
            explicit
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().to_owned())
                .unwrap_or_else(|| default.to_owned())
        };
        let huniu = HuniuSettings {
            base_url: normalize_base_url(&pick(file.huniu.base_url, HUNIU_DEFAULT_URL))?,
            language_model: pick(file.huniu.language_model, "geminiflash3.1"),
            image_model: pick(file.huniu.image_model, "gpt-image-2"),
            video_model: pick(file.huniu.video_model, "sora-2"),
            multimodal_model: pick(file.huniu.multimodal_model, "gemini-3.8-flash"),
            image_size: pick(file.huniu.image_size, "1024x1024"),
            strategy: Some(pick(file.huniu.strategy, "XUE")).filter(|s| s != "none"),
        };
        let volc = VolcSettings {
            base_url: normalize_base_url(&pick(file.volc.base_url, VOLC_DEFAULT_URL))?,
            audio_model: pick(file.volc.audio_model, "seed-tts-2.0"),
            speaker: pick(file.volc.speaker, "zh_female_mizai_uranus_bigtts"),
        };
        let limits = Limits {
            chat_inflight: file.limits.chat_inflight.unwrap_or(4).clamp(1, 32),
            image_inflight: file.limits.image_inflight.unwrap_or(40).clamp(1, 64),
            image_rate_per_second: file
                .limits
                .image_rate_per_second
                .filter(|rate| rate.is_finite() && *rate > 0.0)
                .unwrap_or(2.0)
                .min(20.0),
            speech_inflight: file.limits.speech_inflight.unwrap_or(2).clamp(1, 16),
            image_paid_retries: file.limits.image_paid_retries.unwrap_or(2).min(10),
            image_resubmit_after: Duration::from_secs(
                file.limits.image_resubmit_sec.unwrap_or(300).clamp(30, 1800),
            ),
        };
        if !valid_size(&huniu.image_size) {
            return Err(ConfigError::Invalid(format!(
                "image size {:?} must be WIDTHxHEIGHT",
                huniu.image_size
            )));
        }
        let language_base_url = match file.language.as_ref().and_then(|l| l.base_url.as_deref()) {
            Some(raw) => Some(normalize_base_url(raw)?),
            None => None,
        };
        Ok(Self {
            huniu,
            language_base_url,
            volc,
            limits,
            proxy: file.proxy.unwrap_or(ProxyMode::Auto),
        })
    }
}

/// `https://host[/v1][/]` → `https://host`; endpoints append their paths.
pub fn normalize_base_url(raw: &str) -> Result<String, ConfigError> {
    let mut url = raw.trim().trim_end_matches('/').to_owned();
    if url.to_ascii_lowercase().ends_with("/v1") {
        url.truncate(url.len() - 3);
    }
    let url = url.trim_end_matches('/').to_owned();
    let host = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || host.is_empty()
        || host.contains(char::is_whitespace)
    {
        return Err(ConfigError::Invalid(format!(
            "服务地址 {raw:?} 须以 http:// 或 https:// 开头"
        )));
    }
    Ok(url)
}

fn valid_size(size: &str) -> bool {
    size.split_once('x').is_some_and(|(w, h)| {
        [w, h]
            .iter()
            .all(|n| n.parse::<u32>().is_ok_and(|n| (64..=8192).contains(&n)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_values_override_built_in_defaults() {
        let file: ConfigFile = serde_json::from_str(
            r#"{"huniu": {"baseUrl": "https://gateway.example/v1/", "imageModel": "gpt-image-2.5-flare"},
                "volc": {"speaker": "S_abc"}, "limits": {"imageInflight": 500}}"#,
        )
        .unwrap();
        let config = AiConfig::resolve(file).unwrap();
        assert_eq!(config.huniu.base_url, "https://gateway.example");
        assert_eq!(config.huniu.image_model, "gpt-image-2.5-flare");
        assert_eq!(config.huniu.language_model, "geminiflash3.1");
        assert_eq!(config.huniu.strategy.as_deref(), Some("XUE"));
        assert_eq!(config.volc.base_url, VOLC_DEFAULT_URL);
        assert_eq!(config.volc.speaker, "S_abc");
        assert_eq!(config.limits.image_inflight, 64);
        assert_eq!(config.proxy, ProxyMode::Auto);
    }

    #[test]
    fn unknown_fields_and_bad_values_are_rejected() {
        assert!(serde_json::from_str::<ConfigFile>(r#"{"huniu": {"imageModle": "x"}}"#).is_err());
        assert!(serde_json::from_str::<ConfigFile>(r#"{"mishiEnv": "D:/x/.env"}"#).is_err());
        let file: ConfigFile = serde_json::from_str(r#"{"huniu": {"imageSize": "big"}}"#).unwrap();
        assert!(AiConfig::resolve(file).is_err());
        assert!(normalize_base_url("huniuai.com").is_err());
        assert!(normalize_base_url("https://").is_err());
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/ai-config-tests")
            .join(unique.to_string());
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            AiConfig::load(&root).unwrap().huniu.base_url,
            HUNIU_DEFAULT_URL
        );
        let mut file = ConfigFile::default();
        file.huniu.base_url = Some("https://my-gateway.example".into());
        file.volc.audio_model = Some("seed-icl-2.0".into());
        AiConfig::save(&root, &file).unwrap();
        assert_eq!(AiConfig::load_file(&root).unwrap(), file);
        assert_eq!(
            AiConfig::load(&root).unwrap().huniu.base_url,
            "https://my-gateway.example"
        );
        file.huniu.base_url = Some("not a url".into());
        assert!(
            AiConfig::save(&root, &file).is_err(),
            "invalid settings are not written"
        );
        assert_eq!(
            AiConfig::load(&root).unwrap().huniu.base_url,
            "https://my-gateway.example"
        );
    }
}
