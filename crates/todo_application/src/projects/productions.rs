//! Productions of the open project: routing adapter reports to sessions,
//! starting what can run, and putting results on the timeline as one undo
//! step per pass. Placement waits while the user is in the middle of a
//! gesture and resumes when the gesture commits/cancels (or on the next
//! production report).
use super::*;
use crate::productions::{
    LocalRunId, ProductionOutput, ProductionSession, ProductionSetup, ProductionStatus,
};
use std::collections::{BTreeMap, BTreeSet};
use todo_core::EditCommand;
use todo_production::production::Edit;
use todo_production::{Production, TaskKey};

#[derive(Debug, Default)]
pub(crate) struct Productions {
    pub(super) sessions: BTreeMap<String, ProductionSession>,
    next_run: LocalRunId,
    /// Repository-relative files being registered as media.
    pub(super) adopting: BTreeSet<String>,
    /// Files whose registration failed; their items stay off the timeline.
    pub(super) unadoptable: BTreeSet<String>,
    adopt_tickets: HashMap<Ticket, Vec<String>>,
    /// Production → the batch the editor last rejected, so it is not retried
    /// on every report.
    pub(super) rejected: BTreeMap<String, String>,
    /// Generated clips reached the editor while another save was on disk:
    /// save again once it lands, or a reopen would see them as deleted.
    pub(super) resave: bool,
    /// Library names for generated files being registered (path → name).
    pub(super) adopt_names: BTreeMap<String, String>,
}

impl Workspace {
    /// Return the directory currently used to resolve this production's
    /// generated files. The path is repository-relative and is shared with
    /// the workflow host so emitted items and media registration agree.
    pub fn production_workdir(&self, id: &str) -> Option<String> {
        self.productions
            .sessions
            .get(id)
            .map(|session| session.setup().workdir.clone())
    }

    /// Move a workflow run onto its durable production directory before any
    /// service writes an artifact. This prevents cache/workdir path skew on
    /// resume.
    pub fn set_production_workdir(&mut self, id: &str, workdir: String) -> Result<(), String> {
        let session = self
            .productions
            .sessions
            .get_mut(id)
            .ok_or_else(|| format!("成片会话不存在：{id}"))?;
        session.set_workdir(workdir);
        Ok(())
    }

    /// Start (or resume) a production of the open project. An existing
    /// session with the same id is replaced.
    pub fn add_production(&mut self, doc: Production, setup: ProductionSetup) -> AppUpdate {
        let Some(editor) = &self.editor else {
            return AppUpdate::default();
        };
        let project = editor.project_name().to_owned();
        let id = doc.id.clone();
        let mut effects = Vec::new();
        if let Some(mut old) = self.productions.sessions.remove(&id) {
            effects.extend(old.stop().into_iter().map(Effect::Production));
        }
        self.productions
            .sessions
            .insert(id, ProductionSession::new(project, doc, setup));
        let mut update = self.advance_productions();
        effects.append(&mut update.effects);
        update.effects = effects;
        update
    }

    pub fn production_status(&self) -> Vec<ProductionStatus> {
        self.productions
            .sessions
            .values()
            .map(ProductionSession::status)
            .collect()
    }

    pub fn production(&self, id: &str) -> Option<&Production> {
        self.productions
            .sessions
            .get(id)
            .map(ProductionSession::doc)
    }

    pub fn production_ids(&self) -> Vec<String> {
        self.productions.sessions.keys().cloned().collect()
    }

    /// Bind a canvas workflow run to a production session so emitted items
    /// reach placement. Returns the local run id the host must use.
    pub fn bind_canvas_workflow(
        &mut self,
        extension: &str,
        extension_version: &str,
        script: &str,
        workdir: String,
        canvas: todo_workflow::workflow::protocol::Canvas,
    ) -> Result<LocalRunId, String> {
        let project = self
            .editor
            .as_ref()
            .ok_or_else(|| "请先打开一个工程".to_string())?
            .project_name()
            .to_owned();
        let existing = self
            .productions
            .sessions
            .iter()
            .find(|(_, s)| s.setup().extension == extension)
            .map(|(id, _)| id.clone());
        let session_id = if let Some(id) = existing {
            id
        } else {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let id = format!("w{stamp:x}");
            let doc = Production::new(
                id.clone(),
                todo_production::PackRef {
                    id: extension.to_owned(),
                    version: extension_version.to_owned(),
                },
                script,
                Default::default(),
                todo_production::SplitMode::Paragraph,
                todo_production::Limits {
                    target: 120,
                    max: 200,
                },
            );
            let setup = ProductionSetup {
                extension: extension.to_owned(),
                recipe: crate::productions::Recipe::Workflow {
                    op: "canvas".into(),
                    input: serde_json::json!({ "script": script }),
                },
                canvas: canvas.clone(),
                workdir: workdir.clone(),
            };
            self.productions.sessions.insert(
                id.clone(),
                ProductionSession::new(project, doc, setup),
            );
            id
        };
        let session = self
            .productions
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| "成片会话丢失".to_string())?;
        session.set_workdir(workdir);
        session.set_canvas(canvas);
        session.set_workflow_script(script);
        Ok(session.attach_canvas_run(&mut self.productions.next_run))
    }

    /// Workdir the canvas session actually uses after bind (may refuse cache switch).
    pub fn canvas_session_workdir(&self, extension: &str) -> Option<String> {
        self.productions
            .sessions
            .values()
            .find(|s| s.setup().extension == extension)
            .map(|s| s.setup().workdir.clone())
            .filter(|w| !w.is_empty())
    }

    /// The user's edits to a production (one batch).
    pub fn edit_production(&mut self, id: &str, edits: &[Edit]) -> AppUpdate {
        let Some(session) = self.productions.sessions.get_mut(id) else {
            return AppUpdate::default();
        };
        if let Err(message) = session.edit(edits) {
            self.problem("production", message);
            return AppUpdate::default();
        }
        self.productions.rejected.remove(id);
        self.productions.unadoptable.clear();
        session.clear_placement_errors();
        self.advance_productions()
    }

    /// Retry one failed request, or (`None`) every failed bake/run.
    pub fn retry_production(&mut self, id: &str, key: Option<TaskKey>) -> AppUpdate {
        let Some(session) = self.productions.sessions.get_mut(id) else {
            return AppUpdate::default();
        };
        let tasks = session.retry(key.as_ref());
        self.productions.rejected.remove(id);
        // Failed adopts are retryable; clear so the next place can re-queue.
        self.productions.unadoptable.clear();
        session.clear_placement_errors();
        let mut update = self.advance_productions();
        update
            .effects
            .splice(0..0, tasks.into_iter().map(Effect::Production));
        update
    }

    /// Put back the production's clips the user deleted from the timeline.
    pub fn restore_production(&mut self, id: &str) -> AppUpdate {
        let Some(session) = self.productions.sessions.get_mut(id) else {
            return AppUpdate::default();
        };
        session.restore_deleted();
        self.productions.rejected.remove(id);
        self.productions.unadoptable.clear();
        session.clear_placement_errors();
        self.advance_productions()
    }

    /// Forget a production (its clips stay on the timeline).
    pub fn remove_production(&mut self, id: &str) -> AppUpdate {
        let Some(mut session) = self.productions.sessions.remove(id) else {
            return AppUpdate::default();
        };
        AppUpdate::effects(session.stop().into_iter().map(Effect::Production).collect())
    }

    /// Stop every production (the project closes or another one opens).
    pub(super) fn stop_productions(&mut self) -> Vec<Effect> {
        let sessions = std::mem::take(&mut self.productions.sessions);
        self.productions = Productions::default();
        sessions
            .into_values()
            .flat_map(|mut s| s.stop())
            .map(Effect::Production)
            .collect()
    }

    pub(super) fn production_event(&mut self, output: ProductionOutput) -> AppUpdate {
        let quiet = matches!(
            output,
            ProductionOutput::RunProgress { .. } | ProductionOutput::TaskState { .. }
        );
        let mut changed = false;
        match &output {
            ProductionOutput::RunProgress { run, .. }
            | ProductionOutput::RunItems { run, .. }
            | ProductionOutput::RunDone { run, .. }
            | ProductionOutput::RunFailed { run, .. } => {
                if let Some(session) = self
                    .productions
                    .sessions
                    .values_mut()
                    .find(|s| s.owns_run(*run))
                {
                    changed = session.on_output(output);
                }
            }
            // A request may serve several productions (same content).
            _ => {
                for session in self.productions.sessions.values_mut() {
                    changed |= session.on_output(output.clone());
                }
            }
        }
        if !changed {
            return AppUpdate::default();
        }
        if quiet {
            return AppUpdate::default();
        }
        self.advance_productions()
    }

    /// Registered (or failed to register) generated media; returns whether
    /// the ticket belonged to productions.
    pub(super) fn production_media(
        &mut self,
        ticket: Ticket,
        ok: bool,
        error: Option<&str>,
    ) -> bool {
        let Some(files) = self.productions.adopt_tickets.remove(&ticket) else {
            return false;
        };
        for file in files {
            self.productions.adopting.remove(&file);
            if !ok {
                let detail = match error {
                    Some(msg) if !msg.trim().is_empty() => {
                        format!("素材登记失败（{file}）：{msg}")
                    }
                    _ => format!("素材登记失败（{file}）"),
                };
                // Prefer the session that still needs this path; if none match
                // (path/workdir skew), still surface on every live session so
                // UI/Bridge never silently stay "completed/running".
                let mut noted = false;
                for session in self.productions.sessions.values_mut() {
                    if session.needs_media_path(&file) {
                        session.note_placement_error(detail.clone());
                        noted = true;
                    }
                }
                if !noted {
                    for session in self.productions.sessions.values_mut() {
                        session.note_placement_error(detail.clone());
                    }
                }
                self.productions.unadoptable.insert(file);
            }
        }
        true
    }

    /// Start what can run and place what is ready.
    pub(super) fn advance_productions(&mut self) -> AppUpdate {
        let mut effects = Vec::new();
        for session in self.productions.sessions.values_mut() {
            effects.extend(
                session
                    .tick(&mut self.productions.next_run)
                    .into_iter()
                    .map(Effect::Production),
            );
        }
        let mut update = self.place_productions();
        effects.append(&mut update.effects);
        update.effects = effects;
        update
    }

    fn place_productions(&mut self) -> AppUpdate {
        let mut update = AppUpdate::default();
        let Some(editor) = self.editor.as_ref() else {
            return update;
        };
        if editor.transaction_active() {
            return update;
        }
        let project_name = editor.project_name().to_owned();
        let ids: Vec<String> = self.productions.sessions.keys().cloned().collect();
        let mut adopt = Vec::new();
        let mut placed_any = false;
        for id in ids {
            let Some(editor) = self.editor.as_mut() else {
                break;
            };
            if let Some(session) = self.productions.sessions.get_mut(&id) {
                session.recover_stale_placement(editor.project());
            }
            let session = &self.productions.sessions[&id];
            let plan = session.placement_plan(editor.project());
            for file in &plan.adopt {
                if !self.productions.adopting.contains(file)
                    && !self.productions.unadoptable.contains(file)
                {
                    adopt.push(file.clone());
                    if let Some(name) = session.media_name(file) {
                        self.productions.adopt_names.insert(file.clone(), name);
                    }
                }
            }
            if plan.is_empty() {
                continue;
            }
            let signature = format!("{:?}", plan.edits);
            if !plan.edits.is_empty() {
                if self.productions.rejected.get(&id) == Some(&signature) {
                    continue;
                }
                let mut edits = plan.edits.clone();
                let copies = crate::control::motions::missing_motions(
                    editor.project(),
                    &self.motions,
                    plan.edits.iter().flat_map(|edit| edit.motions()),
                );
                if !copies.is_empty() {
                    edits.push(todo_core::GeneratedEdit::AddMotions(copies));
                }
                let result = editor.apply(EditorCommand::Edit(EditCommand::ApplyGenerated(edits)));
                if let Some(error) = result.error.clone() {
                    self.productions.rejected.insert(id.clone(), signature);
                    let hint = format!("{error}。入轨已暂停同批重试；请解锁轨道或点「重试」。");
                    editor.notice(Status::External {
                        operation: "placement",
                        message: hint,
                    });
                    if let Some(session) = self.productions.sessions.get_mut(&id) {
                        session.note_placement_error(error.to_string());
                    }
                    continue;
                }
                if let Some(session) = self.productions.sessions.get_mut(&id) {
                    // ApplyGenerated succeeded for this batch. Keep adopt-failure
                    // notes while siblings remain unadoptable (retry clears them).
                    if session.unadoptable_count(&self.productions.unadoptable) == 0 {
                        session.clear_placement_errors();
                    }
                }
                let editor_effects = self.editor_update(result).effects;
                update.effects.extend(editor_effects);
                placed_any = true;
            }
            if let Some(session) = self.productions.sessions.get_mut(&id) {
                session.placed(&plan);
                // Every task of this tick runs: a run started here is
                // already counted as in flight by the session.
                update.effects.extend(
                    session
                        .tick(&mut self.productions.next_run)
                        .into_iter()
                        .map(Effect::Production),
                );
            }
        }
        if !adopt.is_empty() {
            adopt.sort();
            adopt.dedup();
            // Each file has its own acknowledgement. A missing/corrupt sibling
            // must not discard successful probes or delay them until batch end.
            for file in adopt {
                let ticket = self.media_ticket();
                self.productions.adopting.insert(file.clone());
                self.productions.adopt_tickets.insert(ticket, vec![file.clone()]);
                update.effects.push(Effect::Media {
                    ticket,
                    task: MediaTask::Adopt {
                        project: project_name.clone(),
                        files: vec![std::path::PathBuf::from(file)],
                    },
                });
            }
        }
        // Generated clips are only safe once the project file has them: the
        // production remembers what it placed, and a later open must agree.
        if placed_any {
            if self.saves.is_empty() {
                update.effects.extend(self.save(AfterSave::Stay).effects);
            } else {
                self.productions.resave = true;
            }
        }
        update
    }
}
