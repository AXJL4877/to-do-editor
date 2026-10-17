//! Rubber-band (marquee) multi-select helpers.

use todo_core::{linked_clip_ids, Project};

/// Clips on unlocked tracks whose time span intersects `[t0, t1]` and whose
/// track index is in `[track_lo, track_hi]` (inclusive).
pub fn clips_in_marquee(
    project: &Project,
    t0: f64,
    t1: f64,
    track_lo: usize,
    track_hi: usize,
) -> Vec<String> {
    let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
    let (track_lo, track_hi) = if track_lo <= track_hi {
        (track_lo, track_hi)
    } else {
        (track_hi, track_lo)
    };
    let mut ids = Vec::new();
    for (index, track) in project.tracks.iter().enumerate() {
        if index < track_lo || index > track_hi || track.locked {
            continue;
        }
        for clip in &track.clips {
            let end = clip.start + clip.duration;
            if end > t0 && clip.start < t1 {
                ids.push(clip.id.clone());
            }
        }
    }
    ids
}

/// Selection produced by a marquee rectangle in time × track-index space.
#[allow(clippy::too_many_arguments)] // Explicit timeline geometry at this projection/input boundary.
pub fn marquee_selection(
    project: &Project,
    t0: f64,
    t1: f64,
    track_lo: usize,
    track_hi: usize,
    linked: bool,
    additive: bool,
    base_selection: &[String],
) -> Vec<String> {
    let mut hits = clips_in_marquee(project, t0, t1, track_lo, track_hi);
    if linked {
        hits = linked_clip_ids(project, &hits);
    }
    if !additive {
        return hits;
    }
    let mut ids = base_selection.to_vec();
    for id in hits {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::{add_clip, create_project, normalize_clip, MediaItem};

    fn two_clips() -> Project {
        let mut project = create_project(None, None, Some(10.0), None, None);
        project.media.push(MediaItem {
            id: "m1".into(),
            name: "a.mp4".into(),
            path: None,
            duration: 4.0,
            has_audio: Some(true),
            width: Some(16),
            height: Some(9),
            kind: None,
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        project = add_clip(
            project,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 0.0, 2.0, 0.0, 1.0),
        );
        add_clip(
            project,
            "v1",
            normalize_clip("c2".into(), "m1".into(), 3.0, 2.0, 0.0, 1.0),
        )
    }

    #[test]
    fn marquee_hits_intersecting_clips_only() {
        let project = two_clips();
        let hits = clips_in_marquee(&project, 1.5, 3.5, 0, 0);
        assert_eq!(hits, vec!["c1".to_string(), "c2".to_string()]);
        let only_second = clips_in_marquee(&project, 2.5, 4.0, 0, 0);
        assert_eq!(only_second, vec!["c2".to_string()]);
    }
}
