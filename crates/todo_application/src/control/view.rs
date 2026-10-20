//! Read-side view of a project for automations: every element with its timing,
//! placement, on-canvas rectangle in pixels and resolved text metrics.
//!
//! This is the counterpart of [`super::ControlEdit`]: edits write, this reads.
//! The raw [`Project`] is enough for a machine that knows the units; an agent
//! needs the derived geometry — where the picture actually lands, how big it is,
//! and what is visible at a given second. Pictures are exact (aspect-fit plus
//! transform); text boxes are estimated from the font size and are marked so.
use serde::{Deserialize, Serialize};
use todo_core::{
    content_duration, hit_transform, media_kind, transition_is_active, visible_hits, BaseSize,
    ClipSource, MediaKind, MotionRef, Project, TrackKind, Transform,
};

pub const VIEW_API_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Size {
    /// Canvas fractions (0..1).
    pub width: f64,
    pub height: f64,
}

/// Pixels on the program canvas, origin at the top-left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The same rectangle as canvas fractions, for prompts that speak in %.
    pub nx: f64,
    pub ny: f64,
    pub nw: f64,
    pub nh: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasView {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackView {
    pub id: String,
    pub name: String,
    pub kind: TrackKind,
    pub muted: bool,
    pub locked: bool,
    /// 0 is the bottom-most track (drawn first).
    pub z: usize,
    pub clips: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextView {
    pub content: String,
    pub style_id: String,
    /// Font size in canvas pixels (`layout.fontSize × canvas height`).
    pub font_px: f64,
    pub align: String,
    pub chars: usize,
    /// Text boxes are estimated from the font size, not measured by the
    /// rasterizer; picture rectangles are exact.
    pub estimated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementView {
    pub clip_id: String,
    pub track_id: String,
    /// 0 is bottom-most; a later element covers an earlier one.
    pub z: usize,
    /// `video` / `image` / `audio` / `text`.
    pub kind: String,
    pub name: String,
    pub start: f64,
    pub duration: f64,
    pub end: f64,
    pub source_offset: f64,
    pub rate: f64,
    /// Canvas fractions: offsets from the canvas centre, scale multipliers.
    pub transform: Transform,
    /// Placement after entrance/exit curves, when `at` falls inside the clip.
    pub effective: Option<Transform>,
    /// Where the element lands, in pixels and in canvas fractions.
    pub rect: Option<Rect>,
    pub text: Option<TextView>,
    /// Whether the element is on screen at `at` (pictures and text only).
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<MotionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outro: Option<MotionRef>,
    /// Transition into this element from its same-track predecessor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<TransitionView>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionView {
    pub kind: String,
    pub duration: f64,
    /// False while the predecessor is not adjacent or the preset is unknown.
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub api_version: u32,
    pub canvas: CanvasView,
    /// Workspace length, when the project pins one.
    pub duration: Option<f64>,
    pub content_duration: f64,
    pub playhead: f64,
    /// The second the view was evaluated at, when requested.
    pub at: Option<f64>,
    /// Top track first, matching the timeline.
    pub tracks: Vec<TrackView>,
    /// Sorted by `z`, then by start time.
    pub elements: Vec<ElementView>,
}

fn rect_of(transform: &Transform, base: Size, canvas: CanvasView) -> Option<Rect> {
    let bounds = transform.bounds_for_base(BaseSize {
        width: base.width,
        height: base.height,
    })?;
    let width = f64::from(canvas.width);
    let height = f64::from(canvas.height);
    Some(Rect {
        x: bounds.left * width,
        y: bounds.top * height,
        width: bounds.width() * width,
        height: bounds.height() * height,
        nx: bounds.left,
        ny: bounds.top,
        nw: bounds.width(),
        nh: bounds.height(),
    })
}

/// Aspect-fit base size of a media item, as canvas fractions.
fn media_base(project: &Project, media_id: &str) -> (Option<Size>, Vec<String>) {
    let Some(media) = project.media.iter().find(|item| item.id == media_id) else {
        return (None, vec!["media-unknown".into()]);
    };
    match (media.width, media.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let canvas = (f64::from(project.width), f64::from(project.height));
            match BaseSize::fit(f64::from(width) / f64::from(height), canvas) {
                Some(base) => (
                    Some(Size {
                        width: base.width,
                        height: base.height,
                    }),
                    Vec::new(),
                ),
                None => (None, vec!["canvas-invalid".into()]),
            }
        }
        _ => (None, vec!["media-size-unknown".into()]),
    }
}

/// Text rasters cover the whole canvas, so a text clip is measured as its block:
/// one line of `font_px` per line of content, capped to the canvas width.
fn text_block(text: &str, font_px: f64, line_height: f64, canvas: (f64, f64)) -> Size {
    let lines = text.lines().count().max(1) as f64;
    let longest = text
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as f64;
    let width = (longest * font_px).min(canvas.0).max(1.0);
    let height = (font_px * line_height * lines).min(canvas.1).max(1.0);
    Size {
        width: width / canvas.0,
        height: height / canvas.1,
    }
}

fn kind_label(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Image => "image",
    }
}

pub fn project_view(project: &Project, at: Option<f64>) -> ProjectView {
    let canvas = CanvasView {
        width: project.width,
        height: project.height,
        fps: project.fps,
    };
    let canvas_size = (f64::from(project.width), f64::from(project.height));
    let total = project.tracks.len();
    let mut tracks = Vec::with_capacity(total);
    let mut elements = Vec::new();
    // What preview and export draw at `at`: one rule for visibility and motion.
    let hits = at
        .map(|time| visible_hits(project, time))
        .unwrap_or_default();

    for (index, track) in project.tracks.iter().enumerate() {
        // The document stores tracks top-first; `z` counts from the bottom.
        let z = total - index - 1;
        tracks.push(TrackView {
            id: track.id.clone(),
            name: track.name.clone(),
            kind: track.kind,
            muted: track.muted,
            locked: track.locked,
            z,
            clips: track.clips.len(),
        });
        for clip in &track.clips {
            let mut warnings = Vec::new();
            let (base, kind, name, text) = match &clip.source {
                ClipSource::Media(source) => {
                    let (base, mut notes) = media_base(project, &source.media_id);
                    warnings.append(&mut notes);
                    let media = project.media.iter().find(|item| item.id == source.media_id);
                    let media_kind = media.map(media_kind).unwrap_or(MediaKind::Video);
                    (
                        base,
                        kind_label(media_kind).to_string(),
                        media
                            .map(|item| item.name.clone())
                            .unwrap_or_else(|| source.media_id.clone()),
                        None,
                    )
                }
                ClipSource::Text(content) => {
                    if !project
                        .text_styles
                        .iter()
                        .any(|style| style.id == content.style_id)
                    {
                        warnings.push("text-style-missing".into());
                    }
                    let font_px = content.layout.font_size * canvas_size.1;
                    (
                        Some(text_block(
                            &content.text,
                            font_px,
                            content.layout.line_height,
                            canvas_size,
                        )),
                        "text".to_string(),
                        content.text.clone(),
                        Some(TextView {
                            content: content.text.clone(),
                            style_id: content.style_id.clone(),
                            font_px,
                            align: format!("{:?}", content.layout.align).to_lowercase(),
                            chars: content.text.chars().count(),
                            estimated: true,
                        }),
                    )
                }
            };

            let hit = hits
                .iter()
                .find(|hit| hit.clip.id == clip.id && hit.track_id == track.id);
            let inside = hit.is_some();
            let effective = hit.map(|hit| hit_transform(project, hit));
            let placement = effective.unwrap_or(clip.transform);
            let rect = base.and_then(|base| rect_of(&placement, base, canvas));
            let visible = match &clip.source {
                ClipSource::Media(source) => {
                    let audio_only = project
                        .media
                        .iter()
                        .find(|item| item.id == source.media_id)
                        .is_some_and(|item| media_kind(item) == MediaKind::Audio);
                    (!audio_only).then_some(inside)
                }
                ClipSource::Text(_) => Some(inside),
            };
            elements.push(ElementView {
                clip_id: clip.id.clone(),
                track_id: track.id.clone(),
                z,
                kind,
                name,
                start: clip.start,
                duration: clip.duration,
                end: clip.start + clip.duration,
                source_offset: clip.media().map_or(0.0, |media| media.offset),
                rate: clip.media().map_or(1.0, |media| media.rate),
                transform: clip.transform,
                effective,
                rect,
                text,
                visible,
                intro: clip.intro.clone(),
                outro: clip.outro.clone(),
                transition: clip.transition.as_ref().map(|motion| TransitionView {
                    kind: motion.kind.clone(),
                    duration: motion.duration,
                    active: transition_is_active(project, track, &clip.id),
                }),
                warnings,
            });
        }
    }
    elements.sort_by(|left, right| {
        left.z
            .cmp(&right.z)
            .then(left.start.total_cmp(&right.start))
    });

    ProjectView {
        api_version: VIEW_API_VERSION,
        canvas,
        duration: project.duration,
        content_duration: content_duration(project),
        playhead: project.playhead,
        at,
        tracks,
        elements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::{add_clip, create_project, normalize_clip, Intro, MediaItem};

    fn project_with_media(width: u32, height: u32) -> Project {
        let mut project = create_project(None, None, None, None, None);
        project.width = 1920;
        project.height = 1080;
        project.media.push(MediaItem {
            id: "m1".into(),
            name: "plate.png".into(),
            path: Some("plate.png".into()),
            duration: 10.0,
            has_audio: Some(false),
            width: Some(width),
            height: Some(height),
            kind: Some(MediaKind::Image),
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        project
    }

    #[test]
    fn full_frame_picture_covers_the_canvas() {
        let project = project_with_media(1920, 1080);
        let project = add_clip(
            project,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 0.0, 4.0, 0.0, 1.0),
        );
        let view = project_view(&project, None);
        let element = view
            .elements
            .iter()
            .find(|item| item.clip_id == "c1")
            .unwrap();
        let rect = element.rect.unwrap();
        assert!((rect.width - 1920.0).abs() < 0.01, "{rect:?}");
        assert!((rect.height - 1080.0).abs() < 0.01, "{rect:?}");
        assert!(rect.x.abs() < 0.01 && rect.y.abs() < 0.01, "{rect:?}");
    }

    #[test]
    fn half_scale_picture_is_centered_and_half_size() {
        let project = project_with_media(1920, 1080);
        let mut project = add_clip(
            project,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 0.0, 4.0, 0.0, 1.0),
        );
        for track in &mut project.tracks {
            for clip in &mut track.clips {
                clip.transform.scale = 0.5;
            }
        }
        let view = project_view(&project, None);
        let element = view
            .elements
            .iter()
            .find(|item| item.clip_id == "c1")
            .unwrap();
        let rect = element.rect.unwrap();
        assert!((rect.width - 960.0).abs() < 0.01, "{rect:?}");
        assert!((rect.x - 480.0).abs() < 0.01, "{rect:?}");
        assert!((rect.y - 270.0).abs() < 0.01, "{rect:?}");
    }

    #[test]
    fn evaluation_applies_the_entrance_curve() {
        let project = project_with_media(1920, 1080);
        let mut project = add_clip(
            project,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 0.0, 4.0, 0.0, 1.0),
        );
        for track in &mut project.tracks {
            for clip in &mut track.clips {
                clip.intro = Some(Intro::new("fade", 1.0));
            }
        }
        let view = project_view(&project, Some(0.0));
        let element = view
            .elements
            .iter()
            .find(|item| item.clip_id == "c1")
            .unwrap();
        assert_eq!(element.visible, Some(true));
        assert!(element.effective.unwrap().opacity < 0.01);
    }
}
