use super::track_space::{allocate, free};
use crate::model::media::{is_image_media, media_kind, place_duration, MediaKind};
use crate::{add_clip, default_clip_id, normalize_clip, Project, TrackKind};

fn suitable_track(
    mut project: Project,
    kind: TrackKind,
    start: f64,
    duration: f64,
) -> (Project, String) {
    let id = allocate(&mut project, kind, None, &[(start, duration)]);
    (project, id)
}

pub(crate) fn unique_id(project: &Project) -> String {
    loop {
        let id = default_clip_id();
        if !project
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .any(|c| c.id == id || c.link_id.as_deref() == Some(&id))
        {
            return id;
        }
    }
}

pub fn place_media(
    project: Project,
    media_id: &str,
    track_id: Option<&str>,
    start: f64,
) -> Result<Project, crate::DomainError> {
    let media = project
        .media
        .iter()
        .find(|m| m.id == media_id)
        .ok_or_else(|| crate::DomainError::MediaNotFound(media_id.into()))?;
    let duration = place_duration(media);
    place_span(project, media_id, track_id, start, 0.0, duration)
}

/// Place a source interval at an absolute timeline time, at normal speed.
/// Stills use a zero source offset and an arbitrary positive hold duration.
pub fn place_media_range(
    project: Project,
    media_id: &str,
    track_id: Option<&str>,
    start: f64,
    source_in: f64,
    source_out: f64,
) -> Result<Project, crate::DomainError> {
    crate::validate_project(&project)?;
    let media = project
        .media
        .iter()
        .find(|m| m.id == media_id)
        .ok_or_else(|| crate::DomainError::MediaNotFound(media_id.into()))?;
    if !source_in.is_finite()
        || !source_out.is_finite()
        || source_in < 0.0
        || source_out <= source_in
        || (is_image_media(media) && source_in != 0.0)
        || (!is_image_media(media) && source_out > media.duration)
    {
        return Err(crate::DomainError::SourceBounds(media_id.into()));
    }
    place_span(
        project,
        media_id,
        track_id,
        start,
        source_in,
        source_out - source_in,
    )
}

fn place_span(
    project: Project,
    media_id: &str,
    track_id: Option<&str>,
    start: f64,
    source_in: f64,
    duration: f64,
) -> Result<Project, crate::DomainError> {
    let media = project
        .media
        .iter()
        .find(|m| m.id == media_id)
        .cloned()
        .ok_or_else(|| crate::DomainError::MediaNotFound(media_id.into()))?;
    if !start.is_finite()
        || start < 0.0
        || !duration.is_finite()
        || duration + 1e-9 < crate::edit::minimum_duration(&project)
        || !(start + duration).is_finite()
    {
        return Err(crate::DomainError::InvalidTime);
    }
    let resolved = media_kind(&media);
    // Stills land on a video track like other visual media; no linked audio.
    let audio_only = resolved == MediaKind::Audio;
    let kind = if audio_only {
        TrackKind::Audio
    } else {
        TrackKind::Video
    };
    let (mut next, target) = if let Some(id) = track_id {
        let track = project
            .tracks
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| crate::DomainError::TrackNotFound(id.into()))?;
        let compatible = track.kind == kind || (audio_only && track.kind == TrackKind::Sfx);
        if !compatible {
            return Err(crate::DomainError::IncompatibleTrack(id.into()));
        }
        if track.locked {
            return Err(crate::DomainError::TrackLocked(id.into()));
        }
        if free(track, start, duration) {
            (project, id.to_string())
        } else {
            // Occupied lane: spill to a free track or create one (PR-style).
            suitable_track(project, kind, start, duration)
        }
    } else {
        suitable_track(project, kind, start, duration)
    };
    let mut picture = normalize_clip(
        unique_id(&next),
        media.id.clone(),
        start,
        duration,
        source_in,
        1.0,
    );
    // Unknown dimensions keep aspect-fit fill; known sizes follow project media_scale.
    if let (Some(w), Some(h)) = (media.width, media.height) {
        picture.transform = crate::Transform::for_media_placement(
            (w, h),
            next.output_size(),
            next.media_scale,
        );
    }
    let link_audio = !audio_only
        && !is_image_media(&media)
        && media.has_audio == Some(true)
        && resolved == MediaKind::Video;
    if link_audio {
        picture.link_id = Some(unique_id(&next));
    }
    next = add_clip(next, &target, picture.clone());
    if link_audio {
        let (project, audio_track) = suitable_track(next, TrackKind::Audio, start, duration);
        next = project;
        let mut sound = picture;
        sound.id = unique_id(&next);
        next = add_clip(next, &audio_track, sound);
    }
    Ok(next)
}
