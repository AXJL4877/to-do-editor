use crate::model::text::{text_content_valid, text_style_valid};
use crate::{
    clip_end, is_image_media, media_kind, Clip, ClipSource, DomainError, MediaItem, MediaKind,
    MediaSource, Project, Track, TrackKind, MAX_CLIP_RATE, MIN_CLIP_DURATION, MIN_CLIP_RATE,
};
use std::collections::HashSet;

pub(crate) const TIME_EPSILON: f64 = 1e-7;

pub(crate) fn minimum_duration(project: &Project) -> f64 {
    if project.fps.is_finite() && project.fps > 0.0 {
        MIN_CLIP_DURATION.max(1.0 / project.fps)
    } else {
        1.0 / 30.0
    }
}

pub(crate) fn clip_compatible(media: &MediaItem, kind: TrackKind) -> bool {
    match kind {
        TrackKind::Text => false,
        TrackKind::Video => media_kind(media) != MediaKind::Audio,
        TrackKind::Audio | TrackKind::Sfx => {
            media_kind(media) == MediaKind::Audio
                || (media_kind(media) == MediaKind::Video && media.has_audio == Some(true))
        }
    }
}

pub(crate) fn validate_clip(
    project: &Project,
    clip: &Clip,
    kind: TrackKind,
) -> Result<(), DomainError> {
    if clip.id.is_empty() {
        return Err(DomainError::InvalidProject("clip_id"));
    }
    if !clip.start.is_finite() || clip.start < 0.0 || !clip_end(clip).is_finite() {
        return Err(DomainError::InvalidTime);
    }
    if !clip.duration.is_finite() || clip.duration + 1e-9 < minimum_duration(project) {
        return Err(DomainError::ClipTooShort(clip.id.clone()));
    }
    if !clip.transform.is_valid() {
        return Err(DomainError::InvalidTransform);
    }
    match &clip.source {
        ClipSource::Media(source) => validate_media_source(project, clip, source, kind),
        ClipSource::Text(content) => {
            if kind != TrackKind::Text {
                return Err(DomainError::IncompatibleTrack(clip.id.clone()));
            }
            if !text_content_valid(content) {
                return Err(DomainError::InvalidText(clip.id.clone()));
            }
            if project.text_style(&content.style_id).is_none() {
                return Err(DomainError::TextStyleNotFound(content.style_id.clone()));
            }
            Ok(())
        }
    }
}

fn validate_media_source(
    project: &Project,
    clip: &Clip,
    source: &MediaSource,
    kind: TrackKind,
) -> Result<(), DomainError> {
    if source.media_id.is_empty() {
        return Err(DomainError::InvalidProject("clip_id"));
    }
    if !source.rate.is_finite()
        || !(MIN_CLIP_RATE..=MAX_CLIP_RATE).contains(&source.rate)
        || !source.offset.is_finite()
        || source.offset < 0.0
        || !(source.offset + clip.duration * source.rate).is_finite()
    {
        return Err(DomainError::SourceBounds(clip.id.clone()));
    }
    let media = project
        .media
        .iter()
        .find(|m| m.id == source.media_id)
        .ok_or_else(|| DomainError::MediaNotFound(source.media_id.clone()))?;
    if !clip_compatible(media, kind) {
        return Err(DomainError::IncompatibleTrack(clip.id.clone()));
    }
    if !is_image_media(media)
        && (!media.duration.is_finite()
            || source.offset + clip.duration * source.rate > media.duration + TIME_EPSILON)
    {
        return Err(DomainError::SourceBounds(clip.id.clone()));
    }
    Ok(())
}

pub(crate) fn validate_placement(
    project: &Project,
    track: &Track,
    clip: &Clip,
    excluded_ids: &[&str],
) -> Result<(), DomainError> {
    if track.locked {
        return Err(DomainError::TrackLocked(track.id.clone()));
    }
    validate_clip(project, clip, track.kind)?;
    if track
        .clips
        .iter()
        .filter(|c| !excluded_ids.contains(&c.id.as_str()))
        .any(|c| clip.start < clip_end(c) - TIME_EPSILON && clip_end(clip) > c.start + TIME_EPSILON)
    {
        return Err(DomainError::Overlap(track.id.clone()));
    }
    Ok(())
}

/// Validate persisted data as well as the result of an edit, without I/O or repair.
/// Legacy projects may omit workspace duration and contain no tracks.
pub fn validate_project(project: &Project) -> Result<(), DomainError> {
    if project.version != 1 {
        return Err(DomainError::InvalidProject("version"));
    }
    if !project.fps.is_finite()
        || project.fps < super::project::MIN_FPS
        || project.fps > super::project::MAX_FPS
    {
        return Err(DomainError::InvalidProject("fps"));
    }
    if project.width == 0
        || project.height == 0
        || project.width > super::project::MAX_OUTPUT_EDGE
        || project.height > super::project::MAX_OUTPUT_EDGE
    {
        return Err(DomainError::InvalidProject("output_size"));
    }
    if !project.playhead.is_finite() || project.playhead < 0.0 {
        return Err(DomainError::InvalidTime);
    }
    if project.duration.is_some_and(|d| !d.is_finite() || d <= 0.0) {
        return Err(DomainError::InvalidDuration);
    }
    let mut folder_ids = HashSet::new();
    for folder in &project.folders {
        if folder.id.is_empty() || folder.name.trim().is_empty() {
            return Err(DomainError::InvalidProject("folder"));
        }
        if !folder_ids.insert(folder.id.as_str()) {
            return Err(DomainError::DuplicateId(folder.id.clone()));
        }
    }
    for folder in &project.folders {
        if let Some(parent_id) = &folder.parent_id {
            if !folder_ids.contains(parent_id.as_str()) {
                return Err(DomainError::FolderNotFound(parent_id.clone()));
            }
            if folder_is_ancestor(project, &folder.id, parent_id) {
                return Err(DomainError::FolderCycle(folder.id.clone()));
            }
        }
    }
    let mut media_ids = HashSet::new();
    for media in &project.media {
        if media.id.is_empty() {
            return Err(DomainError::InvalidProject("media_id"));
        }
        if !media_ids.insert(&media.id) {
            return Err(DomainError::DuplicateId(media.id.clone()));
        }
        if let Some(folder_id) = &media.folder_id {
            if !folder_ids.contains(folder_id.as_str()) {
                return Err(DomainError::FolderNotFound(folder_id.clone()));
            }
        }
        if !media.duration.is_finite()
            || media.duration < 0.0
            || (!is_image_media(media) && media.duration == 0.0)
        {
            return Err(DomainError::InvalidDuration);
        }
    }
    let mut style_ids = HashSet::new();
    for style in &project.text_styles {
        if !text_style_valid(style) {
            return Err(DomainError::InvalidTextStyle(style.id.clone()));
        }
        if !style_ids.insert(&style.id) {
            return Err(DomainError::DuplicateId(style.id.clone()));
        }
    }
    let mut track_ids = HashSet::new();
    let mut clip_ids = HashSet::new();
    for track in &project.tracks {
        if track.id.is_empty() {
            return Err(DomainError::InvalidProject("track_id"));
        }
        if !track_ids.insert(&track.id) {
            return Err(DomainError::DuplicateId(track.id.clone()));
        }
        for clip in &track.clips {
            if !clip_ids.insert(&clip.id) {
                return Err(DomainError::DuplicateId(clip.id.clone()));
            }
            validate_clip(project, clip, track.kind)?;
        }
        let mut clips: Vec<_> = track.clips.iter().collect();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        if clips
            .windows(2)
            .any(|pair| clip_end(pair[0]) > pair[1].start + TIME_EPSILON)
        {
            return Err(DomainError::Overlap(track.id.clone()));
        }
    }
    Ok(())
}

/// True when `ancestor_id` appears on the parent chain of `node_id` (inclusive).
pub(crate) fn folder_is_ancestor(project: &Project, ancestor_id: &str, node_id: &str) -> bool {
    let mut current = Some(node_id);
    let mut steps = 0usize;
    while let Some(id) = current {
        if id == ancestor_id {
            return true;
        }
        current = project
            .folders
            .iter()
            .find(|folder| folder.id == id)
            .and_then(|folder| folder.parent_id.as_deref());
        steps += 1;
        if steps > project.folders.len() + 1 {
            return true;
        }
    }
    false
}
