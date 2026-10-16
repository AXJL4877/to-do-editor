//! Desktop composition root: render, collect input, and execute application effects.
mod effects;
mod frame;
mod proxies;
mod window_state;
use crate::adapters::{
    editing_proxies::EditingProxies, paths, preview::PreviewBridge, workers::Workers,
};
use eframe::egui;
use std::time::Instant;
use todo_application::{AppAction, Workspace};
use todo_interaction::EditorController;
use todo_ui::{install_theme, ThumbCache};

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
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_theme(&cc.egui_ctx);
        let repo = paths::repo_root();
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
            proxies: EditingProxies::start(repo),
            proxy_project: None,
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
}
