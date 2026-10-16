use super::*;
use todo_core::{execute_edit, register_media, validate_project, EditCommand, MediaItem};

impl EditorSession {
    /// Registration and optional placement share validation and one history entry.
    pub(super) fn import_media(&mut self, media: MediaItem, append: bool) -> EditorUpdate {
        self.finish_edit();
        self.status = None;
        let id = media.id.clone();
        let registered = register_media(self.project.clone(), media);
        if let Err(error) = validate_project(&registered) {
            self.status = Some(Status::Domain(error.clone()));
            return EditorUpdate {
                error: Some(error),
                ..Default::default()
            };
        }
        if !append {
            return self.commit_project(registered);
        }

        let time = todo_core::content_duration(&registered);
        match execute_edit(
            registered.clone(),
            EditCommand::PlaceMedia {
                media_id: id,
                track_id: None,
                time,
            },
        ) {
            Ok(mut placed) => {
                placed.playhead = time.min(todo_core::project_duration(&placed));
                self.commit_project(placed)
            }
            Err(error) => {
                // A valid imported file remains available even if automatic placement fails.
                let mut update = self.commit_project(registered);
                self.status = Some(Status::Domain(error.clone()));
                update.error = Some(error);
                update
            }
        }
    }
}
