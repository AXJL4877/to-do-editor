//! Pure track management; creating an empty track never performs cleanup.
use crate::{DomainError, Project, Track, TrackKind};

/// Stable partition: Text block → Video block → Audio block → Sfx block.
/// Relative order inside each kind is preserved.
pub fn ensure_track_sections(mut project: Project) -> Project {
    project
        .tracks
        .sort_by_key(|track| track.kind.section_rank());
    project
}

/// Index after the last track of `kind`, or the start of that kind's section if none exist.
fn insert_index_for_kind(project: &Project, kind: TrackKind) -> usize {
    let rank = kind.section_rank();
    if let Some(last) = project.tracks.iter().rposition(|track| track.kind == kind) {
        return last + 1;
    }
    project
        .tracks
        .iter()
        .position(|track| track.kind.section_rank() > rank)
        .unwrap_or(project.tracks.len())
}

/// Insert an empty track of the anchor's exact kind, preserving all existing tracks.
/// A locked anchor remains valid because its content is not changed.
pub fn insert_track_below(mut project: Project, track_id: &str) -> Result<Project, DomainError> {
    let anchor_index = project
        .tracks
        .iter()
        .position(|track| track.id == track_id)
        .ok_or_else(|| DomainError::TrackNotFound(track_id.into()))?;
    let track = empty_track(&project, project.tracks[anchor_index].kind, true);
    project.tracks.insert(anchor_index + 1, track);
    Ok(project)
}

pub fn add_track(mut project: Project, kind: TrackKind) -> Project {
    add_empty_track(&mut project, kind);
    project
}

pub(super) fn add_empty_track(project: &mut Project, kind: TrackKind) -> String {
    let index = insert_index_for_kind(project, kind);
    let track = empty_track(project, kind, false);
    let id = track.id.clone();
    project.tracks.insert(index, track);
    project
        .tracks
        .sort_by_key(|track| track.kind.section_rank());
    id
}

/// Swap `track_id` with the adjacent same-kind neighbor in `up` direction.
/// Cross-kind moves are rejected so sections keep their order.
pub fn move_track(mut project: Project, track_id: &str, up: bool) -> Result<Project, DomainError> {
    let index = project
        .tracks
        .iter()
        .position(|track| track.id == track_id)
        .ok_or_else(|| DomainError::TrackNotFound(track_id.into()))?;
    let neighbor = if up {
        index.checked_sub(1)
    } else if index + 1 < project.tracks.len() {
        Some(index + 1)
    } else {
        None
    }
    .ok_or(DomainError::TrackOrder)?;
    if project.tracks[neighbor].kind != project.tracks[index].kind {
        return Err(DomainError::TrackOrder);
    }
    project.tracks.swap(index, neighbor);
    Ok(project)
}

/// Whether `track_id` can move toward the top (smaller index) within its kind.
pub fn can_move_track_up(project: &Project, track_id: &str) -> bool {
    let Some(index) = project.tracks.iter().position(|t| t.id == track_id) else {
        return false;
    };
    index
        .checked_sub(1)
        .is_some_and(|prev| project.tracks[prev].kind == project.tracks[index].kind)
}

/// Whether `track_id` can move toward the bottom (larger index) within its kind.
pub fn can_move_track_down(project: &Project, track_id: &str) -> bool {
    let Some(index) = project.tracks.iter().position(|t| t.id == track_id) else {
        return false;
    };
    index + 1 < project.tracks.len() && project.tracks[index + 1].kind == project.tracks[index].kind
}

fn empty_track(project: &Project, kind: TrackKind, unique_name: bool) -> Track {
    let (prefix, label) = match kind {
        TrackKind::Text => ("t", "文字"),
        TrackKind::Video => ("v", "视频"),
        TrackKind::Audio => ("a", "音频"),
        TrackKind::Sfx => ("sfx", "音效"),
    };
    let mut number = 1;
    while project.tracks.iter().any(|t| {
        t.id == format!("{prefix}{number}") || (unique_name && t.name == format!("{label}{number}"))
    }) {
        number += 1;
    }
    Track {
        id: format!("{prefix}{number}"),
        name: format!("{label}{number}"),
        kind,
        muted: false,
        locked: false,
        clips: vec![],
    }
}

pub fn ensure_av_tracks(mut project: Project) -> Project {
    project = ensure_track_sections(project);
    for kind in [TrackKind::Video, TrackKind::Audio] {
        if !project.tracks.iter().any(|t| t.kind == kind) {
            project = add_track(project, kind);
        }
    }
    ensure_track_sections(project)
}

/// Remove extra unlocked empty tracks after a destructive clip edit.
pub fn cleanup_empty_tracks(project: Project) -> Project {
    let mut project = ensure_av_tracks(project);
    // Editing tracks never changes the configured composition duration.
    let mut videos = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .count();
    let mut audios = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .count();
    project.tracks.retain(|t| {
        if t.locked || !t.clips.is_empty() {
            return true;
        }
        let count = match t.kind {
            TrackKind::Video => &mut videos,
            TrackKind::Audio => &mut audios,
            TrackKind::Text | TrackKind::Sfx => return false,
        };
        if *count <= 1 {
            return true;
        }
        *count -= 1;
        false
    });
    ensure_track_sections(project)
}

pub fn set_track_locked(mut project: Project, track_id: &str, locked: bool) -> Project {
    if let Some(track) = project.tracks.iter_mut().find(|t| t.id == track_id) {
        track.locked = locked;
    }
    project
}

pub fn set_track_muted(mut project: Project, track_id: &str, muted: bool) -> Project {
    if let Some(track) = project.tracks.iter_mut().find(|t| t.id == track_id) {
        track.muted = muted;
    }
    project
}
