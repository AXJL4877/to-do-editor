use crate::{execute_edit, DomainError, EditCommand, Project};

/// Ask the same policy used by placement commits, including lane spill/creation.
/// The caller's project is never changed by this query.
pub fn can_place_media(
    project: &Project,
    media_id: &str,
    track_id: Option<&str>,
    start: f64,
) -> Result<(), DomainError> {
    execute_edit(
        project.clone(),
        EditCommand::PlaceMedia {
            media_id: media_id.into(),
            track_id: track_id.map(str::to_string),
            time: start,
        },
    )
    .map(|_| ())
}
