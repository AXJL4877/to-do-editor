use super::*;
use crate::adapters::desktop::window;
use crate::adapters::production::host::HostReport;
use std::time::Duration;
use todo_application::{
    AppAction, AppEvent, AppUpdate, AssetRequest, EditorCommand, Effect, MediaOutput, Route,
    CLOCK_POLL_SECS,
};
use todo_interaction::{
    status_text, AvailableModelView, Intent, ModelCapability, PreviewPointerKind,
};
use todo_preview::PreviewNavigationKind as Navigation;
use todo_preview::{HostEvent, PreviewPointerKind as HostPointer};
use todo_ui::{
    ai_settings_open, close_project_settings, draw_ai_settings, draw_close_prompt, draw_editor,
    draw_export, draw_production_panel, draw_projects, export_open, production_panel_open,
    project_settings_open, AiSettingsAction, BG,
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
            let export_event = matches!(
                &event,
                AppEvent::Media { ticket, .. } if self.export_ticket == Some(*ticket)
            );
            if export_event {
                if let AppEvent::Media {
                    result: Err(message),
                    ..
                } = &event
                {
                    self.export.running = false;
                    self.export.report = Some(format!("导出失败：{message}"));
                    self.export_ticket = None;
                }
            }
            if let AppEvent::Media {
                result: Ok(result), ..
            } = &event
            {
                match result {
                    MediaOutput::ExportProgress { done, total } => {
                        self.export.progress = Some((*done, *total));
                        self.export.running = true;
                    }
                    MediaOutput::Exported {
                        output,
                        report,
                        frames,
                        failed_checks,
                    } => {
                        self.export.running = false;
                        self.export.progress = Some((*frames, *frames));
                        let checks = if failed_checks.is_empty() {
                            "质检通过".to_owned()
                        } else {
                            format!("质检未通过：{}", failed_checks.join("、"))
                        };
                        self.export.report = Some(format!(
                            "导出完成：{}\n报告：{}\n帧数：{}\n{}",
                            output.display(),
                            report.display(),
                            frames,
                            checks
                        ));
                        self.export_ticket = None;
                    }
                    _ => {}
                }
            }
            let update = self.workspace.on_event(event);
            self.handle(update, ctx);
        }
        let reports = self
            .production
            .as_mut()
            .map(|host| host.poll())
            .unwrap_or_default();
        for report in reports {
            match report {
                HostReport::Output(output) => {
                    let update = self.workspace.on_event(AppEvent::Production(output));
                    self.handle(update, ctx);
                }
                HostReport::Notice(message) => self.notice(message),
                HostReport::Models { provider, models } => {
                    if provider == todo_production::Provider::Huniu {
                        self.ai_settings.huniu_models = models
                            .iter()
                            .map(|model| AvailableModelView {
                                id: model.id.clone(),
                                capabilities: model
                                    .capabilities
                                    .iter()
                                    .map(|capability| match capability {
                                        todo_ai::ModelCapability::Text => ModelCapability::Text,
                                        todo_ai::ModelCapability::Image => ModelCapability::Image,
                                        todo_ai::ModelCapability::Video => ModelCapability::Video,
                                    })
                                    .collect(),
                            })
                            .collect();
                        self.ai_settings.huniu_model_status = Some(format!(
                            "已发现 {} 个可用模型",
                            self.ai_settings.huniu_models.len()
                        ));
                        if !self.ai_settings.huniu_models.iter().any(|model| {
                            model.id == self.ai_settings.huniu_chat_model
                                && model.capabilities.contains(&ModelCapability::Text)
                        }) {
                            if let Some(model) =
                                self.ai_settings.huniu_models.iter().find(|model| {
                                    model.capabilities.contains(&ModelCapability::Text)
                                })
                            {
                                self.ai_settings.huniu_chat_model = model.id.clone();
                            }
                        }
                        if !self.ai_settings.huniu_models.iter().any(|model| {
                            model.id == self.ai_settings.huniu_image_model
                                && model.capabilities.contains(&ModelCapability::Image)
                        }) {
                            if let Some(model) =
                                self.ai_settings.huniu_models.iter().find(|model| {
                                    model.capabilities.contains(&ModelCapability::Image)
                                })
                            {
                                self.ai_settings.huniu_image_model = model.id.clone();
                            }
                        }
                    }
                }
                HostReport::ModelsFailed {
                    provider,
                    class,
                    message,
                } => {
                    let provider_name = match provider {
                        todo_production::Provider::Huniu => "虎妞",
                        todo_production::Provider::Volc => "火山",
                    };
                    if provider == todo_production::Provider::Huniu {
                        self.ai_settings.huniu_model_status =
                            Some(format!("发现失败：{class}：{message}"));
                    }
                    self.notice(format!("{provider_name} 模型发现失败：{class}：{message}"));
                }
            }
        }
        self.sync_edit_proxies(ctx);
        if self.workspace.route() != Route::Editor || self.workspace.close_prompt() {
            close_project_settings(ctx);
            todo_ui::close_ai_settings(ctx);
            todo_ui::close_production_panel(ctx);
            todo_ui::set_export_open(ctx, false);
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
        let preview_input_blocked = self.workspace.close_prompt()
            || project_settings_open(ctx)
            || ai_settings_open(ctx)
            || production_panel_open(ctx)
            || export_open(ctx);
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
        match draw_ai_settings(ctx, &mut self.ai_settings) {
            AiSettingsAction::Save => {
                self.save_ai_settings();
            }
            AiSettingsAction::TestConnection => {
                if self.save_ai_settings() {
                    self.ai_settings.huniu_model_status = Some("正在读取模型目录…".into());
                    match self.production.as_mut() {
                        Some(host) => {
                            if let Err(message) = host.test_connection() {
                                self.notice(format!("连接测试：{message}"));
                            } else {
                                self.notice("正在测试虎妞对话接口…".into());
                            }
                        }
                        None => self.notice("连接测试：AI 服务没有启动".into()),
                    }
                }
            }
            AiSettingsAction::None => {}
        }
        let production_view = self.production_panel();
        match draw_production_panel(ctx, &production_view, &mut self.thumbs) {
            todo_ui::ProductionAction::OpenSettings => todo_ui::set_ai_settings_open_ctx(ctx, true),
            todo_ui::ProductionAction::Start {
                extension,
                params,
                script,
            } => self.start_production(&extension, &params, &script, ctx),
            todo_ui::ProductionAction::Retry { production, key } => {
                let update = self
                    .workspace
                    .retry_production(&production, key.map(todo_application::TaskKey));
                self.handle(update, ctx);
            }
            todo_ui::ProductionAction::Remove { production } => {
                let update = self.workspace.remove_production(&production);
                self.handle(update, ctx);
            }
            todo_ui::ProductionAction::Edit { production, edit } => {
                let update = self.workspace.edit_production(&production, &[edit]);
                self.handle(update, ctx);
            }
            todo_ui::ProductionAction::Upload {
                production,
                element,
            } => self.handle(
                AppUpdate {
                    effects: vec![Effect::ChooseProductionUpload {
                        production,
                        element,
                    }],
                    new_clips: Vec::new(),
                },
                ctx,
            ),
            todo_ui::ProductionAction::Thumbnail { path } => {
                let update = self.workspace.dispatch(AppAction::RequestAssets {
                    now: self.now(),
                    requests: vec![AssetRequest::Thumbnail { path, width: 160 }],
                });
                self.handle(update, ctx);
            }
            todo_ui::ProductionAction::None => {}
        }
        match draw_export(ctx, &mut self.export) {
            todo_ui::ExportAction::ChooseOutput => self.handle(
                AppUpdate {
                    effects: vec![Effect::ChooseExportPath],
                    new_clips: Vec::new(),
                },
                ctx,
            ),
            todo_ui::ExportAction::Start { output, preset } => {
                let preset = match preset {
                    todo_ui::ExportPresetChoice::Standard => "standard",
                    todo_ui::ExportPresetChoice::High => "high",
                    todo_ui::ExportPresetChoice::Vertical => "vertical",
                    todo_ui::ExportPresetChoice::Draft => "draft",
                };
                self.export.running = true;
                self.export.progress = None;
                self.export.report = Some("正在导出…".into());
                let update = self.workspace.dispatch(AppAction::ExportTimeline {
                    output: output.into(),
                    preset: preset.into(),
                });
                self.export_ticket = update.effects.iter().find_map(|effect| match effect {
                    Effect::Media { ticket, .. } => Some(*ticket),
                    _ => None,
                });
                self.handle(update, ctx);
            }
            todo_ui::ExportAction::Cancel => {
                self.export.report = Some("正在取消导出…".into());
                self.handle(
                    AppUpdate {
                        effects: vec![Effect::CancelExport],
                        new_clips: Vec::new(),
                    },
                    ctx,
                );
            }
            todo_ui::ExportAction::None => {}
        }
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
        // The AI production panel is a side window anchored to the right of
        // the editor. It can cover part of the monitor, but hiding the native
        // preview for the whole monitor leaves a misleading black canvas in
        // the uncovered area. Keep the preview alive underneath it; only
        // modal/settings/export surfaces that own the editor canvas should
        // suspend the native popup.
        let overlay_blocks_preview = self.workspace.close_prompt()
            || project_settings_open(ctx)
            || ai_settings_open(ctx)
            || export_open(ctx);
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
