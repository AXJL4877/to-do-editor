use crate::model::clip::Clip;
use crate::model::Project;
use crate::DomainError;

use super::mutate::patch_clip;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEdge {
    In,
    Out,
}

/// How far a clip's edges may travel: a media bound (None for stills and text)
/// and whether the in-point is anchored to the source (stills, text).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EdgeLimits {
    pub bound: Option<f64>,
    pub anchored: bool,
}

pub(crate) fn edge_limits(project: &Project, clip: &Clip) -> Result<EdgeLimits, DomainError> {
    let Some(source) = clip.media() else {
        return Ok(EdgeLimits {
            bound: None,
            anchored: true,
        });
    };
    let media = project
        .media
        .iter()
        .find(|m| m.id == source.media_id)
        .ok_or_else(|| DomainError::MediaNotFound(source.media_id.clone()))?;
    let still = crate::is_image_media(media);
    Ok(EdgeLimits {
        bound: (!still).then_some(media.duration),
        anchored: still,
    })
}

pub fn trim(
    project: Project,
    clip_id: &str,
    edge: TrimEdge,
    time: f64,
    media_duration: Option<f64>,
) -> Project {
    if !time.is_finite() {
        return project;
    }
    let minimum = crate::edit::minimum_duration(&project);
    let Some(found) = crate::find_clip(&project, clip_id) else {
        return project;
    };
    let Ok(mut limits) = edge_limits(&project, &found.clip) else {
        return project;
    };
    if let (Some(bound), Some(requested)) = (limits.bound, media_duration) {
        if requested.is_finite() && requested > 0.0 {
            limits.bound = Some(requested.min(bound));
        }
    }
    patch_clip(project, clip_id, |clip, track_id| {
        let next = match edge {
            TrimEdge::In => trim_in(clip, time, limits, minimum),
            TrimEdge::Out => trim_out(clip, time, limits, minimum),
        };
        Some((next, track_id))
    })
}

pub(crate) fn trim_in(mut clip: Clip, time: f64, limits: EdgeLimits, minimum: f64) -> Clip {
    let end = clip.start + clip.duration;
    let mut start = time.max(0.0).min(end - minimum);
    let delta = start - clip.start;
    let mut duration = end - start;
    if let Some(source) = clip.media_mut() {
        let mut offset = if limits.anchored {
            source.offset
        } else {
            source.offset + delta * source.rate
        };
        if offset < 0.0 {
            let fix = -offset / source.rate;
            start += fix;
            offset = 0.0;
            duration = end - start;
        }
        if let Some(media_duration) = limits.bound {
            let max_span = media_duration - offset;
            if duration * source.rate > max_span {
                duration = (max_span / source.rate).max(minimum);
            }
        }
        source.offset = offset;
    }
    clip.start = start;
    clip.duration = duration;
    clip
}

pub(crate) fn trim_out(mut clip: Clip, time: f64, limits: EdgeLimits, minimum: f64) -> Clip {
    let mut duration = (time - clip.start).max(minimum);
    if let (Some(media_duration), Some(source)) = (limits.bound, clip.media()) {
        let max_span = (media_duration - source.offset).max(minimum * source.rate);
        duration = duration.min(max_span / source.rate);
    }
    clip.duration = duration;
    clip
}
