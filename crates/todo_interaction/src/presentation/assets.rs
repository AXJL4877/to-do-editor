//! The asset panels beside the media library: the motion library (every
//! preset with a short animation to preview) and the font library (every font
//! file and every text style, with rendered samples). The app loads the
//! libraries from disk; this module only shapes them for the panel.
use crate::input::Intent;
use crate::inspector::{ClipProperties, MotionSlot};
use todo_core::motion::{LibraryEntry, MotionKind, MotionPreset, MotionSource};
use todo_core::{builtin_text_styles, MotionRef, Project, TextStyle};

/// Poses per preview animation (plus the resting end).
pub const POSE_SAMPLES: usize = 32;

/// Sample text of the font previews.
pub const FONT_SAMPLE: &str = "海绵宝宝 诱饵效应 Aa123";
/// Sample text of the text-style previews.
pub const STYLE_SAMPLE: &str = "花字样式";

/// One evaluated moment of a motion: multiply opacity and size, add the
/// offset (canvas fractions).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionPose {
    pub opacity: f32,
    pub dx: f32,
    pub dy: f32,
    pub scale_x: f32,
    pub scale_y: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotionOrigin {
    Builtin,
    /// The user's own library (`motions/`).
    User,
    /// Shipped by this extension.
    Extension(String),
}

impl MotionOrigin {
    pub fn label(&self) -> String {
        match self {
            Self::Builtin => "内置".into(),
            Self::User => "我的".into(),
            Self::Extension(_) => "扩展".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionCard {
    pub id: String,
    pub name: String,
    pub kind: MotionKind,
    pub origin: MotionOrigin,
    pub duration: f64,
    pub description: String,
    pub tags: Vec<String>,
    /// The animated clip over the window (for a transition, the incoming one).
    pub poses: Vec<MotionPose>,
    /// Transition only: the outgoing clip.
    pub outgoing: Option<Vec<MotionPose>>,
    pub outgoing_on_top: bool,
    /// The selected clip uses this preset.
    pub in_use: bool,
}

/// The clip a click on a motion applies to.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionTarget {
    pub clip_id: String,
    pub name: String,
    pub can_transition: bool,
}

/// A font file as the app found it on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct FontInput {
    pub file: String,
    /// The family the file declares.
    pub family: Option<String>,
    /// From the font records (`fonts.json`); `None` when not on record.
    pub name: Option<String>,
    pub source: String,
    /// Empty: the licence is not on record.
    pub license: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontCard {
    pub file: String,
    pub name: String,
    pub family: Option<String>,
    pub source: String,
    pub license: String,
    pub note: String,
    /// Thumbnail key of the rendered sample.
    pub preview: String,
    /// Names of the text styles set in this font (first choice, not fallback).
    pub styles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyleCard {
    pub id: String,
    pub name: String,
    pub font_file: String,
    /// The font file is not in the library: the style renders with a fallback.
    pub font_missing: bool,
    pub preview: String,
    /// Carried by the open project (else a built-in).
    pub in_project: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssetsView {
    pub motions: Vec<MotionCard>,
    pub target: Option<MotionTarget>,
    pub fonts: Vec<FontCard>,
    pub styles: Vec<StyleCard>,
    pub warnings: Vec<String>,
    /// Where the user's own motions and the fonts live (shown, and opened).
    pub motions_dir: String,
    pub fonts_dir: String,
}

pub fn font_preview_key(file: &str) -> String {
    format!("asset:font/{file}")
}

pub fn style_preview_key(id: &str) -> String {
    format!("asset:style/{id}")
}

/// Text styles a panel shows: the project's own, then the built-ins it does
/// not override.
pub fn panel_styles(project: Option<&Project>) -> Vec<(TextStyle, bool)> {
    let mut styles: Vec<(TextStyle, bool)> = project
        .map(|p| p.text_styles.iter().cloned().map(|s| (s, true)).collect())
        .unwrap_or_default();
    for style in builtin_text_styles() {
        if !styles.iter().any(|(s, _)| s.id == style.id) {
            styles.push((style, false));
        }
    }
    styles
}

pub struct AssetsInput<'a> {
    pub motions: &'a [LibraryEntry],
    pub fonts: Vec<FontInput>,
    pub project: Option<&'a Project>,
    pub selected: Option<&'a ClipProperties>,
    pub warnings: Vec<String>,
    pub motions_dir: String,
    pub fonts_dir: String,
}

pub fn assets_view(input: AssetsInput<'_>) -> AssetsView {
    let target = input.selected.and_then(|props| {
        let motion = props.motion.as_ref()?;
        Some(MotionTarget {
            clip_id: props.clip_id.clone(),
            name: props.name.clone(),
            can_transition: motion.can_transition,
        })
    });
    let used: Vec<&str> = input
        .selected
        .and_then(|props| props.motion.as_ref())
        .map(|m| {
            [&m.intro, &m.outro, &m.transition]
                .into_iter()
                .flatten()
                .map(|r| r.kind.as_str())
                .collect()
        })
        .unwrap_or_default();
    let mut motions: Vec<MotionCard> = input
        .motions
        .iter()
        .map(|entry| motion_card(entry, used.contains(&entry.preset.id.as_str())))
        .collect();
    motions.sort_by_key(|card| kind_rank(card.kind));
    let styles = panel_styles(input.project);
    let fonts = input
        .fonts
        .into_iter()
        .map(|font| FontCard {
            preview: font_preview_key(&font.file),
            name: font
                .name
                .clone()
                .or_else(|| font.family.clone())
                .unwrap_or_else(|| font.file.clone()),
            styles: styles
                .iter()
                .filter(|(s, _)| s.font.file == font.file)
                .map(|(s, _)| s.name.clone())
                .collect(),
            file: font.file,
            family: font.family,
            source: font.source,
            license: font.license,
            note: font.note,
        })
        .collect::<Vec<_>>();
    let styles = styles
        .into_iter()
        .map(|(style, in_project)| StyleCard {
            preview: style_preview_key(&style.id),
            font_missing: !fonts.iter().any(|f| f.file == style.font.file),
            font_file: style.font.file,
            id: style.id,
            name: style.name,
            in_project,
        })
        .collect();
    AssetsView {
        motions,
        target,
        fonts,
        styles,
        warnings: input.warnings,
        motions_dir: input.motions_dir,
        fonts_dir: input.fonts_dir,
    }
}

fn kind_rank(kind: MotionKind) -> u8 {
    match kind {
        MotionKind::Intro => 0,
        MotionKind::Outro => 1,
        MotionKind::Transition => 2,
    }
}

fn poses(sample: impl Fn(f64) -> todo_core::motion::MotionSample) -> Vec<MotionPose> {
    (0..=POSE_SAMPLES)
        .map(|i| {
            let s = sample(i as f64 / POSE_SAMPLES as f64);
            MotionPose {
                opacity: s.opacity as f32,
                dx: s.dx as f32,
                dy: s.dy as f32,
                scale_x: (s.scale * s.scale_x) as f32,
                scale_y: (s.scale * s.scale_y) as f32,
            }
        })
        .collect()
}

fn motion_card(entry: &LibraryEntry, in_use: bool) -> MotionCard {
    let preset: &MotionPreset = &entry.preset;
    MotionCard {
        id: preset.id.clone(),
        name: preset.name.clone(),
        kind: preset.kind,
        origin: match &entry.source {
            MotionSource::Builtin => MotionOrigin::Builtin,
            MotionSource::User { .. } => MotionOrigin::User,
            MotionSource::Extension { id } => MotionOrigin::Extension(id.clone()),
        },
        duration: preset.duration,
        description: preset.description.clone(),
        tags: preset.tags.clone(),
        poses: poses(|u| preset.sample(u)),
        outgoing: (preset.kind == MotionKind::Transition).then(|| poses(|u| preset.sample_outgoing(u))),
        outgoing_on_top: preset.outgoing_on_top,
        in_use,
    }
}

impl MotionCard {
    /// The edit a click on this card makes to `target` (none for a
    /// transition when nothing ends right where the clip starts).
    pub fn apply(&self, target: &MotionTarget) -> Option<Intent> {
        let slot = match self.kind {
            MotionKind::Intro => MotionSlot::Intro,
            MotionKind::Outro => MotionSlot::Outro,
            MotionKind::Transition if target.can_transition => MotionSlot::Transition,
            MotionKind::Transition => return None,
        };
        Some(Intent::SetClipMotion {
            clip_id: target.clip_id.clone(),
            slot,
            motion: Some(MotionRef {
                kind: self.id.clone(),
                duration: self.duration,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::motion::MotionLibrary;

    #[test]
    fn every_motion_animates_to_rest_and_fonts_list_their_styles() {
        let library = MotionLibrary::builtin();
        let view = assets_view(AssetsInput {
            motions: library.entries(),
            fonts: vec![FontInput {
                file: "zcool-kuaile.ttf".into(),
                family: Some("站酷快乐体".into()),
                name: None,
                source: String::new(),
                license: String::new(),
                note: String::new(),
            }],
            project: None,
            selected: None,
            warnings: Vec::new(),
            motions_dir: String::new(),
            fonts_dir: String::new(),
        });
        assert_eq!(view.motions.len(), library.entries().len());
        for card in &view.motions {
            assert_eq!(card.poses.len(), POSE_SAMPLES + 1);
            let rest = card.poses.last().unwrap();
            assert!((rest.opacity - 1.0).abs() < 1e-3 || card.kind == MotionKind::Outro, "{}", card.id);
            assert_eq!(card.outgoing.is_some(), card.kind == MotionKind::Transition);
        }
        let kuaile = &view.fonts[0];
        assert_eq!(kuaile.name, "站酷快乐体");
        assert!(!kuaile.styles.is_empty(), "subtitle-kuaile is set in it");
        assert!(view.styles.iter().any(|s| s.id == "title-red-brush" && s.font_missing));
        let target = MotionTarget {
            clip_id: "c".into(),
            name: "c".into(),
            can_transition: false,
        };
        let dissolve = view.motions.iter().find(|m| m.id == "dissolve").unwrap();
        assert!(dissolve.apply(&target).is_none(), "no adjacent clip: no transition");
        let pop = view.motions.iter().find(|m| m.id == "pop").unwrap();
        assert!(matches!(pop.apply(&target), Some(Intent::SetClipMotion { slot: MotionSlot::Intro, .. })));
    }
}
