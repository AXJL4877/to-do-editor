use super::*;
use crate::control::{batch::run_batch, *};

impl Workspace {
    /// Library motion presets (user and extensions) clips may name; the
    /// built-ins are always available. A preset is copied into a project the
    /// first time an edit or production uses it.
    pub fn set_motion_library(&mut self, presets: Vec<todo_core::MotionPreset>) {
        self.motions = presets
            .into_iter()
            .filter(|preset| !todo_core::motion::is_builtin_motion(&preset.id))
            .collect();
    }

    pub fn motion_library(&self) -> &[todo_core::MotionPreset] {
        &self.motions
    }

    /// Read one consistent snapshot on the workspace-owning thread.
    pub fn control_context(&self) -> Option<ControlContext> {
        let editor = self.editor.as_ref()?;
        let mut project = editor.project().clone();
        project.playhead = editor.playhead();
        Some(ControlContext {
            api_version: CONTROL_API_VERSION,
            session_id: self.generation,
            revision: editor.revision(),
            project_name: editor.project_name().into(),
            project,
            playhead: editor.playhead(),
            playing: editor.playing(),
            dirty: editor.dirty(),
            can_undo: editor.can_undo(),
            can_redo: editor.can_redo(),
            busy: self.control_busy(),
            imports: self.control.jobs(),
        })
    }

    /// Execute a shared capability. The caller must process `outcome.update`
    /// through the existing runtime, including worker events and preview updates.
    pub fn control(&mut self, request: ControlRequest) -> ControlOutcome {
        if let Err(error) = self.check_control_envelope(&request) {
            return self.control_reply(&request, Err(error), AppUpdate::default());
        }
        if request.command.is_mutation() {
            match self.control.replay(&request) {
                Ok(Some(response)) => {
                    return ControlOutcome {
                        response,
                        update: AppUpdate::default(),
                    }
                }
                Err(error) => {
                    return self.control_reply(&request, Err(error), AppUpdate::default())
                }
                Ok(None) => {}
            }
        }
        let (result, update) = match self.execute_control(&request) {
            Ok((result, update)) => (Ok(result), update),
            Err(error) => (Err(error), AppUpdate::default()),
        };
        let outcome = self.control_reply(&request, result, update);
        if request.command.is_mutation() {
            self.control.remember(request, outcome.response.clone());
        }
        outcome
    }

    fn check_control_envelope(&self, request: &ControlRequest) -> Result<(), ControlError> {
        if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
            return Err(ControlError::new(
                "invalid_request",
                "requestId must contain 1..128 nonblank bytes",
            ));
        }
        // Agents bootstrap with get_context (often sessionId=0) to learn the
        // live session — do not require a match for that command alone.
        if request.session_id != self.generation
            && !matches!(request.command, ControlCommand::GetContext)
        {
            return Err(ControlError::new(
                "session_conflict",
                "Read a new context before controlling this project session",
            ));
        }
        if self.editor.is_none() {
            return Err(ControlError::new(
                "no_project",
                "Open a project before using editing capabilities",
            ));
        }
        Ok(())
    }

    fn control_busy(&self) -> bool {
        self.close_prompt
            || self.pending_open.is_some()
            || self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.transaction_active() || editor.importing())
    }

    fn execute_control(
        &mut self,
        request: &ControlRequest,
    ) -> Result<(ControlResult, AppUpdate), ControlError> {
        if request.command.is_mutation() {
            let revision = request.expected_revision.ok_or_else(|| {
                ControlError::new("revision_required", "A mutation requires expectedRevision")
            })?;
            if self.editor.as_ref().unwrap().revision() != revision {
                return Err(ControlError::new(
                    "revision_conflict",
                    "Project content changed; read a new context and replan",
                ));
            }
            if self.control_busy() {
                return Err(ControlError::new(
                    "editor_busy",
                    "Wait for the active gesture, import or project transition",
                ));
            }
        }
        match &request.command {
            ControlCommand::GetContext => Ok((
                ControlResult::Context {
                    context: Box::new(self.control_context().unwrap()),
                },
                AppUpdate::default(),
            )),
            ControlCommand::Describe { at } => {
                let editor = self.editor.as_ref().expect("checked above");
                let mut project = editor.project().clone();
                project.playhead = editor.playhead();
                Ok((
                    ControlResult::Document {
                        document: Box::new(crate::control::project_view(&project, *at)),
                    },
                    AppUpdate::default(),
                ))
            }
            ControlCommand::GetImportJob { job_id } => {
                let job = self.control.job(*job_id).ok_or_else(|| {
                    ControlError::new(
                        "job_not_found",
                        "No retained import job has this ID in the current session",
                    )
                })?;
                Ok((ControlResult::ImportJob { job }, AppUpdate::default()))
            }
            ControlCommand::ImportMedia { paths, folder_id } => {
                if paths.is_empty()
                    || paths.len() > 256
                    || paths.iter().any(|path| path.as_os_str().is_empty())
                {
                    return Err(ControlError::new(
                        "invalid_request",
                        "Import requires 1..256 nonempty file paths",
                    ));
                }
                if let Some(id) = folder_id {
                    if self
                        .editor
                        .as_ref()
                        .is_none_or(|editor| !editor.project().folders.iter().any(|f| f.id == *id))
                    {
                        return Err(ControlError::new(
                            "folder_not_found",
                            "Import folder_id must refer to an existing library folder",
                        ));
                    }
                }
                self.set_import_folder(folder_id.clone());
                let update = self.start_import(paths.clone(), false);
                let ticket = self.jobs.import.unwrap();
                let job = self.control.job(ticket.id).unwrap();
                Ok((ControlResult::ImportStarted { job }, update))
            }
            ControlCommand::ApplyEdits { edits, dry_run } => {
                let editor = self.editor.as_mut().unwrap();
                let (project, receipt) =
                    run_batch(editor.project(), edits, *dry_run, &self.motions)?;
                let update = if *dry_run || !receipt.changed {
                    AppUpdate::default()
                } else {
                    let committed = editor.commit_project(project);
                    self.editor_update(committed)
                };
                Ok((ControlResult::Edits { receipt }, update))
            }
            ControlCommand::Undo | ControlCommand::Redo => {
                let command = if matches!(request.command, ControlCommand::Undo) {
                    EditorCommand::Undo
                } else {
                    EditorCommand::Redo
                };
                let update = self.editor.as_mut().unwrap().apply(command);
                let changed = update.content_changed;
                Ok((
                    ControlResult::History { changed },
                    self.editor_update(update),
                ))
            }
            ControlCommand::GetProductionContext { production_id } => {
                Ok((self.production_context_result(production_id.as_deref())?, AppUpdate::default()))
            }
            ControlCommand::GetWorkflowRun {
                run_id,
                production_id,
            } => Ok((
                self.workflow_run_result(production_id.as_deref(), *run_id)?,
                AppUpdate::default(),
            )),
            ControlCommand::ListExtensions
            | ControlCommand::StartWorkflow { .. }
            | ControlCommand::CancelWorkflow { .. }
            | ControlCommand::RetryWorkflow { .. }
            | ControlCommand::ImportExtension { .. } => Err(ControlError::new(
                "workflow_host_required",
                "此命令需由正在运行的桌面宿主（Agent Bridge）执行，与 UI 共用 ProductionHost；请通过 Bridge 发送，勿直接改 JSON",
            )),
        }
    }

    fn production_context_result(
        &self,
        production_id: Option<&str>,
    ) -> Result<ControlResult, ControlError> {
        let mut productions = Vec::new();
        for status in self.production_status() {
            if production_id.is_some_and(|id| id != status.id) {
                continue;
            }
            let Some(doc) = self.production(&status.id) else {
                continue;
            };
            let local_run_ids: Vec<u64> = self
                .productions
                .sessions
                .get(&status.id)
                .map(|s| s.runs().collect())
                .unwrap_or_default();
            let has_live_run = !local_run_ids.is_empty();
            let blocked = self.productions.rejected.contains_key(&status.id);
            let session = self.productions.sessions.get(&status.id);
            let unadoptable = session
                .map(|s| s.unadoptable_count(&self.productions.unadoptable))
                .unwrap_or(0);
            let pending_adopt = session
                .map(|s| {
                    self.productions
                        .adopting
                        .iter()
                        .filter(|path| s.needs_media_path(path))
                        .count()
                })
                .unwrap_or(0);
            productions.push(crate::control::snapshot_from_with_placement(
                &status,
                doc,
                local_run_ids,
                has_live_run,
                blocked,
                unadoptable,
                pending_adopt,
            ));
        }
        if let Some(id) = production_id {
            if productions.is_empty() {
                return Err(ControlError::new(
                    "production_not_found",
                    &format!("没有成片会话 {id}"),
                ));
            }
        }
        Ok(ControlResult::ProductionContext {
            context: ProductionContextView { productions },
        })
    }

    fn workflow_run_result(
        &self,
        production_id: Option<&str>,
        run_id: Option<u64>,
    ) -> Result<ControlResult, ControlError> {
        let id = if let Some(pid) = production_id {
            pid.to_owned()
        } else if let Some(run) = run_id {
            self.productions
                .sessions
                .iter()
                .find(|(_, s)| s.owns_run(run))
                .map(|(id, _)| id.clone())
                .ok_or_else(|| {
                    ControlError::new("run_not_found", &format!("没有本地 runId={run}"))
                })?
        } else {
            self.productions
                .sessions
                .keys()
                .next()
                .cloned()
                .ok_or_else(|| {
                    ControlError::new(
                        "production_not_found",
                        "当前工程没有成片/工作流会话；先 start_workflow 或打开已有成片",
                    )
                })?
        };
        let status = self
            .productions
            .sessions
            .get(&id)
            .map(|s| s.status())
            .ok_or_else(|| ControlError::new("production_not_found", &format!("没有成片 {id}")))?;
        let doc = self
            .production(&id)
            .ok_or_else(|| ControlError::new("production_not_found", &format!("没有成片 {id}")))?;
        let local_run_ids: Vec<u64> = self
            .productions
            .sessions
            .get(&id)
            .map(|s| s.runs().collect())
            .unwrap_or_default();
        let has_live_run = !local_run_ids.is_empty();
        let blocked = self.productions.rejected.contains_key(&id);
        let session = self.productions.sessions.get(&id);
        let unadoptable = session
            .map(|s| s.unadoptable_count(&self.productions.unadoptable))
            .unwrap_or(0);
        let pending_adopt = session
            .map(|s| {
                self.productions
                    .adopting
                    .iter()
                    .filter(|path| s.needs_media_path(path))
                    .count()
            })
            .unwrap_or(0);
        let snap = crate::control::snapshot_from_with_placement(
            &status,
            doc,
            local_run_ids.clone(),
            has_live_run,
            blocked,
            unadoptable,
            pending_adopt,
        );
        let phase = status
            .stages
            .iter()
            .find_map(|s| s.phase.as_ref().map(|(p, f)| (p.clone(), *f)));
        Ok(ControlResult::WorkflowRun {
            run: WorkflowRunView {
                production_id: snap.production_id,
                extension_id: snap.extension_id,
                run_id: run_id.or_else(|| local_run_ids.last().copied()),
                run_state: snap.run_state,
                phase: phase.as_ref().map(|(p, _)| p.clone()),
                fraction: phase.map(|(_, f)| f),
                error: snap.workflow_error,
                placement: snap.placement,
                nodes: snap.nodes,
            },
        })
    }

    fn control_reply(
        &self,
        request: &ControlRequest,
        result: Result<ControlResult, ControlError>,
        update: AppUpdate,
    ) -> ControlOutcome {
        ControlOutcome {
            update,
            response: ControlResponse {
                api_version: CONTROL_API_VERSION,
                request_id: request.request_id.clone(),
                session_id: self.generation,
                revision: self.editor.as_ref().map(EditorSession::revision),
                outcome: match result {
                    Ok(result) => ControlStatus::Ok { result },
                    Err(error) => ControlStatus::Error { error },
                },
            },
        }
    }
}
