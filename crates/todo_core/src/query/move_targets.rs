use crate::{DomainError, Project};

/// Check the same atomic move used by commands without changing the supplied project.
pub fn can_move_clips(
    project: &Project,
    clip_ids: &[String],
    target: &str,
) -> Result<(), DomainError> {
    crate::edit::move_clips::try_move_clips(project.clone(), clip_ids, 0.0, Some(target))
        .map(|_| ())
}
