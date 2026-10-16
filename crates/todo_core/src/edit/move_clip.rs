use crate::model::Project;
pub fn move_clip(project: Project, clip_id: &str, start: f64, track_id: Option<&str>) -> Project {
    let Some(found) = crate::find_clip(&project, clip_id) else {
        return project;
    };
    if !start.is_finite() {
        return project;
    }
    let delta = start.max(0.0) - found.clip.start;
    crate::move_clips(project, &[clip_id.to_string()], delta, track_id)
}
