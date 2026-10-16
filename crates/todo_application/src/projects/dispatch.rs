use super::*;

impl Workspace {
    pub fn dispatch(&mut self, action: AppAction) -> AppUpdate {
        match action {
            AppAction::RefreshProjects => {
                let ticket = self.ticket();
                self.pending_list = Some(ticket);
                AppUpdate::effects(vec![Effect::Project {
                    ticket,
                    task: ProjectTask::List,
                }])
            }
            AppAction::CreateProject(name) => self.open(ProjectTask::Create { name }),
            AppAction::OpenProject(name) => self.open(ProjectTask::Load { name }),
            AppAction::DeleteProject(name) => {
                let ticket = self.ticket();
                self.pending_delete = Some(ticket);
                AppUpdate::effects(vec![Effect::Project {
                    ticket,
                    task: ProjectTask::Delete { name },
                }])
            }
            AppAction::ChooseProjectFile => AppUpdate::effects(vec![Effect::ChooseProjectFile]),
            AppAction::ImportProjectFile(path) => {
                let name = path
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "imported".into());
                self.open(ProjectTask::Import { name, path })
            }
            AppAction::ChooseImport => {
                if self.jobs.import.is_some() {
                    return AppUpdate::default();
                }
                AppUpdate::effects(vec![Effect::ChooseMediaFiles])
            }
            AppAction::ImportFiles(paths) => self.import_files(paths),
            AppAction::Save => self.save(AfterSave::Stay),
            AppAction::Back => {
                if self.jobs.import.is_some() {
                    if let Some(editor) = &mut self.editor {
                        editor.notice(Status::WaitingImport);
                    }
                    return AppUpdate::default();
                }
                if self.editor.as_ref().is_some_and(|e| e.dirty()) {
                    self.save(AfterSave::Back)
                } else {
                    self.leave(false)
                }
            }
            AppAction::RequestClose => {
                let mut update = AppUpdate::default();
                if let Some(editor) = &mut self.editor {
                    let _ = editor.apply(EditorCommand::CommitEdit);
                    if editor.playing() {
                        let paused = editor.apply(EditorCommand::TogglePlay);
                        update = self.editor_update(paused);
                    }
                }
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.dirty() || e.importing())
                {
                    self.close_prompt = true;
                    update
                } else {
                    update.effects.extend(self.leave(true).effects);
                    update
                }
            }
            AppAction::ConfirmClose(CloseChoice::Cancel) => {
                self.close_prompt = false;
                AppUpdate::default()
            }
            AppAction::ConfirmClose(CloseChoice::Discard) => self.leave(true),
            AppAction::ConfirmClose(CloseChoice::Save) => {
                if self.jobs.import.is_some() {
                    if let Some(editor) = &mut self.editor {
                        editor.notice(Status::WaitingImport);
                    }
                    AppUpdate::default()
                } else {
                    self.close_prompt = false;
                    self.save(AfterSave::Close)
                }
            }
            AppAction::Editor(command) => match self.editor.as_mut() {
                Some(editor) => {
                    let update = editor.apply(command);
                    self.editor_update(update)
                }
                None => AppUpdate::default(),
            },
            AppAction::RequestAssets { now, requests } => {
                self.now = now;
                let mut effects = Vec::new();
                for request in requests {
                    let ticket = self.ticket();
                    if self.jobs.request(ticket, &request, now) {
                        effects.push(Effect::Media {
                            ticket,
                            task: request.task(),
                        });
                    }
                }
                AppUpdate::effects(effects)
            }
            AppAction::Notice(message) => {
                self.problem("runtime", message);
                AppUpdate::default()
            }
        }
    }
}
