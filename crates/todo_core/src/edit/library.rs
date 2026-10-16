//! Project media references and virtual library folders. Never modifies source files.
use crate::edit::validate::folder_is_ancestor;
use crate::{cleanup_empty_tracks, DomainError, MediaFolder, MediaItem, Project};

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

fn require_folder<'a>(project: &'a Project, id: &str) -> Result<&'a MediaFolder, DomainError> {
    project
        .folders
        .iter()
        .find(|folder| folder.id == id)
        .ok_or_else(|| DomainError::FolderNotFound(id.into()))
}

fn require_parent(project: &Project, parent_id: Option<&str>) -> Result<(), DomainError> {
    if let Some(id) = parent_id {
        require_folder(project, id)?;
    }
    Ok(())
}

pub fn create_folder(
    mut project: Project,
    id: String,
    name: String,
    parent_id: Option<String>,
) -> Result<Project, DomainError> {
    let name = name.trim().to_string();
    if id.is_empty() || name.is_empty() {
        return Err(DomainError::InvalidProject("folder"));
    }
    if project.folders.iter().any(|folder| folder.id == id) {
        return Err(DomainError::DuplicateId(id));
    }
    require_parent(&project, parent_id.as_deref())?;
    project.folders.push(MediaFolder {
        id,
        name,
        parent_id,
    });
    Ok(project)
}

pub fn rename_folder(mut project: Project, id: &str, name: String) -> Result<Project, DomainError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(DomainError::InvalidProject("folder"));
    }
    let folder = project
        .folders
        .iter_mut()
        .find(|folder| folder.id == id)
        .ok_or_else(|| DomainError::FolderNotFound(id.into()))?;
    folder.name = name;
    Ok(project)
}

/// Remove a folder; children and media move to its parent (or root).
pub fn delete_folder(mut project: Project, id: &str) -> Result<Project, DomainError> {
    let parent_id = require_folder(&project, id)?.parent_id.clone();
    for folder in &mut project.folders {
        if folder.parent_id.as_deref() == Some(id) {
            folder.parent_id = parent_id.clone();
        }
    }
    for media in &mut project.media {
        if media.folder_id.as_deref() == Some(id) {
            media.folder_id = parent_id.clone();
        }
    }
    project.folders.retain(|folder| folder.id != id);
    Ok(project)
}

pub fn move_folder(
    mut project: Project,
    id: &str,
    new_parent_id: Option<String>,
) -> Result<Project, DomainError> {
    require_folder(&project, id)?;
    require_parent(&project, new_parent_id.as_deref())?;
    if let Some(parent) = &new_parent_id {
        if folder_is_ancestor(&project, id, parent) {
            return Err(DomainError::FolderCycle(id.into()));
        }
    }
    let folder = project
        .folders
        .iter_mut()
        .find(|folder| folder.id == id)
        .ok_or_else(|| DomainError::FolderNotFound(id.into()))?;
    folder.parent_id = new_parent_id;
    Ok(project)
}

pub fn move_media(
    mut project: Project,
    media_id: &str,
    folder_id: Option<String>,
) -> Result<Project, DomainError> {
    require_parent(&project, folder_id.as_deref())?;
    let media = project
        .media
        .iter_mut()
        .find(|media| media.id == media_id)
        .ok_or_else(|| DomainError::MediaNotFound(media_id.into()))?;
    media.folder_id = folder_id;
    Ok(project)
}
