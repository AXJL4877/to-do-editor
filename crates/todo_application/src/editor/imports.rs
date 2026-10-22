use super::*;
use crate::history::same_content;
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

    /// Register production-adopted media without recording history. The next
    /// ApplyGenerated is the user's undo step for that placement batch.
    pub(super) fn adopt_media(&mut self, media: MediaItem) -> EditorUpdate {
        self.finish_edit();
        self.status = None;
        let registered = register_media(self.project.clone(), media);
        if let Err(error) = validate_project(&registered) {
            self.status = Some(Status::Domain(error.clone()));
            return EditorUpdate {
                error: Some(error),
                ..Default::default()
            };
        }
        let content_changed = !same_content(&self.project, &registered);
        if content_changed {
            self.revision = self.revision.wrapping_add(1);
        }
        let moved = playhead_changed(registered.playhead, self.synced_playhead);
        self.project = registered;
        if moved {
            self.playhead = self.project.playhead;
            self.synced_playhead = self.playhead;
        }
        EditorUpdate {
            commands: self.emit(Want {
                audio: true,
                transport: moved,
                picture: true,
            }),
            new_clips: Vec::new(),
            content_changed,
            error: None,
        }
    }
}
