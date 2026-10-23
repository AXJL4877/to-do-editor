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
                if self.editor.is_some() {
                    if let Some(editor) = self.editor.as_mut() {
                        let _ = editor.apply(EditorCommand::CommitEdit);
                    }
                    let placed = self.advance_productions();
                    update.effects.extend(placed.effects);
                    update.new_clips.extend(placed.new_clips);
                    if self.editor.as_ref().is_some_and(|e| e.playing()) {
                        let paused = self
                            .editor
                            .as_mut()
                            .map(|e| e.apply(EditorCommand::TogglePlay))
                            .unwrap_or_default();
                        let next = self.editor_update(paused);
                        update.effects.extend(next.effects);
                        update.new_clips.extend(next.new_clips);
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
                    // Placement pauses during a gesture; resume as soon as the
                    // user commits or cancels so a live workflow can keep
                    // placing. Idle sessions are disarmed — this only syncs
                    // deletion tombstones, never forces adopt/Place.
                    let resume_placement = matches!(
                        command,
                        EditorCommand::CommitEdit | EditorCommand::CancelEdit
                    );
                    let update = editor.apply(command);
                    let mut out = self.editor_update(update);
                    if resume_placement {
                        let next = self.advance_productions();
                        out.effects.extend(next.effects);
                        out.new_clips.extend(next.new_clips);
                    }
                    out
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
            AppAction::ExportTimeline { output, preset } => {
                let Some(project) = self.editor.as_ref().map(|editor| editor.project().clone())
                else {
                    return AppUpdate::default();
                };
                if preset == "vertical" && project.width >= project.height {
                    self.problem("export", "竖屏预设要求工程画布为竖屏".into());
                    return AppUpdate::default();
                }
                let ticket = self.ticket();
                AppUpdate::effects(vec![Effect::Media {
                    ticket,
                    task: MediaTask::ExportTimeline {
                        project: Box::new(project),
                        output,
                        preset,
                    },
                }])
            }
            AppAction::Notice(message) => {
                self.problem("runtime", message);
                AppUpdate::default()
            }
        }
    }
}
