use crate::{clip_end, Project, DEFAULT_WORKSPACE_SECS};

/// Fixed workspace length shown on the timeline ruler (composition duration).
pub fn workspace_duration(project: &Project) -> f64 {
    project
        .duration
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(DEFAULT_WORKSPACE_SECS)
}

/// Playback / sequence end is the workspace composition length.
pub fn project_duration(project: &Project) -> f64 {
    workspace_duration(project)
}

/// End of placed clips, independent of the configured composition duration.
pub fn content_duration(project: &Project) -> f64 {
    project
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .map(clip_end)
        .fold(0.0_f64, f64::max)
}

/// Scrollable editing range includes overflow while playback keeps its own end.
pub fn timeline_extent(project: &Project) -> f64 {
    workspace_duration(project).max(content_duration(project))
}
