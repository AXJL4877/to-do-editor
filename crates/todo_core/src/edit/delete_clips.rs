use crate::{cleanup_empty_tracks, clip_end, Project};
use std::collections::HashSet;

/// Delete exactly these clip IDs. Link expansion belongs to selection handling.
/// Ripple rejects an interval intersecting unselected material, preserving it atomically.
pub fn delete_clips(mut project: Project, clip_ids: &[String], ripple: bool) -> Project {
    let ids: HashSet<&str> = clip_ids.iter().map(String::as_str).collect();
    let selected: Vec<_> = project
        .tracks
        .iter()
        .flat_map(|t| {
            t.clips
                .iter()
                .filter(|c| ids.contains(c.id.as_str()))
                .map(move |c| (t.locked, c))
        })
        .collect();
    if selected.is_empty()
        || selected.len() != ids.len()
        || selected.iter().any(|(locked, _)| *locked)
    {
        return project;
    }
    let mut intervals = Vec::<(f64, f64)>::new();
    if ripple {
        let mut ranges: Vec<_> = selected
            .iter()
            .map(|(_, c)| (c.start, clip_end(c)))
            .collect();
        if ranges
            .iter()
            .any(|(s, e)| !s.is_finite() || !e.is_finite() || *s < 0.0 || e <= s)
        {
            return project;
        }
        ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (start, end) in ranges {
            if let Some(last) = intervals.last_mut().filter(|last| start <= last.1) {
                last.1 = last.1.max(end);
            } else {
                intervals.push((start, end));
            }
        }
        for track in &project.tracks {
            for clip in &track.clips {
                if ids.contains(clip.id.as_str()) {
                    continue;
                }
                if intervals
                    .iter()
                    .any(|(s, e)| clip.start < *e && clip_end(clip) > *s)
                    || (track.locked && intervals.iter().any(|(_, e)| *e <= clip.start))
                {
                    return project;
                }
            }
        }
    }
    for track in &mut project.tracks {
        track.clips.retain(|c| !ids.contains(c.id.as_str()));
        if ripple {
            for clip in &mut track.clips {
                let removed: f64 = intervals
                    .iter()
                    .filter(|(_, e)| *e <= clip.start)
                    .map(|(s, e)| e - s)
                    .sum();
                clip.start -= removed;
            }
        }
    }
    cleanup_empty_tracks(project)
}
