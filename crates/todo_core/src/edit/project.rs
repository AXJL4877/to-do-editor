use crate::{DomainError, MediaScaleMode, Project};

/// Soft caps for user-editable program settings.
pub const MAX_OUTPUT_EDGE: u32 = 8192;
pub const MIN_FPS: f64 = 1.0;
pub const MAX_FPS: f64 = 240.0;

/// Set the fixed workspace length and keep the playhead within its new bounds.
pub fn set_project_duration(mut project: Project, seconds: f64) -> Result<Project, DomainError> {
    if !seconds.is_finite() || seconds < 1.0 {
        return Err(DomainError::InvalidDuration);
    }
    project.duration = Some(seconds);
    project.playhead = project.playhead.min(seconds);
    Ok(project)
}

/// Set the program-canvas pixel size (aspect comes from width/height).
pub fn set_project_output_size(
    mut project: Project,
    width: u32,
    height: u32,
) -> Result<Project, DomainError> {
    if width == 0 || height == 0 || width > MAX_OUTPUT_EDGE || height > MAX_OUTPUT_EDGE {
        return Err(DomainError::InvalidOutputSize);
    }
    project.width = width;
    project.height = height;
    Ok(project)
}

/// Set the project frame rate used by the timeline and playback floor.
pub fn set_project_fps(mut project: Project, fps: f64) -> Result<Project, DomainError> {
    if !fps.is_finite() || fps < MIN_FPS || fps > MAX_FPS {
        return Err(DomainError::InvalidFps);
    }
    project.fps = fps;
    Ok(project)
}

/// Set how newly placed media is scaled onto the program canvas.
pub fn set_media_scale_mode(mut project: Project, mode: MediaScaleMode) -> Project {
    project.media_scale = mode;
    project
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;

    #[test]
    fn output_size_and_fps_reject_invalid_values() {
        let p = create_project(None, None, None, None, None);
        assert!(set_project_output_size(p.clone(), 0, 1080).is_err());
        assert!(set_project_output_size(p.clone(), 1920, 9000).is_err());
        let next = set_project_output_size(p.clone(), 1080, 1920).unwrap();
        assert_eq!(next.output_size(), (1080, 1920));
        assert!(set_project_fps(p.clone(), 0.5).is_err());
        assert!(set_project_fps(p.clone(), 300.0).is_err());
        assert_eq!(set_project_fps(p, 23.976).unwrap().fps, 23.976);
    }
}
