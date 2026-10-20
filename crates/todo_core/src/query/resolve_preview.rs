use crate::model::clip::{clip_end, Clip, ClipSource};
use crate::model::effects::{apply_motion, find_motion, picture_transform, VisibleSpan};
use crate::model::media::{is_video_media, MediaItem};
use crate::model::text::{TextFrame, TextStyle};
use crate::model::track::{Track, TrackKind};
use crate::model::Project;
use crate::{MotionKind, MotionPreset, Transform};

/// Clips closer than this are adjacent: a transition between them plays.
pub const ADJACENT_EPSILON: f64 = 1e-4;

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewHit {
    pub track_id: String,
    pub track_kind: TrackKind,
    pub clip: Clip,
    pub source: HitSource,
    /// Intrinsic picture size in pixels. Text is composed on a canvas-sized frame.
    pub source_size: Option<(u32, u32)>,
    pub timeline_time: f64,
    /// Seconds from the clip start; negative while a transition shows the
    /// clip early, past `duration` while one keeps it after its end.
    pub local_time: f64,
    /// Extra visibility transitions give this clip around its own span.
    pub span: VisibleSpan,
    /// Set while the hit takes part in a transition.
    pub transition: Option<TransitionPhase>,
}

/// Where a hit is inside a transition window.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionPhase {
    /// Preset id.
    pub kind: String,
    /// 0..1 across the window, which is centred on the cut.
    pub progress: f64,
    /// The clip being cut to (`false`: the one being left).
    pub incoming: bool,
    /// Effective window length after clamping to both clips.
    pub duration: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HitSource {
    Media { media: MediaItem, media_time: f64 },
    Text { style: TextStyle, frame: TextFrame },
}

impl PreviewHit {
    pub fn media(&self) -> Option<&MediaItem> {
        match &self.source {
            HitSource::Media { media, .. } => Some(media),
            HitSource::Text { .. } => None,
        }
    }

    pub fn media_time(&self) -> Option<f64> {
        match self.source {
            HitSource::Media { media_time, .. } => Some(media_time),
            HitSource::Text { .. } => None,
        }
    }

    /// Library name for media, the text itself for text clips.
    pub fn display_name(&self) -> String {
        clip_display_name(&self.clip, self.media())
    }
}

/// One naming rule for timeline, inspector and preview captions.
pub fn clip_display_name(clip: &Clip, media: Option<&MediaItem>) -> String {
    match (&clip.source, media) {
        (ClipSource::Text(content), _) => {
            let line = content.text.lines().next().unwrap_or_default().trim();
            if line.is_empty() {
                "文字".into()
            } else {
                line.into()
            }
        }
        (ClipSource::Media(_), Some(media)) => media.name.clone(),
        (ClipSource::Media(_), None) => "clip".into(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAudioClip {
    pub id: String,
    pub path: String,
    pub start: f64,
    pub duration: f64,
    pub offset: f64,
    pub rate: f64,
    pub gain: f64,
    pub fade_in: f64,
    pub fade_out: f64,
}

pub fn resolve_audio_clips(project: &Project) -> Vec<ResolvedAudioClip> {
    let media_by_id: std::collections::HashMap<&str, &MediaItem> =
        project.media.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut clips = Vec::new();
    for track in &project.tracks {
        if track.muted || (track.kind != TrackKind::Audio && track.kind != TrackKind::Sfx) {
            continue;
        }
        for clip in &track.clips {
            let Some(source) = clip.media() else {
                continue;
            };
            let Some(media) = media_by_id.get(source.media_id.as_str()) else {
                continue;
            };
            if media.has_audio == Some(false) {
                continue;
            }
            let Some(path) = media.path.as_ref().filter(|p| !p.trim().is_empty()) else {
                continue;
            };
            if !clip.start.is_finite()
                || clip.start < 0.0
                || !clip.duration.is_finite()
                || clip.duration <= 0.0
                || !source.offset.is_finite()
                || source.offset < 0.0
                || !source.rate.is_finite()
                || source.rate <= 0.0
                || !(clip.start + clip.duration).is_finite()
                || !(source.offset + clip.duration * source.rate).is_finite()
            {
                continue;
            }
            let audio = clip.audio.unwrap_or_default();
            let gain = if audio.gain.is_finite() {
                audio.gain.clamp(0.0, 4.0)
            } else {
                1.0
            };
            let fade_in = if audio.fade_in.is_finite() {
                audio.fade_in.max(0.0).min(clip.duration)
            } else {
                0.0
            };
            let fade_out = if audio.fade_out.is_finite() {
                audio.fade_out.max(0.0).min(clip.duration)
            } else {
                0.0
            };
            clips.push(ResolvedAudioClip {
                id: clip.id.clone(),
                path: path.clone(),
                start: clip.start,
                duration: clip.duration,
                offset: source.offset,
                rate: source.rate,
                gain,
                fade_in,
                fade_out,
            });
        }
    }
    clips
}

/// Hardware decoder budget for live preview: layers showing video media. Extra
/// lower ones are dropped so the topmost (covering) tracks stay visible. Stills
/// on video tracks decode once and only count towards the total.
pub const MAX_DECODED_LAYERS: usize = 6;
/// Text rasters are stills with their own budget.
pub const MAX_TEXT_LAYERS: usize = 4;
/// Total stack size accepted by the preview protocol and native compositor.
/// The single source for the limit; `todo_preview` and the native host mirror it.
pub const MAX_PREVIEW_LAYERS: usize = 16;

pub fn resolve_preview(
    project: &Project,
    timeline_time: f64,
    track_id: Option<&str>,
    kinds: Option<&[TrackKind]>,
) -> Option<PreviewHit> {
    let t = timeline_time.max(0.0);
    let default_kinds = [
        TrackKind::Text,
        TrackKind::Video,
        TrackKind::Audio,
        TrackKind::Sfx,
    ];
    let kinds = kinds.unwrap_or(&default_kinds);
    for track in project.tracks.iter().filter(|track| match track_id {
        Some(id) => track.id == id,
        None => kinds.contains(&track.kind),
    }) {
        if track.muted {
            continue;
        }
        if let Some(hit) = track_hits(project, track, t).pop() {
            return Some(hit);
        }
    }
    None
}

/// Visible picture layers (text and video) at `timeline_time`, bottom → top.
///
/// Earlier tracks are visually higher (they cover), so this is the reverse of
/// track order. Over budget, the deepest layers of the exceeding group are
/// dropped: at most [`MAX_DECODED_LAYERS`] decoded videos, [`MAX_TEXT_LAYERS`]
/// text layers and [`MAX_PREVIEW_LAYERS`] layers in total.
pub fn resolve_layers(project: &Project, timeline_time: f64) -> Vec<PreviewHit> {
    let t = timeline_time.max(0.0);
    let mut layers = Vec::new();
    for track in project.tracks.iter().rev() {
        if track.muted || !track.kind.is_visual() {
            continue;
        }
        layers.extend(track_hits(project, track, t));
    }
    let decoded = |hit: &PreviewHit| hit.media().is_some_and(is_video_media);
    let text = |hit: &PreviewHit| hit.track_kind == TrackKind::Text;
    let any = |_: &PreviewHit| true;
    drop_deepest(&mut layers, MAX_DECODED_LAYERS, decoded);
    drop_deepest(&mut layers, MAX_TEXT_LAYERS, text);
    drop_deepest(&mut layers, MAX_PREVIEW_LAYERS, any);
    layers
}

/// Exact layer stack for export. Unlike live preview this does not silently
/// drop decoded-video or text layers; callers get an actionable error when the
/// composition exceeds the protocol's hard 16-layer limit.
pub fn export_layers(project: &Project, timeline_time: f64) -> Result<Vec<PreviewHit>, String> {
    let layers = visible_hits(project, timeline_time);
    if layers.len() > MAX_PREVIEW_LAYERS {
        return Err(format!(
            "export frame has {} visual layers; maximum is {}",
            layers.len(),
            MAX_PREVIEW_LAYERS
        ));
    }
    Ok(layers)
}

/// Every visible picture hit at `timeline_time`, bottom → top, without any
/// budget: what export draws and what the control view reports.
pub fn visible_hits(project: &Project, timeline_time: f64) -> Vec<PreviewHit> {
    let t = timeline_time.max(0.0);
    let mut layers = Vec::new();
    for track in project.tracks.iter().rev() {
        if track.muted || !track.kind.is_visual() {
            continue;
        }
        layers.extend(track_hits(project, track, t));
    }
    layers
}

/// Keep at most `cap` matching layers of a bottom → top stack, dropping the deepest.
fn drop_deepest(layers: &mut Vec<PreviewHit>, cap: usize, matches: impl Fn(&PreviewHit) -> bool) {
    let mut excess = layers
        .iter()
        .filter(|hit| matches(hit))
        .count()
        .saturating_sub(cap);
    layers.retain(|hit| {
        if excess > 0 && matches(hit) {
            excess -= 1;
            return false;
        }
        true
    });
}

/// The transform a hit is drawn with: the clip's own transform, its intro and
/// outro on the visible span, and its side of an active transition. One rule
/// for preview, export and the control `describe` view.
pub fn hit_transform(project: &Project, hit: &PreviewHit) -> Transform {
    let mut transform = picture_transform(project, &hit.clip, hit.local_time, hit.span);
    if let Some(phase) = &hit.transition {
        if let Some(preset) = transition_preset(project, &phase.kind) {
            let sample = if phase.incoming {
                preset.sample(phase.progress)
            } else {
                preset.sample_outgoing(phase.progress)
            };
            apply_motion(&mut transform, sample);
        }
    }
    transform
}

fn transition_preset<'a>(project: &'a Project, id: &str) -> Option<&'a MotionPreset> {
    find_motion(project, id).filter(|preset| preset.kind == MotionKind::Transition)
}

/// The transition into `clips[index]` from its adjacent predecessor, with its
/// window clamped so it never exceeds either clip (two windows on one clip
/// therefore never overlap).
fn incoming_transition<'a>(
    project: &'a Project,
    clips: &[&Clip],
    index: usize,
) -> Option<(&'a MotionPreset, f64)> {
    let clip = clips.get(index)?;
    let motion = clip.transition.as_ref()?;
    let previous = clips.get(index.checked_sub(1)?)?;
    if (clip_end(previous) - clip.start).abs() > ADJACENT_EPSILON {
        return None;
    }
    let preset = transition_preset(project, &motion.kind)?;
    let duration = motion.duration.min(previous.duration).min(clip.duration);
    (duration.is_finite() && duration > 1e-6).then_some((preset, duration))
}

/// Whether the transition stored on `clip_id` plays: its predecessor on
/// `track` is adjacent and the preset resolves.
pub fn transition_is_active(project: &Project, track: &Track, clip_id: &str) -> bool {
    let clips = sorted_clips(track);
    clips
        .iter()
        .position(|clip| clip.id == clip_id)
        .is_some_and(|index| incoming_transition(project, &clips, index).is_some())
}

/// The clip a transition into `clip_id` would come from: the one ending
/// where it starts on `track`.
pub fn adjacent_predecessor<'a>(track: &'a Track, clip_id: &str) -> Option<&'a Clip> {
    let clip = track.clips.iter().find(|clip| clip.id == clip_id)?;
    track.clips.iter().find(|other| {
        other.id != clip.id && (clip_end(other) - clip.start).abs() <= ADJACENT_EPSILON
    })
}

fn sorted_clips(track: &Track) -> Vec<&Clip> {
    let mut clips: Vec<&Clip> = track.clips.iter().collect();
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    clips
}

/// Visible hits of one track at `t`, bottom → top: one clip, or two inside a
/// transition window (the incoming one on top unless the preset says otherwise).
fn track_hits(project: &Project, track: &Track, t: f64) -> Vec<PreviewHit> {
    let clips = sorted_clips(track);
    let mut hits = Vec::new();
    let mut outgoing_on_top = false;
    for index in 0..clips.len() {
        let clip = clips[index];
        let into = incoming_transition(project, &clips, index);
        let out_of = incoming_transition(project, &clips, index + 1);
        let span = VisibleSpan {
            lead: into.map_or(0.0, |(_, d)| d / 2.0),
            tail: out_of.map_or(0.0, |(_, d)| d / 2.0),
        };
        if t < clip.start - span.lead || t >= clip_end(clip) + span.tail {
            continue;
        }
        let phase = match (into, out_of) {
            (Some((preset, d)), _) if t < clip.start + d / 2.0 => Some(TransitionPhase {
                kind: preset.id.clone(),
                progress: ((t - (clip.start - d / 2.0)) / d).clamp(0.0, 1.0),
                incoming: true,
                duration: d,
            }),
            (_, Some((preset, d))) if t >= clip_end(clip) - d / 2.0 => {
                outgoing_on_top = preset.outgoing_on_top;
                Some(TransitionPhase {
                    kind: preset.id.clone(),
                    progress: ((t - (clip_end(clip) - d / 2.0)) / d).clamp(0.0, 1.0),
                    incoming: false,
                    duration: d,
                })
            }
            _ => None,
        };
        if let Some(hit) = make_hit(project, track, clip, t, span, phase) {
            hits.push(hit);
        }
        if hits.len() == 2 {
            break;
        }
    }
    if outgoing_on_top && hits.len() == 2 {
        hits.swap(0, 1);
    }
    hits
}

fn make_hit(
    project: &Project,
    track: &Track,
    clip: &Clip,
    t: f64,
    span: VisibleSpan,
    transition: Option<TransitionPhase>,
) -> Option<PreviewHit> {
    let local_time = t - clip.start;
    let (source, source_size) = match &clip.source {
        ClipSource::Media(source) => {
            let media = project.media.iter().find(|m| m.id == source.media_id)?;
            // Outside the clip (transition handles) the source runs on and
            // holds its first or last frame where it has no more material.
            let media_time = source.offset + local_time * source.rate;
            let size = match (media.width, media.height) {
                (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
                _ => None,
            };
            let media_time = clamp_media_time(media_time, media.duration);
            (
                HitSource::Media {
                    media: media.clone(),
                    media_time,
                },
                size,
            )
        }
        ClipSource::Text(content) => {
            let style = project.text_style(&content.style_id)?;
            (
                HitSource::Text {
                    style: style.clone(),
                    // Text animation runs on the visible span, like intros.
                    frame: content
                        .animation
                        .frame_at((local_time + span.lead).max(0.0)),
                },
                Some(project.output_size()),
            )
        }
    };
    Some(PreviewHit {
        track_id: track.id.clone(),
        track_kind: track.kind,
        clip: clip.clone(),
        source,
        source_size,
        timeline_time: t,
        local_time,
        span,
        transition,
    })
}

fn clamp_media_time(time: f64, duration: f64) -> f64 {
    if !duration.is_finite() || duration <= 0.0 {
        return time.max(0.0);
    }
    time.max(0.0).min((duration - 1e-4).max(0.0))
}
