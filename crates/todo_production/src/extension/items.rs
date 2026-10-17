//! `todo-items/1`: what an extension puts on the timeline. Stable keys and
//! content revisions let the host place results progressively, replace them
//! in place and respect the user's deletions and moves.
use super::manifest::valid_relative_path;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemRole {
    Voice,
    Music,
    Sfx,
    Visual,
    Text,
    Subtitle,
}

impl ItemRole {
    pub fn is_audio(self) -> bool {
        matches!(self, Self::Voice | Self::Music | Self::Sfx)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ItemSource {
    /// A file inside the worker's workdir (relative path).
    File {
        file: String,
        /// Source time at the item start, seconds.
        #[serde(default)]
        r#in: f64,
    },
    /// Editable styled text (the editor's `TextContent`).
    Text { content: todo_core::TextContent },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemTransform {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default = "unit")]
    pub scale: f64,
    #[serde(default = "unit")]
    pub scale_x: f64,
    #[serde(default = "unit")]
    pub scale_y: f64,
    #[serde(default = "unit")]
    pub opacity: f64,
}

fn unit() -> f64 {
    1.0
}

impl Default for ItemTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
            opacity: 1.0,
        }
    }
}

pub use todo_core::{Intro, IntroKind};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Item {
    /// Stable across reruns for the same logical product, e.g. `s3/e2/draw`.
    pub key: String,
    /// Changes when the content changes.
    pub rev: String,
    pub role: ItemRole,
    /// Stacking within the role, 0 at the bottom.
    #[serde(default)]
    pub lane: u32,
    /// Seconds from the production origin.
    pub start: f64,
    pub duration: f64,
    pub source: ItemSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<ItemTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<Intro>,
    /// Moves with the item of this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade: Option<ItemFade>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemFade {
    #[serde(default)]
    pub r#in: f64,
    #[serde(default)]
    pub out: f64,
}

pub const MAX_LANES: u32 = 16;
/// Six hours.
pub const MAX_TIME: f64 = 6.0 * 3600.0;

fn valid_key(key: &str) -> bool {
    (1..=120).contains(&key.len())
        && key.split('/').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        })
}

/// Check a batch of items (and the items already emitted in this run, for
/// key uniqueness and links). `exists` tells whether a workdir file exists.
pub fn validate_items(
    items: &[Item],
    earlier: &BTreeSet<String>,
    exists: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    let mut keys = earlier.clone();
    for item in items {
        let at = &item.key;
        if !valid_key(at) {
            return Err(format!(
                "条目 key {at:?} 不合法（字母数字 _ - . 与 / 分段）"
            ));
        }
        if !keys.insert(item.key.clone()) {
            return Err(format!("条目 {at} 重复"));
        }
        if item.rev.is_empty() || item.rev.len() > 128 {
            return Err(format!("条目 {at} 缺少 rev"));
        }
        if !(item.start.is_finite() && item.start >= 0.0 && item.start <= MAX_TIME) {
            return Err(format!("条目 {at} 的 start 不合法"));
        }
        if !(item.duration.is_finite() && item.duration > 0.0 && item.duration <= MAX_TIME) {
            return Err(format!("条目 {at} 的 duration 不合法"));
        }
        if item.lane >= MAX_LANES {
            return Err(format!("条目 {at} 的 lane 超过 {MAX_LANES}"));
        }
        match &item.source {
            ItemSource::File { file, r#in } => {
                if !valid_relative_path(file) || !exists(file) {
                    return Err(format!("条目 {at} 的文件 {file:?} 不存在或不在工作目录内"));
                }
                if !(r#in.is_finite() && *r#in >= 0.0) {
                    return Err(format!("条目 {at} 的 in 不合法"));
                }
            }
            ItemSource::Text { .. } => {
                if item.role.is_audio() {
                    return Err(format!("条目 {at} 是文字却放在音频角色"));
                }
            }
        }
        if let Some(t) = item.transform {
            let values = [t.x, t.y, t.scale, t.scale_x, t.scale_y, t.opacity];
            if values.iter().any(|v| !v.is_finite())
                || t.scale <= 0.0
                || t.scale_x <= 0.0
                || t.scale_y <= 0.0
                || !(0.0..=1.0).contains(&t.opacity)
            {
                return Err(format!("条目 {at} 的 transform 不合法"));
            }
            if item.role.is_audio() {
                return Err(format!("音频条目 {at} 不能有 transform"));
            }
        }
        if let Some(intro) = item.intro {
            if !(intro.duration.is_finite()
                && intro.duration > 0.0
                && intro.duration <= item.duration)
            {
                return Err(format!("条目 {at} 的 intro 时长不合法"));
            }
        }
        if let Some(gain) = item.gain {
            if !(gain.is_finite() && (0.0..=4.0).contains(&gain)) || !item.role.is_audio() {
                return Err(format!("条目 {at} 的 gain 不合法"));
            }
        }
        if let Some(fade) = item.fade {
            // Pictures fade in through `intro`; their `fade` is the fade-out.
            if (!item.role.is_audio() && fade.r#in != 0.0)
                || !fade.r#in.is_finite()
                || !fade.out.is_finite()
                || fade.r#in < 0.0
                || fade.out < 0.0
                || fade.r#in + fade.out > item.duration
            {
                return Err(format!("条目 {at} 的 fade 不合法"));
            }
        }
    }
    for item in items {
        if let Some(link) = &item.link {
            if link == &item.key || !keys.contains(link) {
                return Err(format!("条目 {} 链接的 {link} 不存在", item.key));
            }
        }
    }
    Ok(())
}
