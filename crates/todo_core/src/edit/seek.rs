use crate::{project_duration, Project};

pub fn seek(project: Project, time: f64) -> Project {
    let end = project_duration(&project);
    let playhead = time.max(0.0).min(end);
    if (playhead - project.playhead).abs() < f64::EPSILON {
        return project;
    }
    Project {
        playhead,
        ..project
    }
}
