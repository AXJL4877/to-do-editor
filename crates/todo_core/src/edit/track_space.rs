//! Shared lane allocation for library placement and timeline moves.
use super::validate::TIME_EPSILON;
use crate::{clip_end, Project, Track, TrackKind};

pub(super) fn free(track: &Track, start: f64, duration: f64) -> bool {
    !track.locked
        && track.clips.iter().all(|clip| {
            clip_end(clip) <= start + TIME_EPSILON || clip.start >= start + duration - TIME_EPSILON
        })
}

/// Try the requested lane, then another of the exact kind, then create one.
/// Ranges describe each selected clip separately so gaps remain available.
pub(super) fn allocate(
    project: &mut Project,
    kind: TrackKind,
    preferred: Option<&str>,
    ranges: &[(f64, f64)],
) -> String {
    let fits = |track: &&Track| {
        track.kind == kind
            && ranges
                .iter()
                .all(|&(start, duration)| free(track, start, duration))
    };
    if let Some(track) = project
        .tracks
        .iter()
        .find(|track| Some(track.id.as_str()) == preferred)
        .filter(fits)
    {
        return track.id.clone();
    }
    if let Some(track) = project.tracks.iter().find(fits) {
        return track.id.clone();
    }
    super::tracks::add_empty_track(project, kind)
}
