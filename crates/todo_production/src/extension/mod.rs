//! The extension standard: manifest (`todo-extension/1`), worker protocol
//! (`todo-worker/1`), timeline items (`todo-items/1`) and optional workflow
//! graph IR (`todo-graph/1`). Normative text: `docs/architecture/extension-spec.md`.
pub mod graph;
pub mod items;
pub mod manifest;
pub mod nodes;
pub mod protocol;
pub mod services;

use crate::pacing::Pacing;

/// `WIDTHxHEIGHT`, each side 64..=8192.
pub fn valid_size(size: &str) -> bool {
    size.split_once('x').is_some_and(|(w, h)| {
        [w, h]
            .iter()
            .all(|n| n.parse::<u32>().is_ok_and(|n| (64..=8192).contains(&n)))
    })
}

/// Pacing overrides from a manifest, on top of the defaults.
pub fn pacing_from(value: &serde_json::Value) -> Result<Pacing, String> {
    let overrides = value.as_object().ok_or("须为对象")?;
    let mut merged = serde_json::to_value(Pacing::default()).expect("pacing serializes");
    let base = merged.as_object_mut().expect("pacing is an object");
    for (key, v) in overrides {
        if !base.contains_key(key) {
            return Err(format!("未知字段 {key}"));
        }
        if !v
            .as_f64()
            .is_some_and(|x| x.is_finite() && (0.0..=30.0).contains(&x))
        {
            return Err(format!("{key} 须为 0~30 的秒数或比例"));
        }
        base.insert(key.clone(), v.clone());
    }
    serde_json::from_value(merged).map_err(|e| e.to_string())
}

/// Host defaults a manifest cannot choose (models come from the user's configuration).
#[derive(Debug, Clone, PartialEq)]
pub struct HostDefaults {
    pub audio_model: String,
    pub sample_rate: u32,
}

/// Recipe settings of a `narration` extension with resolved `params`:
/// declared well-known parameters override the manifest's values.
pub fn narration_spec(
    manifest: &manifest::Manifest,
    params: &std::collections::BTreeMap<String, serde_json::Value>,
    read: &dyn Fn(&str) -> Option<String>,
    host: &HostDefaults,
) -> Result<
    (
        crate::work::ProductionSpec,
        crate::production::SplitMode,
        crate::segment::Limits,
    ),
    String,
> {
    use crate::production::SplitMode;
    let n = manifest.narration.as_ref().ok_or("不是口播配方扩展")?;
    let number = |id: &str| params.get(id).and_then(serde_json::Value::as_f64);
    let text = |path: &str| {
        read(path)
            .map(|t| t.trim().to_owned())
            .ok_or_else(|| format!("找不到 {path}"))
    };
    let speaker = params
        .get("voice")
        .and_then(|v| v.as_str())
        .ok_or("缺少音色参数 voice")?
        .to_owned();
    // Cloned voices (S_…) use the voice cloning resource.
    let resource = services::speech_resource_id(&speaker, &host.audio_model);
    let split = match params.get("split").and_then(|v| v.as_str()) {
        Some("paragraph") => SplitMode::Paragraph,
        Some("length") => SplitMode::Length,
        _ => SplitMode::Semantic,
    };
    let target = number("segmentChars").map_or(n.segment.target, |v| v as usize);
    let limits = crate::segment::Limits {
        target,
        max: n.segment.max.max(target),
    };
    let subtitles_on = params
        .get("subtitles")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(n.subtitles.is_some());
    let subtitle_chars = subtitles_on.then(|| {
        number("subtitleChars").map_or_else(
            || n.subtitles.as_ref().map_or(18, |s| s.max_chars),
            |v| v as usize,
        )
    });
    let pacing = n
        .pacing
        .as_ref()
        .map(pacing_from)
        .transpose()?
        .unwrap_or_default();
    let spec = crate::work::ProductionSpec {
        planner_system: text(&n.planner.prompt)?,
        max_elements: n.planner.max_elements,
        board_sec: number("boardSec").unwrap_or(n.board_sec),
        variants: number("variants").map_or(1, |v| v.max(1.0) as usize),
        asset: crate::work::AssetSpec {
            template: text(&n.assets.template)?,
            roles: n.assets.roles.clone(),
            avoid: n.assets.avoid.clone(),
            vars: manifest.prompt_vars(params),
            size: n.assets.size.clone(),
            quality: n.assets.quality.clone(),
            transparent: n.assets.transparent,
        },
        voice: crate::work::VoiceSpec {
            speaker,
            resource_id: resource,
            sample_rate: host.sample_rate,
            speech_rate: number("speech_rate").map(|value| value as i32),
        },
        subtitle_chars,
        pacing,
        tail: number("tail").unwrap_or(0.0),
    };
    Ok((spec, split, limits))
}
