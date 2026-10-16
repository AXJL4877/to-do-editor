use super::*;
use crate::adapters::desktop::window;
use std::time::Duration;
use todo_application::{EditorCommand, Route, CLOCK_POLL_SECS};
use todo_interaction::{status_text, Intent, PreviewPointerKind};
use todo_preview::PreviewNavigationKind as Navigation;
use todo_preview::{HostEvent, PreviewPointerKind as HostPointer};
use todo_ui::{
    close_project_settings, draw_close_prompt, draw_editor, draw_projects, project_settings_open,
    BG,
};

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let pixels_per_point = ctx.pixels_per_point();
        ctx.input(|input| {
            self.window_state
                .observe(input.viewport(), pixels_per_point)
        });
        if let Some(hwnd) = window::viewport_hwnd(frame) {
            window::apply_dark_frame(hwnd);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_approved {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let update = self
                .controller
                .apply(&mut self.workspace, Intent::RequestClose);
            self.handle(update, ctx);
        }
        while let Some(event) = self.workers.poll() {
            let update = self.workspace.on_event(event);
            self.handle(update, ctx);
        }
        self.sync_edit_proxies(ctx);
        if self.workspace.route() != Route::Editor || self.workspace.close_prompt() {
            close_project_settings(ctx);
        }
        let now = self.now();
        if self.workspace.route() == Route::Editor && now - self.last_preview_attempt >= 2.0 {
            self.last_preview_attempt = now;
            match self.preview.ensure() {
                Ok(true) => {
                    let update = self.workspace.resync_preview();
                    self.handle(update, ctx);
                }
                Ok(false) => {}
                Err(e) => self.notice(format!("预览：{e}")),
            }
        }
        let preview_input_blocked = self.workspace.close_prompt() || project_settings_open(ctx);
        while let Some(event) = self.preview.poll() {
            if let HostEvent::PreviewNavigation(input) = event {
                if preview_input_blocked {
                    continue;
                }
                let intent = match input.kind {
                    Navigation::Wheel => Intent::PreviewWheel {
                        delta: input.delta,
                        x: input.x,
                        y: input.y,
                    },
                    kind => Intent::PreviewPan {
                        kind: match kind {
                            Navigation::PanDown => PreviewPointerKind::Down,
                            Navigation::PanMove => PreviewPointerKind::Move,
                            _ => PreviewPointerKind::Up,
                        },
                        x: input.x,
                        y: input.y,
                    },
                };
                let update = self.controller.apply(&mut self.workspace, intent);
                self.handle(update, ctx);
                ctx.request_repaint();
            }
            if let HostEvent::SourceAspect { generation, aspect } = event {
                if let Some(path) = self.preview.source_path(generation).map(str::to_owned) {
                    let update = self.controller.apply(
                        &mut self.workspace,
                        Intent::PreviewSourceAspect { path, aspect },
                    );
                    self.handle(update, ctx);
                }
            }
            if let HostEvent::PreviewPointer { kind, x, y } = event {
                if preview_input_blocked {
                    continue;
                }
                if kind == HostPointer::Down {
                    // The native popup deliberately does not activate itself. Release
                    // egui text focus so keyboard shortcuts target the captured gesture.
                    ctx.memory_mut(|memory| {
                        if let Some(id) = memory.focused() {
                            memory.surrender_focus(id);
                        }
                    });
                }
                let kind = match kind {
                    HostPointer::Down => PreviewPointerKind::Down,
                    HostPointer::Move => PreviewPointerKind::Move,
                    HostPointer::Up => PreviewPointerKind::Up,
                };
                let update = self
                    .controller
                    .apply(&mut self.workspace, Intent::PreviewPointer { kind, x, y });
                self.handle(update, ctx);
            }
        }
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>()
        });
        if !preview_input_blocked && !dropped.is_empty() && self.workspace.route() == Route::Editor
        {
            self.dispatch(AppAction::ImportFiles(dropped), ctx);
        }
        let mut hole = None;
        let mut active = false;
        let mut intents = Vec::new();
        let mut requests = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ctx, |ui| {
                if let Some(editor) = self.workspace.editor() {
                    let mut view = self.controller.view(editor);
                    if view.status.is_none() {
                        view.status = self.proxies.summary();
                    }
                    active = view.monitor_active;
                    requests = self.controller.asset_requests(&view);
                    let (output, slot) =
                        draw_editor(ui, &view, self.workspace.peaks(), &mut self.thumbs);
                    intents = if self.workspace.close_prompt() {
                        Vec::new()
                    } else {
                        output
                    };
                    hole = slot;
                } else {
                    let error = self.workspace.error().map(status_text);
                    intents = draw_projects(
                        ui,
                        self.workspace.projects(),
                        &mut self.project_draft,
                        error.as_deref(),
                    );
                }
            });
        intents.extend(draw_close_prompt(
            ctx,
            self.workspace.close_prompt(),
            self.workspace.saving(),
        ));
        for intent in intents {
            let released_scrub = matches!(intent, Intent::PointerUp);
            let update = self.controller.apply(&mut self.workspace, intent);
            self.handle(update, ctx);
            if released_scrub {
                if let Err(error) = self.preview.finish_scrub() {
                    self.notice(format!("预览：{error}"));
                }
            }
        }
        if self.workspace.route() == Route::Editor {
            self.dispatch(AppAction::RequestAssets { now, requests }, ctx);
        }
        self.sync_edit_proxies(ctx);
        if let Some(editor) = self.workspace.editor() {
            if editor.playing() {
                if now - self.last_clock_poll >= CLOCK_POLL_SECS {
                    self.last_clock_poll = now;
                    let epoch = editor.clock_epoch();
                    let sample = self.preview.clock();
                    self.dispatch(
                        AppAction::Editor(EditorCommand::Clock { epoch, now, sample }),
                        ctx,
                    );
                }
                self.dispatch(AppAction::Editor(EditorCommand::Frame { now }), ctx);
            }
        }
        // Re-read after UI so a settings modal opened this frame hides the native popup.
        let overlay_blocks_preview = self.workspace.close_prompt() || project_settings_open(ctx);
        self.preview.sync_popup(
            ctx,
            frame,
            hole.filter(|_| self.workspace.route() == Route::Editor),
            active && !overlay_blocks_preview,
        );
        let preview_view = if overlay_blocks_preview {
            None
        } else {
            self.workspace
                .editor()
                .map(|editor| self.controller.preview_view(editor))
        };
        if let Err(error) = self.preview.sync_view(preview_view) {
            self.notice(format!("预览控制框：{error}"));
        }
        if self.preview.syncing_restore()
            || self.workspace.editor().is_some_and(|e| e.playing())
            || self.workspace.busy()
            || self.controller.preview_gesture_active()
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.proxies.shutdown();
        self.preview.shutdown();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.window_state.save(storage);
    }
}
