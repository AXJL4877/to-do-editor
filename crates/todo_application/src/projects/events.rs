use super::*;
use crate::text_rasters::RasterOutcome;

impl Workspace {
    pub fn on_event(&mut self, event: AppEvent) -> AppUpdate {
        match event {
            AppEvent::Project { ticket, result } => self.project_event(ticket, result),
            AppEvent::Media { ticket, result } => self.media_event(ticket, result),
        }
    }
    fn project_event(
        &mut self,
        ticket: Ticket,
        result: Result<ProjectOutput, String>,
    ) -> AppUpdate {
        if ticket.session != self.generation {
            return AppUpdate::default();
        }
        if let Some(after) = self.saves.remove(&ticket) {
            return match result {
                Ok(ProjectOutput::Saved(project)) => {
                    if let Some(editor) = &mut self.editor {
                        let _ = editor.apply(EditorCommand::Saved(project));
                        editor.status = None;
                    }
                    // A save acknowledges its own snapshot. Edits can continue while the disk
                    // worker runs, so navigation must also persist the newer revision.
                    if !matches!(after, AfterSave::Stay)
                        && self.editor.as_ref().is_some_and(|e| e.dirty())
                    {
                        return self.save(after);
                    }
                    match after {
                        AfterSave::Stay => AppUpdate::default(),
                        AfterSave::Back => self.leave(false),
                        AfterSave::Close => self.leave(true),
                    }
                }
                Err(message) => {
                    self.problem("save", message);
                    AppUpdate::default()
                }
                _ => AppUpdate::default(),
            };
        }
        if self.pending_list == Some(ticket) {
            self.pending_list = None;
            match result {
                Ok(ProjectOutput::Listed(names)) => self.projects = names,
                Err(message) => self.problem("list", message),
                _ => {}
            }
        } else if self.pending_delete == Some(ticket) {
            self.pending_delete = None;
            match result {
                Ok(ProjectOutput::Deleted) => return self.dispatch(AppAction::RefreshProjects),
                Err(message) => self.problem("delete", message),
                _ => {}
            }
        } else if self.pending_open == Some(ticket) {
            self.pending_open = None;
            match result {
                Ok(ProjectOutput::Opened { name, project }) => {
                    self.generation = self.generation.wrapping_add(1);
                    self.jobs = MediaJobs::default();
                    self.control = Default::default();
                    self.peaks = PeaksCache::default();
                    self.saves.clear();
                    self.raster_jobs.clear();
                    self.editor = Some(EditorSession::new(project, name));
                    self.route = Route::Editor;
                    self.error = None;
                    let mut update = AppUpdate::effects(vec![Effect::ResetPresentation]);
                    update.effects.extend(self.resync_preview().effects);
                    return update;
                }
                Err(message) => self.problem("open", message),
                _ => {}
            }
        }
        AppUpdate::default()
    }
    fn media_event(&mut self, ticket: Ticket, result: Result<MediaOutput, String>) -> AppUpdate {
        if ticket.session != self.generation {
            return AppUpdate::default();
        }
        if self.jobs.import == Some(ticket) {
            return match result {
                Ok(MediaOutput::Imported(item)) => {
                    if self.control.already_imported(ticket, &item.id) {
                        return AppUpdate::default();
                    }
                    let id = item.id.clone();
                    let command = if self.control.append_import(ticket) {
                        EditorCommand::Imported(item)
                    } else {
                        EditorCommand::RegisterMedia(item)
                    };
                    let Some(editor) = self.editor.as_mut() else {
                        return AppUpdate::default();
                    };
                    let editor_update = editor.apply(command);
                    let registered = editor.project().media.iter().any(|media| media.id == id);
                    let error = editor_update.error.as_ref().map(ToString::to_string);
                    if registered {
                        self.control.imported(ticket, id, error);
                    } else {
                        self.control.import_error(
                            ticket,
                            error.unwrap_or_else(|| "Import did not register the media".into()),
                        );
                    }
                    let changed = editor_update.content_changed;
                    let mut update = self.editor_update(editor_update);
                    if changed {
                        update.effects.extend(self.save(AfterSave::Stay).effects);
                    }
                    update
                }
                Ok(MediaOutput::ImportFinished) => {
                    self.control.finish_import(ticket);
                    self.jobs.import = None;
                    self.dispatch(AppAction::Editor(EditorCommand::ImportFinished))
                }
                Ok(MediaOutput::ImportAborted { message }) => {
                    self.control.import_error(ticket, message.clone());
                    self.control.finish_import(ticket);
                    self.jobs.import = None;
                    let update = self.dispatch(AppAction::Editor(EditorCommand::ImportFinished));
                    self.problem("import", message);
                    update
                }
                Err(message) => {
                    self.control.import_error(ticket, message.clone());
                    self.problem("import", message);
                    AppUpdate::default()
                }
                _ => AppUpdate::default(),
            };
        }
        if let Some(digest) = self.raster_jobs.remove(&ticket) {
            let outcome = match result {
                Ok(MediaOutput::TextRaster { path, block }) => {
                    RasterOutcome::Rendered { path, block }
                }
                Ok(MediaOutput::TextRasterSkipped) => RasterOutcome::Skipped,
                Ok(_) => RasterOutcome::Failed,
                Err(message) => {
                    self.problem("text", message);
                    RasterOutcome::Failed
                }
            };
            let Some(editor) = self.editor.as_mut() else {
                return AppUpdate::default();
            };
            let update = editor.text_raster_done(digest, outcome);
            return self.editor_update(update);
        }
        if !self.jobs.complete(ticket, result.is_ok(), self.now) {
            return AppUpdate::default();
        }
        match result {
            Ok(MediaOutput::Peaks {
                path,
                duration,
                peaks,
            }) => {
                self.peaks.insert(path, duration, peaks);
                AppUpdate::default()
            }
            Ok(MediaOutput::Thumbnail { path, bytes }) => {
                AppUpdate::effects(vec![Effect::UploadThumbnail { path, bytes }])
            }
            _ => AppUpdate::default(),
        }
    }
}
