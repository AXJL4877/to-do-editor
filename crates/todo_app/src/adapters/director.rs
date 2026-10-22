//! Runs the application's [`Director`] against the real services: language
//! and multimodal calls through the production host, frames through the
//! offscreen renderer, productions through the host and workspace, edits
//! through `Workspace::control`. The editor and the headless runner share it.
use super::production::host::ProductionHost;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use todo_application::control::{
    ControlCommand, ControlEdit, ControlRequest, ControlResult, ControlStatus,
};
use todo_application::director::{Call, Director, Input, PictureSource};
use todo_application::{AppUpdate, Workspace};
use todo_interaction::ParamValue;
use todo_workflow::workflow::protocol::Canvas;
use todo_production::{ImageRequest, LanguageRequest, MultimodalRequest, TaskRequest};

type Frames = Result<Vec<(f64, String)>, String>;

pub struct DirectorDriver {
    director: Director,
    root: PathBuf,
    /// The production being made (or rerun), until it is placed.
    production: Option<String>,
    /// The production `Produce` made, for reruns.
    made: Option<String>,
    frames: Option<(String, mpsc::Receiver<Frames>)>,
    /// A picture being generated (request id).
    drawing: Option<String>,
    /// A picture being imported: (request id, import job).
    importing: Option<(String, u64)>,
}

impl DirectorDriver {
    /// Start on the open project; the returned update must be handled.
    pub fn start(
        root: PathBuf,
        request: String,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> Result<(Self, AppUpdate), String> {
        let project = workspace
            .editor()
            .ok_or("请先打开一个工程")?
            .project()
            .clone();
        let run = format!(
            "director-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis())
        );
        let (director, calls) = Director::start(run, request, host.styles(), &project);
        let mut driver = Self {
            director,
            root,
            production: None,
            made: None,
            frames: None,
            drawing: None,
            importing: None,
        };
        let update = driver.run(calls, workspace, host);
        Ok((driver, update))
    }

    /// Start the built-in editor agent without selecting an extension.
    /// Production templates are launched explicitly from the production panel.
    pub fn start_edit(
        root: PathBuf,
        request: String,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> Result<(Self, AppUpdate), String> {
        let project = workspace
            .editor()
            .ok_or("请先打开一个工程")?
            .project()
            .clone();
        let run = format!(
            "director-edit-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis())
        );
        let (director, calls) = Director::start_edit(run, request, &project);
        let mut driver = Self {
            director,
            root,
            production: None,
            made: None,
            frames: None,
            drawing: None,
            importing: None,
        };
        let update = driver.run(calls, workspace, host);
        Ok((driver, update))
    }

    pub fn director(&self) -> &Director {
        &self.director
    }

    /// The answer to a request this driver made (`HostReport::AgentReply`).
    pub fn on_reply(
        &mut self,
        id: String,
        result: Result<String, String>,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> AppUpdate {
        if self.drawing.as_ref() == Some(&id) {
            self.drawing = None;
            return match result {
                Ok(path) => self.import(id, path, workspace, host),
                Err(error) => self.feed(
                    Input::Picture {
                        id,
                        result: Err(error),
                    },
                    workspace,
                    host,
                ),
            };
        }
        self.feed(Input::Reply { id, result }, workspace, host)
    }

    /// Bring a picture file into the project library through the control
    /// protocol; `tick` waits for the import job.
    fn import(
        &mut self,
        id: String,
        path: String,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> AppUpdate {
        let Some(context) = workspace.control_context() else {
            return AppUpdate::default();
        };
        let outcome = workspace.control(ControlRequest {
            request_id: format!("{id}-import"),
            session_id: context.session_id,
            expected_revision: Some(context.revision),
            command: ControlCommand::ImportMedia {
                paths: vec![PathBuf::from(path)],
                folder_id: None,
            },
        });
        match &outcome.response.outcome {
            ControlStatus::Ok {
                result: ControlResult::ImportStarted { job },
            } => {
                self.importing = Some((id, job.job_id));
                outcome.update
            }
            ControlStatus::Error { error } => {
                let message = error.message.clone();
                self.feed(
                    Input::Picture {
                        id,
                        result: Err(message),
                    },
                    workspace,
                    host,
                )
            }
            _ => AppUpdate::default(),
        }
    }

    fn poll_import(
        &mut self,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> Option<AppUpdate> {
        let (id, job_id) = self.importing.clone()?;
        let context = workspace.control_context()?;
        let outcome = workspace.control(ControlRequest {
            request_id: format!("{id}-job"),
            session_id: context.session_id,
            expected_revision: None,
            command: ControlCommand::GetImportJob { job_id },
        });
        let ControlStatus::Ok {
            result: ControlResult::ImportJob { job },
        } = &outcome.response.outcome
        else {
            return None;
        };
        use todo_application::control::ImportStatus;
        let result = match job.status {
            ImportStatus::Running => return None,
            _ => job
                .media_ids
                .first()
                .cloned()
                .ok_or_else(|| job.errors.join("；")),
        };
        self.importing = None;
        Some(self.feed(Input::Picture { id, result }, workspace, host))
    }

    /// Poll renders and the production; call every frame.
    pub fn tick(&mut self, workspace: &mut Workspace, host: &mut ProductionHost) -> AppUpdate {
        if let Some(update) = self.poll_import(workspace, host) {
            return update;
        }
        if let Some((id, rx)) = &self.frames {
            if let Ok(result) = rx.try_recv() {
                let id = id.clone();
                self.frames = None;
                return self.feed(Input::Frames { id, result }, workspace, host);
            }
        }
        let Some(production) = self.production.clone() else {
            return AppUpdate::default();
        };
        let Some(status) = workspace
            .production_status()
            .into_iter()
            .find(|s| s.id == production)
        else {
            return AppUpdate::default();
        };
        let result = if !status.failures.is_empty() || status.run_failures > 0 {
            let reasons: Vec<String> = status.failures.iter().map(|f| format!("{:?}", f)).collect();
            Some(Err(format!("成片有失败的步骤：{}", reasons.join("；"))))
        } else if status.total > 0 && status.done == status.total {
            Some(Ok(()))
        } else {
            None
        };
        match result {
            Some(result) => {
                self.production = None;
                self.feed(Input::Produced { result }, workspace, host)
            }
            None => AppUpdate::default(),
        }
    }

    fn feed(
        &mut self,
        input: Input,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> AppUpdate {
        let Some(project) = workspace.editor().map(|e| e.project().clone()) else {
            return AppUpdate::default();
        };
        let calls = self.director.on(input, &project);
        self.run(calls, workspace, host)
    }

    fn run(
        &mut self,
        calls: Vec<Call>,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> AppUpdate {
        let mut update = AppUpdate::default();
        for call in calls {
            let follow = match call {
                Call::Language { id, messages } => {
                    let request = TaskRequest::Language(LanguageRequest {
                        model: String::new(),
                        messages,
                        temperature: 0.2,
                        json: true,
                    });
                    host.agent_task(id.clone(), request)
                        .err()
                        .map(|error| Input::Reply {
                            id,
                            result: Err(error),
                        })
                }
                Call::Vision { id, prompt, frames } => {
                    let request = TaskRequest::Multimodal(MultimodalRequest {
                        model: String::new(),
                        prompt,
                        media: frames,
                        json: true,
                        digests: Vec::new(),
                    });
                    host.agent_task(id.clone(), request)
                        .err()
                        .map(|error| Input::Reply {
                            id,
                            result: Err(error),
                        })
                }
                Call::Render { id, times } => {
                    self.render(id, times, workspace);
                    None
                }
                Call::Produce {
                    extension,
                    params,
                    script,
                } => match self.produce(&extension, params, &script, workspace, host) {
                    Ok(more) => {
                        merge(&mut update, more);
                        None
                    }
                    Err(error) => Some(Input::Produced { result: Err(error) }),
                },
                Call::Rerun { params } => match self.rerun(params, workspace) {
                    Ok(more) => {
                        merge(&mut update, more);
                        None
                    }
                    Err(error) => Some(Input::Produced { result: Err(error) }),
                },
                Call::Apply { id, edits } => {
                    let (result, more) = apply(workspace, &id, edits);
                    merge(&mut update, more);
                    Some(Input::Applied { id, result })
                }
                Call::Picture { id, source } => match source {
                    PictureSource::File(path) => {
                        let more = self.import(id, path, workspace, host);
                        merge(&mut update, more);
                        None
                    }
                    PictureSource::Draw(prompt) => {
                        let request = TaskRequest::Image(ImageRequest {
                            model: String::new(),
                            // The director already appended the style's drawing style.
                            prompt,
                            size: "1536x1024".into(),
                            quality: Some("medium".into()),
                            transparent: true,
                            variant: 0,
                            references: Vec::new(),
                            reference_digests: Vec::new(),
                        });
                        match host.agent_task(id.clone(), request) {
                            Ok(()) => {
                                self.drawing = Some(id);
                                None
                            }
                            Err(error) => Some(Input::Picture {
                                id,
                                result: Err(error),
                            }),
                        }
                    }
                },
            };
            if let Some(input) = follow {
                let more = self.feed(input, workspace, host);
                merge(&mut update, more);
            }
        }
        update
    }

    fn render(&mut self, id: String, times: Vec<f64>, workspace: &Workspace) {
        let (tx, rx) = mpsc::channel();
        self.frames = Some((id.clone(), rx));
        let Some(project) = workspace.editor().map(|e| e.project().clone()) else {
            let _ = tx.send(Err("没有打开的工程".into()));
            return;
        };
        let root = self.root.clone();
        let dir = super::agent_bridge::frame_output_dir(&root, &id);
        std::thread::spawn(move || {
            let result = super::frame::render_frames(&root, &project, &times, &dir).map(|frames| {
                frames
                    .into_iter()
                    .map(|f| (f.time, f.png.to_string_lossy().into_owned()))
                    .collect()
            });
            let _ = tx.send(result);
        });
    }

    fn produce(
        &mut self,
        extension: &str,
        params: BTreeMap<String, serde_json::Value>,
        script: &str,
        workspace: &mut Workspace,
        host: &mut ProductionHost,
    ) -> Result<AppUpdate, String> {
        let editor = workspace.editor().ok_or("请先打开一个工程")?;
        let project = editor.project();
        let canvas = Canvas {
            width: project.width,
            height: project.height,
            fps: project.fps,
        };
        let name = editor.project_name().to_owned();
        let values: BTreeMap<String, ParamValue> = params
            .into_iter()
            .filter_map(|(k, v)| {
                let value = match v {
                    serde_json::Value::String(s) => ParamValue::Text(s),
                    serde_json::Value::Number(n) => ParamValue::Number(n.as_f64()?),
                    serde_json::Value::Bool(b) => ParamValue::Bool(b),
                    _ => return None,
                };
                Some((k, value))
            })
            .collect();
        // A parameter the model filled wrongly is dropped (the style's
        // default applies) instead of failing the whole production.
        let mut values = values;
        let (doc, setup) = loop {
            match host.create(&name, extension, &values, script, canvas.clone()) {
                Ok(created) => break created,
                Err(error) => {
                    let bad = error
                        .strip_prefix("params.")
                        .and_then(|rest| rest.split(':').next())
                        .map(str::trim)
                        .filter(|id| values.contains_key(*id))
                        .map(str::to_owned);
                    match bad {
                        Some(id) => {
                            values.remove(&id);
                        }
                        None => return Err(error),
                    }
                }
            }
        };
        self.production = Some(doc.id.clone());
        self.made = Some(doc.id.clone());
        Ok(workspace.add_production(doc, setup))
    }

    /// Change some parameters of the production; the workflow reruns (its
    /// fingerprint includes the parameters) and replaces its items in place.
    fn rerun(
        &mut self,
        params: BTreeMap<String, serde_json::Value>,
        workspace: &mut Workspace,
    ) -> Result<AppUpdate, String> {
        let id = self.made.clone().ok_or("没有可以重跑的成片")?;
        let mut merged = workspace
            .production(&id)
            .ok_or("成片已不在工程里")?
            .params
            .clone();
        merged.extend(params);
        let update = workspace.edit_production(
            &id,
            &[todo_production::production::Edit::SetParams { params: merged }],
        );
        self.production = Some(id);
        Ok(update)
    }
}

/// Dry-run, then apply on the current revision: one undo step.
fn apply(
    workspace: &mut Workspace,
    id: &str,
    edits: Vec<ControlEdit>,
) -> (Result<usize, String>, AppUpdate) {
    let request = |workspace: &Workspace, dry_run: bool| {
        let context = workspace.control_context()?;
        Some(ControlRequest {
            request_id: format!("{id}-{}", if dry_run { "dry" } else { "apply" }),
            session_id: context.session_id,
            expected_revision: Some(context.revision),
            command: ControlCommand::ApplyEdits {
                edits: edits.clone(),
                dry_run,
            },
        })
    };
    let error = |status: &ControlStatus| match status {
        ControlStatus::Error { error } => format!(
            "{}{}",
            error.message,
            error
                .step
                .map(|s| format!("（第 {s} 步）"))
                .unwrap_or_default()
        ),
        ControlStatus::Ok { .. } => String::new(),
    };
    let Some(dry) = request(workspace, true) else {
        return (Err("没有打开的工程".into()), AppUpdate::default());
    };
    let outcome = workspace.control(dry);
    if !matches!(outcome.response.outcome, ControlStatus::Ok { .. }) {
        return (Err(error(&outcome.response.outcome)), AppUpdate::default());
    }
    let Some(apply) = request(workspace, false) else {
        return (Err("没有打开的工程".into()), AppUpdate::default());
    };
    let outcome = workspace.control(apply);
    let result = match &outcome.response.outcome {
        ControlStatus::Ok {
            result: ControlResult::Edits { receipt },
        } => Ok(receipt.steps.len()),
        other => Err(error(other)),
    };
    (result, outcome.update)
}

fn merge(into: &mut AppUpdate, more: AppUpdate) {
    into.effects.extend(more.effects);
    into.new_clips.extend(more.new_clips);
}
