use super::minimum_duration;
use super::{
    stretch::{stretch_in, stretch_out},
    trim::{edge_limits, trim_in, trim_out},
};
use crate::{
    clip_end, find_clip, linked_clip_ids, validate_project, DomainError, Project, TrimEdge,
};

/// Apply a shared edge delta to linked clips, preserving their relative timing.
/// The whole operation fails if source limits would clamp group members differently.
pub fn trim_clips(
    project: Project,
    primary_id: &str,
    edge: TrimEdge,
    time: f64,
    linked: bool,
) -> Result<Project, DomainError> {
    edge_edit(project, primary_id, edge, time, linked, false)
}

pub fn stretch_clips(
    project: Project,
    primary_id: &str,
    edge: TrimEdge,
    time: f64,
    linked: bool,
) -> Result<Project, DomainError> {
    edge_edit(project, primary_id, edge, time, linked, true)
}

fn edge_edit(
    project: Project,
    primary_id: &str,
    edge: TrimEdge,
    time: f64,
    linked: bool,
    stretch: bool,
) -> Result<Project, DomainError> {
    validate_project(&project)?;
    if !time.is_finite() {
        return Err(DomainError::InvalidTime);
    }
    let primary = find_clip(&project, primary_id)
        .ok_or_else(|| DomainError::ClipNotFound(primary_id.into()))?
        .clip;
    let ids = if linked {
        linked_clip_ids(&project, &[primary_id.into()])
    } else {
        vec![primary_id.into()]
    };
    let minimum = minimum_duration(&project);
    let mut next = project.clone();
    for id in &ids {
        let found = find_clip(&project, id).ok_or_else(|| DomainError::ClipNotFound(id.clone()))?;
        if found.track.locked {
            return Err(DomainError::TrackLocked(found.track_id));
        }
        let original = found.clip;
        let shift = match edge {
            TrimEdge::In => original.start - primary.start,
            TrimEdge::Out => clip_end(&original) - clip_end(&primary),
        };
        let target_time = time + shift;
        if !target_time.is_finite() {
            return Err(DomainError::InvalidTime);
        }
        if stretch && original.is_text() {
            return Err(DomainError::UnsupportedClip(id.clone()));
        }
        let limits = edge_limits(&project, &original)?;
        let changed = match (stretch, edge) {
            (true, TrimEdge::In) => stretch_in(original, target_time, minimum),
            (true, TrimEdge::Out) => stretch_out(original, target_time, minimum),
            (false, TrimEdge::In) => trim_in(original, target_time, limits, minimum),
            (false, TrimEdge::Out) => trim_out(original, target_time, limits, minimum),
        };
        for clip in next
            .tracks
            .iter_mut()
            .flat_map(|t| &mut t.clips)
            .filter(|c| c.id == *id)
        {
            *clip = changed.clone();
        }
    }
    let changed_primary = find_clip(&next, primary_id).unwrap().clip;
    for id in &ids {
        let old = find_clip(&project, id).unwrap().clip;
        let changed = find_clip(&next, id).unwrap().clip;
        if ((changed.duration - old.duration) - (changed_primary.duration - primary.duration)).abs()
            > 1e-6
            || ((changed.start - old.start) - (changed_primary.start - primary.start)).abs() > 1e-6
        {
            return Err(DomainError::LinkedEditMismatch);
        }
    }
    validate_project(&next)?;
    Ok(next)
}
