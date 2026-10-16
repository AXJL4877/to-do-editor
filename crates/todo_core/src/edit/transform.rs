use crate::{DomainError, Project, Transform};

/// Apply one visual transform without changing timeline or linked clip state.
pub(super) fn set_transform(
    mut project: Project,
    clip_id: &str,
    transform: Transform,
) -> Result<Project, DomainError> {
    if !transform.is_valid() {
        return Err(DomainError::InvalidTransform);
    }
    for track in &mut project.tracks {
        if let Some(clip) = track.clips.iter_mut().find(|clip| clip.id == clip_id) {
            if track.locked {
                return Err(DomainError::TrackLocked(track.id.clone()));
            }
            clip.transform = transform;
            return Ok(project);
        }
    }
    Err(DomainError::ClipNotFound(clip_id.to_owned()))
}
