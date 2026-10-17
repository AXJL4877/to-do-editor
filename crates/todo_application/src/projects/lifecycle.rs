use super::*;

impl Workspace {
    pub(super) fn save(&mut self, after: AfterSave) -> AppUpdate {
        let Some(editor) = &mut self.editor else {
            return AppUpdate::default();
        };
        let _ = editor.apply(EditorCommand::CommitEdit);
        let name = editor.project_name().to_owned();
        let project = editor.snapshot();
        editor.notice(Status::Saving);
        let ticket = self.ticket();
        self.saves.insert(ticket, after);
        AppUpdate::effects(vec![Effect::Project {
            ticket,
            task: ProjectTask::Save { name, project },
        }])
    }
    pub(super) fn leave(&mut self, close: bool) -> AppUpdate {
        self.close_prompt = false;
        let mut effects = self.stop_productions();
        if let Some(editor) = &mut self.editor {
            effects.push(Effect::Preview(editor.deactivate()));
        }
        if close {
            effects.push(Effect::CloseWindow);
        } else {
            self.editor = None;
            self.route = Route::Projects;
            self.generation = self.generation.wrapping_add(1);
            self.saves.clear();
            self.jobs = MediaJobs::default();
            self.control = Default::default();
            self.peaks = PeaksCache::default();
            effects.push(Effect::ResetPresentation);
            effects.extend(self.dispatch(AppAction::RefreshProjects).effects);
        }
        AppUpdate::effects(effects)
    }
    pub(super) fn open(&mut self, task: ProjectTask) -> AppUpdate {
        self.error = None;
        let ticket = self.ticket();
        self.pending_open = Some(ticket);
        AppUpdate::effects(vec![Effect::Project { ticket, task }])
    }
    pub(super) fn import_files(&mut self, paths: Vec<std::path::PathBuf>) -> AppUpdate {
        self.start_import(paths, true)
    }
    pub(super) fn start_import(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        append: bool,
    ) -> AppUpdate {
        if paths.is_empty() {
            return AppUpdate::default();
        }
        if self.jobs.import.is_some() {
            if let Some(editor) = &mut self.editor {
                editor.notice(Status::WaitingImport);
            }
            return AppUpdate::default();
        }
        let Some(editor) = &mut self.editor else {
            return AppUpdate::default();
        };
        let project = editor.project_name().to_owned();
        let started = editor.apply(EditorCommand::ImportStarted);
        let ticket = self.ticket();
        self.jobs.import = Some(ticket);
        self.control.begin_import(ticket, paths.len(), append);
        let mut update = self.editor_update(started);
        update.effects.push(Effect::Media {
            ticket,
            task: MediaTask::Import { project, paths },
        });
        update
    }
}
