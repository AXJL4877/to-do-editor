use crate::{clip_end, DomainError, Project};
use std::collections::HashSet;

/// Move a selection atomically.
///
/// When `target_track` is set, every selected clip whose source track is in the
/// same family as the destination (text, video, audio/sfx)
/// prefers that lane. Each source lane moves as a group; collisions reuse or
/// create another lane of the destination kind. Other-family partners prefer
/// their original lanes and spill independently while keeping the same delta.
pub fn move_clips(
    project: Project,
    clip_ids: &[String],
    delta: f64,
    target_track: Option<&str>,
) -> Project {
    try_move_clips(project.clone(), clip_ids, delta, target_track).unwrap_or(project)
}

pub(crate) fn try_move_clips(
    project: Project,
    clip_ids: &[String],
    delta: f64,
    target_track: Option<&str>,
) -> Result<Project, DomainError> {
    crate::validate_project(&project)?;
    if !delta.is_finite() || clip_ids.is_empty() {
        return Err(DomainError::InvalidTime);
    }
    let ids: HashSet<&str> = clip_ids.iter().map(String::as_str).collect();
    let selected: Vec<_> = project
        .tracks
        .iter()
        .flat_map(|t| {
            t.clips
                .iter()
                .filter(|c| ids.contains(c.id.as_str()))
                .map(move |c| (t, c))
        })
        .collect();
    if selected.len() != ids.len() {
        let missing = clip_ids
            .iter()
            .find(|id| !selected.iter().any(|(_, c)| &c.id == *id))
            .unwrap();
        return Err(DomainError::ClipNotFound(missing.clone()));
    }
    for (track, clip) in &selected {
        if track.locked {
            return Err(DomainError::TrackLocked(track.id.clone()));
        }
        if !clip.start.is_finite() {
            return Err(DomainError::InvalidTime);
        }
    }
    let earliest = selected
        .iter()
        .map(|(_, c)| c.start)
        .fold(f64::INFINITY, f64::min);
    let delta = delta.max(-earliest);
    let first = &clip_ids[0];
    if let Some(target_id) = target_track {
        let Some(target) = project.tracks.iter().find(|t| t.id == target_id) else {
            return Err(DomainError::TrackNotFound(target_id.into()));
        };
        if target.locked {
            return Err(DomainError::TrackLocked(target_id.into()));
        }
        let source_kind = selected
            .iter()
            .find(|(_, c)| c.id == *first)
            .unwrap()
            .0
            .kind;
        if !source_kind.same_family(target.kind) {
            return Err(DomainError::IncompatibleTrack(target_id.into()));
        }
    }
    let target_kind =
        target_track.and_then(|id| project.tracks.iter().find(|t| t.id == id).map(|t| t.kind));
    let mut next = project.clone();
    let mut groups = vec![];
    for track in &mut next.tracks {
        let mut keep = vec![];
        let mut moved = vec![];
        for mut clip in std::mem::take(&mut track.clips) {
            if ids.contains(clip.id.as_str()) {
                clip.start += delta;
                if !clip.start.is_finite() || clip.start < 0.0 || !clip_end(&clip).is_finite() {
                    return Err(DomainError::InvalidTime);
                }
                moved.push(clip);
            } else {
                keep.push(clip);
            }
        }
        track.clips = keep;
        if !moved.is_empty() {
            let (destination, kind) = match (target_track, target_kind) {
                (Some(tid), Some(kind)) if track.kind.same_family(kind) => (tid.to_string(), kind),
                _ => (track.id.clone(), track.kind),
            };
            groups.push((destination, kind, moved));
        }
    }
    // The clip being dragged gets first choice of the pointer's destination.
    groups.sort_by_key(|(_, _, clips)| !clips.iter().any(|clip| clip.id == *first));
    for (preferred, kind, clips) in groups {
        let ranges: Vec<_> = clips
            .iter()
            .map(|clip| (clip.start, clip.duration))
            .collect();
        let track_id = super::track_space::allocate(&mut next, kind, Some(&preferred), &ranges);
        let target = next
            .tracks
            .iter_mut()
            .find(|track| track.id == track_id)
            .unwrap();
        target.clips.extend(clips);
    }
    crate::validate_project(&next)?;
    for track in &mut next.tracks {
        track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    Ok(next)
}
