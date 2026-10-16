use super::place_media::unique_id;
use super::validate_placement;
use crate::{add_track, validate_project, Clip, DomainError, Project, TrackKind};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct CopiedClip {
    pub track_id: String,
    pub kind: TrackKind,
    pub clip: Clip,
}

/// Paste fresh instances atomically. Missing original lanes get new compatible lanes.
pub fn paste_clips(
    mut project: Project,
    clips: &[CopiedClip],
    time: f64,
) -> Result<Project, DomainError> {
    validate_project(&project)?;
    if clips.is_empty() {
        return Err(DomainError::InvalidProject("clipboard_empty"));
    }
    if !time.is_finite() || time < 0.0 {
        return Err(DomainError::InvalidTime);
    }
    let first = clips
        .iter()
        .map(|c| c.clip.start)
        .fold(f64::INFINITY, f64::min);
    if !first.is_finite() {
        return Err(DomainError::InvalidTime);
    }
    let mut targets = HashMap::<String, String>::new();
    let existing_tracks: HashSet<_> = project.tracks.iter().map(|t| t.id.clone()).collect();
    let mut links = HashMap::<String, String>::new();
    let mut copied_ids = HashSet::new();
    for copied in clips {
        if !copied_ids.insert(&copied.clip.id) {
            return Err(DomainError::DuplicateId(copied.clip.id.clone()));
        }
        if copied.track_id.is_empty() {
            return Err(DomainError::InvalidProject("track_id"));
        }
        let target = if let Some(id) = targets.get(&copied.track_id) {
            id.clone()
        } else {
            let id = if let Some(track) = project
                .tracks
                .iter()
                .find(|t| t.id == copied.track_id && existing_tracks.contains(&t.id))
            {
                if track.locked {
                    return Err(DomainError::TrackLocked(track.id.clone()));
                }
                if track.kind != copied.kind {
                    return Err(DomainError::IncompatibleTrack(track.id.clone()));
                }
                track.id.clone()
            } else {
                let before: HashSet<_> = project.tracks.iter().map(|t| t.id.clone()).collect();
                project = add_track(project, copied.kind);
                project
                    .tracks
                    .iter()
                    .find(|t| !before.contains(&t.id))
                    .expect("add_track inserts one track")
                    .id
                    .clone()
            };
            targets.insert(copied.track_id.clone(), id.clone());
            id
        };
        let track = project.tracks.iter().find(|t| t.id == target).unwrap();
        if track.kind != copied.kind {
            return Err(DomainError::IncompatibleTrack(target));
        }
        // Validate clipboard source data before replacing identity/timeline position.
        super::validate_clip(&project, &copied.clip, track.kind)?;
        let mut clip = copied.clip.clone();
        clip.id = unique_id(&project);
        clip.start = time + copied.clip.start - first;
        if let Some(original) = &copied.clip.link_id {
            let link = links
                .entry(original.clone())
                .or_insert_with(|| unique_id(&project));
            clip.link_id = Some(link.clone());
        }
        validate_placement(&project, track, &clip, &[])?;
        project
            .tracks
            .iter_mut()
            .find(|t| t.id == target)
            .unwrap()
            .clips
            .push(clip);
    }
    for track in &mut project.tracks {
        track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    validate_project(&project)?;
    Ok(project)
}
