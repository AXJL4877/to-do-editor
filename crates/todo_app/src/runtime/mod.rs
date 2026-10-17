//! Desktop composition root: render, collect input, and execute application effects.
mod effects;
mod frame;
mod productions;
mod proxies;
mod window_state;
use crate::adapters::production::host::ProductionHost;
use crate::adapters::{
    editing_proxies::EditingProxies, paths, preview::PreviewBridge, workers::Workers,
};
use eframe::egui;
use std::time::Instant;
use todo_ai::{AiConfig, Secret, Secrets};
use todo_application::{AppAction, Ticket, Workspace};
use todo_interaction::{AiSettingsView, EditorController};
use todo_ui::{install_theme, ExportView, ThumbCache};

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
    export: ExportView,
    export_ticket: Option<Ticket>,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_theme(&cc.egui_ctx);
        let repo = paths::repo_root();
        let mut ai_settings = AiSettingsView::default();
        if let Ok(config) = AiConfig::load(&repo) {
            ai_settings.huniu_base_url = config.huniu.base_url;
            ai_settings.huniu_chat_model = config.huniu.chat_model;
            ai_settings.huniu_image_model = config.huniu.image_model;
            ai_settings.volc_base_url = config.volc.base_url;
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
        let mut app = Self {
            workspace: Workspace::default(),
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
            production: ProductionHost::start(repo.clone()).ok(),
            export: ExportView::default(),
            export_ticket: None,
        };
        let update = app.workspace.dispatch(AppAction::RefreshProjects);
        app.handle(update, &cc.egui_ctx);
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

    fn save_ai_settings(&mut self) -> bool {
        let root = paths::repo_root();
        let mut file = todo_ai::config::AiConfig::load_file(&root).unwrap_or_default();
        let before = AiConfig::load(&root).ok();
        file.huniu.base_url = Some(self.ai_settings.huniu_base_url.clone());
        file.huniu.chat_model = Some(self.ai_settings.huniu_chat_model.clone());
        file.huniu.image_model = Some(self.ai_settings.huniu_image_model.clone());
        file.volc.base_url = Some(self.ai_settings.volc_base_url.clone());
        file.volc.speaker = Some(self.ai_settings.volc_speaker.clone());
        file.proxy = Some(match self.ai_settings.proxy {
            todo_interaction::ProxyChoice::Auto => todo_ai::ProxyMode::Auto,
            todo_interaction::ProxyChoice::System => todo_ai::ProxyMode::System,
            todo_interaction::ProxyChoice::Direct => todo_ai::ProxyMode::Direct,
        });
        let result = AiConfig::save(&root, &file)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                let store = todo_ai::secrets::default_store();
                let secrets = Secrets {
                    store: store.as_ref(),
                    environment: &todo_ai::secrets::process_environment,
                };
                if self.ai_settings.huniu_key_source != todo_interaction::KeySource::Environment {
                    if self.ai_settings.huniu_clear_key {
                        secrets
                            .save(todo_production::Provider::Huniu, None)
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.huniu_key = None;
                        self.ai_settings.huniu_key_source = todo_interaction::KeySource::Missing;
                        self.ai_settings.huniu_clear_key = false;
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Huniu, None);
                        }
                    }
                    if let Some(value) = Secret::new(self.ai_settings.huniu_key_entry.clone()) {
                        secrets
                            .save(todo_production::Provider::Huniu, Some(&value))
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.huniu_key = Some(value.masked());
                        self.ai_settings.huniu_key_source = todo_interaction::KeySource::Keychain;
                        self.ai_settings.huniu_key_entry.clear();
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Huniu, Some(value));
                        }
                    }
                }
                if self.ai_settings.volc_key_source != todo_interaction::KeySource::Environment {
                    if self.ai_settings.volc_clear_key {
                        secrets
                            .save(todo_production::Provider::Volc, None)
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.volc_key = None;
                        self.ai_settings.volc_key_source = todo_interaction::KeySource::Missing;
                        self.ai_settings.volc_clear_key = false;
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Volc, None);
                        }
                    }
                    if let Some(value) = Secret::new(self.ai_settings.volc_key_entry.clone()) {
                        secrets
                            .save(todo_production::Provider::Volc, Some(&value))
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.volc_key = Some(value.masked());
                        self.ai_settings.volc_key_source = todo_interaction::KeySource::Keychain;
                        self.ai_settings.volc_key_entry.clear();
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Volc, Some(value));
                        }
                    }
                }
                Ok(())
            });
        // Models are read per request; addresses and the proxy need a new
        // client. Restarting interrupts requests in flight, so only then.
        let after = AiConfig::load(&root).ok();
        let endpoint = |c: &Option<AiConfig>| {
            c.as_ref().map(|c| {
                (
                    c.huniu.base_url.clone(),
                    c.volc.base_url.clone(),
                    format!("{:?}", c.proxy),
                    c.huniu.chat_model.clone(),
                    c.huniu.image_model.clone(),
                )
            })
        };
        let result = result.and_then(|()| {
            if endpoint(&before) != endpoint(&after) {
                if let Some(host) = self.production.as_mut() {
                    host.reconfigure()?;
                }
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                self.notice("AI 设置已保存".into());
                true
            }
            Err(error) => {
                self.notice(format!("AI 设置保存失败：{error}"));
                false
            }
        }
    }
}
