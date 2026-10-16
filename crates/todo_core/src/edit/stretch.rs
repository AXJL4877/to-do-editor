use crate::model::clip::{source_span, Clip, MAX_CLIP_RATE, MIN_CLIP_RATE};
use crate::model::Project;

use super::mutate::patch_clip;
use super::trim::TrimEdge;

pub fn stretch(project: Project, clip_id: &str, edge: TrimEdge, time: f64) -> Project {
    if !time.is_finite() {
        return project;
    }
    let minimum = crate::edit::minimum_duration(&project);
    patch_clip(project, clip_id, |clip, track_id| {
        let next = match edge {
            TrimEdge::In => stretch_in(clip, time, minimum),
            TrimEdge::Out => stretch_out(clip, time, minimum),
        };
        Some((next, track_id))
    })
}

/// Retiming applies to media only; text clips have no source to speed up.
pub(crate) fn stretch_out(mut clip: Clip, time: f64, minimum: f64) -> Clip {
    if clip.is_text() {
        return clip;
    }
    let span = source_span(&clip);
    let mut duration = (time - clip.start).max(minimum);
    let mut rate = span / duration;
    if rate < MIN_CLIP_RATE {
        rate = MIN_CLIP_RATE;
        duration = span / rate;
    } else if rate > MAX_CLIP_RATE {
        rate = MAX_CLIP_RATE;
        duration = span / rate;
    }
    clip.duration = duration;
    if let Some(source) = clip.media_mut() {
        source.rate = rate;
    }
    clip
}

pub(crate) fn stretch_in(mut clip: Clip, time: f64, minimum: f64) -> Clip {
    if clip.is_text() {
        return clip;
    }
    let end = clip.start + clip.duration;
    let span = source_span(&clip);
    let mut start = time.max(0.0).min(end - minimum);
    let mut duration = end - start;
    let mut rate = span / duration;
    if rate < MIN_CLIP_RATE {
        rate = MIN_CLIP_RATE;
        duration = span / rate;
        start = end - duration;
    } else if rate > MAX_CLIP_RATE {
        rate = MAX_CLIP_RATE;
        duration = span / rate;
        start = end - duration;
    }
    clip.start = start.max(0.0);
    clip.duration = duration;
    if let Some(source) = clip.media_mut() {
        source.rate = rate;
    }
    clip
}
