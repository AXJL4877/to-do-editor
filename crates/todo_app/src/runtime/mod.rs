//! Desktop composition root: render, collect input, and execute application effects.
mod agent;
mod ai_settings;
mod assets;
mod effects;
mod frame;
mod productions;
mod proxies;
mod window_state;
use crate::adapters::agent_bridge::{AgentBridge, AgentRequest};
use crate::adapters::production::host::ProductionHost;
use crate::adapters::{
    editing_proxies::EditingProxies, paths, preview::PreviewBridge, workers::Workers,
};
use eframe::egui;
use std::sync::mpsc;
use std::time::Instant;
use todo_ai::{AiConfig, Secret, Secrets};
use todo_application::{AppAction, Ticket, Workspace};
use todo_interaction::{AiSettingsView, EditorController, FontInput};
use todo_motion::MotionLibrary;
use todo_ui::{install_theme, AgentView, ExportView, ThumbCache};

pub struct App {
    workspace: Workspace,
    controller: EditorController,
    workers: Workers,
    preview: PreviewBridge,
    thumbs: ThumbCache,
    project_draft: String,
    started: Instant,
    last_clock_poll: f64,
    last_preview_attempt: f64,
    close_approved: bool,
    window_state: window_state::WindowState,
    proxies: EditingProxies,
    proxy_project: Option<(u64, u64)>,
    ai_settings: AiSettingsView,
    production: Option<ProductionHost>,
    /// Why the production host did not start (shown in the AI dock).
    production_error: Option<String>,
    export: ExportView,
    export_ticket: Option<Ticket>,
    agent_bridge: Option<AgentBridge>,
    agent_view: AgentView,
    agent_render_rx: Option<mpsc::Receiver<serde_json::Value>>,
    /// The built-in Agent's running (or approved) director.
    director: Option<crate::adapters::director::DirectorDriver>,
    /// Director log entries already shown in the panel.
    director_logged: usize,
    /// Full motion library (built-in / user / extension) for the assets dock.
    motion_library: MotionLibrary,
    fonts: Vec<FontInput>,
    asset_warnings: Vec<String>,
    motions_dir: String,
    fonts_dir: String,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_theme(&cc.egui_ctx);
        let repo = paths::repo_root();
        let mut ai_settings = AiSettingsView::default();
        if let Ok(config) = AiConfig::load(&repo) {
            ai_settings.huniu_base_url = config.huniu.base_url;
            ai_settings.huniu_strategy = config
                .huniu
                .strategy
                .unwrap_or_else(|| "XUE".into());
            ai_settings.huniu_language_model = config.huniu.language_model;
            ai_settings.huniu_image_model = config.huniu.image_model;
            ai_settings.huniu_video_model = config.huniu.video_model;
            ai_settings.huniu_multimodal_model = config.huniu.multimodal_model;
            ai_settings.volc_base_url = config.volc.base_url;
            ai_settings.volc_audio_model = config.volc.audio_model;
            ai_settings.volc_speaker = config.volc.speaker;
        }
        {
            let store = todo_ai::secrets::default_store();
            let secrets = Secrets {
                store: store.as_ref(),
                environment: &todo_ai::secrets::process_environment,
            };
            if let Ok(Some((key, source))) = secrets.resolve(todo_production::Provider::Huniu) {
                ai_settings.huniu_key = Some(key.masked());
                ai_settings.huniu_key_source = match source {
                    todo_ai::SecretSource::Environment => todo_interaction::KeySource::Environment,
                    todo_ai::SecretSource::Keychain => todo_interaction::KeySource::Keychain,
                };
            }
            if let Ok(Some((key, source))) = secrets.resolve(todo_production::Provider::Volc) {
                ai_settings.volc_key = Some(key.masked());
                ai_settings.volc_key_source = match source {
                    todo_ai::SecretSource::Environment => todo_interaction::KeySource::Environment,
                    todo_ai::SecretSource::Keychain => todo_interaction::KeySource::Keychain,
                };
            }
        }
        let (production, production_error) = match ProductionHost::start(repo.clone()) {
            Ok(host) => (Some(host), None),
            Err(error) => (None, Some(error)),
        };
        let (agent_bridge, agent_bridge_error) = match AgentBridge::start(&repo) {
            Ok(bridge) => (Some(bridge), None),
            Err(error) => (None, Some(error)),
        };
        let loaded = Self::load_asset_libraries(&repo);
        let workspace = Workspace::default();
        let mut app = Self {
            workspace,
            controller: EditorController::new(),
            workers: Workers::start(repo.clone()),
            preview: PreviewBridge::new(&repo),
            thumbs: ThumbCache::default(),
            project_draft: "未命名".into(),
            started: Instant::now(),
            last_clock_poll: f64::NEG_INFINITY,
            last_preview_attempt: f64::NEG_INFINITY,
            close_approved: false,
            window_state: window_state::WindowState::load(cc.storage),
            proxies: EditingProxies::start(repo.clone()),
            proxy_project: None,
            ai_settings,
            production,
            production_error,
            export: ExportView::default(),
            export_ticket: None,
            agent_bridge,
            agent_view: AgentView::default(),
            agent_render_rx: None,
            director: None,
            director_logged: 0,
            motion_library: MotionLibrary::builtin(),
            fonts: Vec::new(),
            asset_warnings: Vec::new(),
            motions_dir: String::new(),
            fonts_dir: String::new(),
        };
        for warning in &loaded.warnings {
            app.notice(format!("资源库：{warning}"));
        }
        app.apply_asset_libraries(loaded);
        let update = app.workspace.dispatch(AppAction::RefreshProjects);
        app.handle(update, &cc.egui_ctx);
        if let Some(error) = agent_bridge_error {
            app.notice(format!("Agent Bridge：{error}"));
        }
        app
    }
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
    fn dispatch(&mut self, action: AppAction, ctx: &egui::Context) {
        let update = self.workspace.dispatch(action);
        self.handle(update, ctx);
    }
    fn notice(&mut self, message: String) {
        let _ = self.workspace.dispatch(AppAction::Notice(message));
    }

    fn poll_agent_bridge(&mut self, ctx: &egui::Context) {
        let mut requests = Vec::new();
        if let Some(bridge) = &self.agent_bridge {
            for _ in 0..8 {
                let Some(request) = bridge.try_recv() else {
                    break;
                };
                requests.push(request);
            }
        }
        for request in requests {
            match request {
                AgentRequest::Control { request, reply } => {
                    let outcome = self.workspace.control(request);
                    let response =
                        serde_json::to_value(&outcome.response).unwrap_or_else(|error| {
                            serde_json::json!({
                                "apiVersion": 1,
                                "status": "error",
                                "error": {
                                    "code": "serialization",
                                    "message": error.to_string()
                                }
                            })
                        });
                    self.handle(outcome.update, ctx);
                    let _ = reply.send(response);
                }
                AgentRequest::RenderPreview {
                    request_id,
                    session_id,
                    times,
                    reply,
                } => self.start_agent_render(request_id, session_id, times, reply),
            }
        }
    }

    fn start_agent_render(
        &mut self,
        request_id: String,
        session_id: u64,
        times: Vec<f64>,
        reply: std::sync::mpsc::Sender<serde_json::Value>,
    ) {
        let Some(context) = self.workspace.control_context() else {
            let _ = reply.send(crate::adapters::agent_bridge::error_response(
                &request_id,
                session_id,
                None,
                "no_project",
                "请先打开一个工程",
            ));
            return;
        };
        if context.session_id != session_id {
            let _ = reply.send(crate::adapters::agent_bridge::error_response(
                &request_id,
                session_id,
                Some(context.revision),
                "session_conflict",
                "工程会话已变化，请重新读取上下文",
            ));
            return;
        }
        let times = if times.is_empty() {
            vec![context.playhead]
        } else {
            times
        };
        if times.len() > 16 || times.iter().any(|time| !time.is_finite() || *time < 0.0) {
            let _ = reply.send(crate::adapters::agent_bridge::error_response(
                &request_id,
                session_id,
                Some(context.revision),
                "invalid_request",
                "times 须为 0..16 个非负有限秒数",
            ));
            return;
        }
        let root = crate::adapters::paths::repo_root();
        let output_dir = crate::adapters::agent_bridge::frame_output_dir(&root, &request_id);
        let revision = context.revision;
        let project = context.project;
        std::thread::spawn(move || {
            let response =
                match crate::adapters::frame::render_frames(&root, &project, &times, &output_dir) {
                    Ok(frames) => crate::adapters::agent_bridge::ok_response(
                        &request_id,
                        session_id,
                        revision,
                        serde_json::json!({
                            "type": "preview_frames",
                            "frames": frames.iter().map(|frame| serde_json::json!({
                                "time": frame.time,
                                "png": frame.png,
                                "width": frame.width,
                                "height": frame.height
                            })).collect::<Vec<_>>()
                        }),
                    ),
                    Err(error) => crate::adapters::agent_bridge::error_response(
                        &request_id,
                        session_id,
                        None,
                        "render_failed",
                        &error,
                    ),
                };
            let _ = reply.send(response);
        });
    }
}
