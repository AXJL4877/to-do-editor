//! Text clip placement and content edits.
use super::place_media::unique_id;
use super::track_space::allocate;
use super::validate_placement;
use crate::model::clip::text_clip;
use crate::model::text::builtin_text_styles;
use crate::{find_clip, DomainError, Project, TextContent, TrackKind};

/// Make `style_id` available to the project, copying a built-in style once.
/// Project styles win over built-ins with the same id.
fn ensure_style(mut project: Project, style_id: &str) -> Result<Project, DomainError> {
    if project.text_style(style_id).is_some() {
        return Ok(project);
    }
    let style = builtin_text_styles()
        .into_iter()
        .find(|style| style.id == style_id)
        .ok_or_else(|| DomainError::TextStyleNotFound(style_id.into()))?;
    project.text_styles.push(style);
    Ok(project)
}

/// Place a text clip on the requested text lane, or the first free one, or a new one.
pub fn add_text(
    project: Project,
    track_id: Option<&str>,
    start: f64,
    duration: f64,
    content: TextContent,
) -> Result<Project, DomainError> {
    if !start.is_finite()
        || start < 0.0
        || !duration.is_finite()
        || duration + 1e-9 < super::minimum_duration(&project)
        || !(start + duration).is_finite()
    {
        return Err(DomainError::InvalidTime);
    }
    if let Some(id) = track_id {
        let track = project
            .tracks
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| DomainError::TrackNotFound(id.into()))?;
        if track.kind != TrackKind::Text {
            return Err(DomainError::IncompatibleTrack(id.into()));
        }
        if track.locked {
            return Err(DomainError::TrackLocked(id.into()));
        }
    }
    let mut next = ensure_style(project, &content.style_id)?;
    let target = allocate(&mut next, TrackKind::Text, track_id, &[(start, duration)]);
    let clip = text_clip(unique_id(&next), start, duration, content);
    let track = next.tracks.iter().find(|t| t.id == target).unwrap();
    validate_placement(&next, track, &clip, &[])?;
    let track = next.tracks.iter_mut().find(|t| t.id == target).unwrap();
    track.clips.push(clip);
    track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(next)
}

/// Replace the content of an existing text clip; timing and transform are kept.
pub fn set_text(
    project: Project,
    clip_id: &str,
    content: TextContent,
) -> Result<Project, DomainError> {
    let found =
        find_clip(&project, clip_id).ok_or_else(|| DomainError::ClipNotFound(clip_id.into()))?;
    if found.track.locked {
        return Err(DomainError::TrackLocked(found.track_id));
    }
    if !found.clip.is_text() {
        return Err(DomainError::UnsupportedClip(clip_id.into()));
    }
    let mut next = ensure_style(project, &content.style_id)?;
    let clip = next
        .tracks
        .iter_mut()
        .flat_map(|t| &mut t.clips)
        .find(|c| c.id == clip_id)
        .unwrap();
    *clip.text_mut().unwrap() = content;
    Ok(next)
}
