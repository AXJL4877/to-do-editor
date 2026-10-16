//! Project media references only. These operations never modify source files.
use crate::{cleanup_empty_tracks, MediaItem, Project};

pub use add_to_library as register_media;

/// Register metadata only; source files are never accessed by the domain.
pub fn add_to_library(mut project: Project, item: MediaItem) -> Project {
    if !project.media.iter().any(|m| m.id == item.id) {
        project.media.push(item);
    }
    project
}

pub fn remove_media(
    mut project: Project,
    media_id: &str,
    cascade: bool,
) -> Result<Project, crate::DomainError> {
    if !project.media.iter().any(|m| m.id == media_id) {
        return Err(crate::DomainError::MediaNotFound(media_id.into()));
    }
    let used = project
        .tracks
        .iter()
        .any(|t| t.clips.iter().any(|c| c.media_id() == Some(media_id)));
    if used && !cascade {
        return Err(crate::DomainError::MediaInUse(media_id.into()));
    }
    if project
        .tracks
        .iter()
        .any(|t| t.locked && t.clips.iter().any(|c| c.media_id() == Some(media_id)))
    {
        let id = project
            .tracks
            .iter()
            .find(|t| t.locked && t.clips.iter().any(|c| c.media_id() == Some(media_id)))
            .unwrap()
            .id
            .clone();
        return Err(crate::DomainError::TrackLocked(id));
    }
    project.media.retain(|m: &MediaItem| m.id != media_id);
    if used {
        for track in &mut project.tracks {
            track.clips.retain(|c| c.media_id() != Some(media_id));
        }
        project = cleanup_empty_tracks(project);
    }
    Ok(project)
}
