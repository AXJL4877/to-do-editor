use super::*;
use crate::adapters::desktop::window;
use crate::adapters::production::host::HostReport;
use std::time::Duration;
use todo_application::{
    AppAction, AppEvent, AppUpdate, AssetRequest, EditorCommand, Effect, MediaOutput, Route,
    CLOCK_POLL_SECS,
};
use todo_interaction::{status_text, Intent, PreviewPointerKind};
use todo_preview::PreviewNavigationKind as Navigation;
use todo_preview::{HostEvent, PreviewPointerKind as HostPointer};
use todo_interaction::{assets_view, AssetsInput};
use todo_ui::{
    agent_open, assets_dock, close_project_settings, draw_agent, draw_ai_settings,
    draw_assets_dock, draw_close_prompt, draw_editor, draw_export, draw_production_dock,
    draw_projects, draw_workflow_overlay, export_open, open_workflow, production_panel_open,
    project_settings_open, workflow_open_id, AssetsAction, LeftTab, BG,
};

/// Any surface that can occupy the editor canvas must suspend the independent
/// native preview window. The preview is a separate HWND, so egui's painter
/// order cannot cover it reliably by itself.
/// The AI production dock is part of the layout and never overlaps it; menus
/// and tooltips over the preview are cut out of it instead (`overlays_over`).
fn preview_overlay_open(ctx: &egui::Context, close_prompt: bool) -> bool {
    close_prompt
        || project_settings_open(ctx)
        || todo_ui::ai_settings_open(ctx)
        || agent_open(ctx)
        || export_open(ctx)
        || todo_ui::workflow_fullscreen_open(ctx)
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_agent_bridge(ctx);
        self.poll_agent_render(ctx);
        self.poll_director(ctx);
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
            match self.ai_report(report) {
                Some(HostReport::Output(output)) => {
                    let update = self.workspace.on_event(AppEvent::Production(output));
                    self.handle(update, ctx);
                }
                Some(HostReport::Notice(message)) => self.notice(message),
                Some(report @ HostReport::AgentReply { .. }) => self.agent_report(report, ctx),
                _ => {}
            }
        }
        self.sync_edit_proxies(ctx);
        if self.workspace.route() != Route::Editor || self.workspace.close_prompt() {
            close_project_settings(ctx);
            todo_ui::close_ai_settings(ctx);
            todo_ui::close_agent(ctx);
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
        let preview_input_blocked = preview_overlay_open(ctx, self.workspace.close_prompt());
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
        // Dock list + `~` fullscreen both need the workflow catalogue.
        let production_view =
            (self.workspace.route() == Route::Editor).then(|| self.production_panel());
        let mut production_action = todo_ui::ProductionAction::None;
        let mut assets_action = AssetsAction::None;
        let mut assets_tab = LeftTab::Media;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ctx, |ui| {
                if let Some(editor) = self.workspace.editor() {
                    let mut view = self
                        .controller
                        .view(editor, self.workspace.motion_library());
                    if view.status.is_none() {
                        view.status = self.proxies.summary();
                    }
                    active = view.monitor_active;
                    requests = self.controller.asset_requests(&view);
                    let (mut output, slot) =
                        draw_editor(ui, &view, self.workspace.peaks(), &mut self.thumbs);
                    if let (Some(rect), Some(panel)) =
                        (todo_ui::production_dock(ui.ctx()), production_view.as_ref())
                    {
                        if production_panel_open(ui.ctx()) {
                            let mut dock = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt("production_dock")
                                    .max_rect(rect),
                            );
                            dock.set_clip_rect(rect);
                            production_action =
                                draw_production_dock(&mut dock, panel, &mut self.thumbs);
                        }
                    }
                    if let Some((rect, tab)) = assets_dock(ui.ctx()) {
                        assets_tab = tab;
                        if tab == LeftTab::Fonts {
                            let missing = self.fonts.iter().any(|font| {
                                !self.thumbs.contains(&todo_interaction::font_preview_key(
                                    &font.file,
                                ))
                            });
                            if missing {
                                super::assets::fill_font_previews(
                                    &crate::adapters::paths::repo_root(),
                                    &self.fonts,
                                    Some(editor.project()),
                                    &mut self.thumbs,
                                );
                            }
                        }
                        let assets = assets_view(AssetsInput {
                            motions: self.motion_library.entries(),
                            fonts: self.fonts.clone(),
                            project: Some(editor.project()),
                            selected: view.properties.as_ref(),
                            warnings: self.asset_warnings.clone(),
                            motions_dir: self.motions_dir.clone(),
                            fonts_dir: self.fonts_dir.clone(),
                        });
                        let mut dock = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt("assets_dock")
                                .max_rect(rect),
                        );
                        dock.set_clip_rect(rect);
                        let (asset_intents, action) =
                            draw_assets_dock(&mut dock, &assets, tab, &mut self.thumbs);
                        assets_action = action;
                        output.extend(asset_intents);
                    }
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
        if let Some(panel) = production_view.as_ref() {
            let open_id = workflow_open_id(ctx);
            let graph = open_id.as_ref().and_then(|id| {
                self.production
                    .as_ref()
                    .and_then(|host| host.workflow_panel(id))
            });
            let title = open_id
                .as_ref()
                .and_then(|id| {
                    panel
                        .workflows
                        .iter()
                        .find(|e| e.extension_id == *id)
                        .map(|e| e.name.as_str())
                })
                .unwrap_or("工作流");
            let wf = draw_workflow_overlay(ctx, panel, graph.as_ref(), title);
            if wf != todo_ui::ProductionAction::None {
                production_action = wf;
            }
        }
        let settings = draw_ai_settings(ctx, &mut self.ai_settings);
        self.ai_settings_action(settings);
        let agent_action = draw_agent(ctx, &mut self.agent_view);
        self.handle_agent_action(agent_action, ctx);
        match production_action {
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
            todo_ui::ProductionAction::Restore { production } => {
                let update = self.workspace.restore_production(&production);
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
            todo_ui::ProductionAction::WorkflowCreate => {
                if let Some(host) = &mut self.production {
                    match host.create_workflow("未命名工作流") {
                        Ok(id) => {
                            open_workflow(ctx, id);
                            self.notice("已新建工作流".into());
                        }
                        Err(e) => self.notice(format!("新建失败：{e}")),
                    }
                }
            }
            todo_ui::ProductionAction::WorkflowSave {
                extension,
                positions,
                edits,
            } => {
                if let Some(host) = &mut self.production {
                    match host.save_workflow_graph(&extension, &positions, &edits) {
                        Ok(()) => self.notice("工作流已保存".into()),
                        Err(e) => self.notice(format!("保存失败：{e}")),
                    }
                }
            }
            todo_ui::ProductionAction::WorkflowSaveParams {
                extension,
                node_id,
                params,
            } => {
                if let Some(host) = &mut self.production {
                    match host.save_workflow_node_params(&extension, &node_id, &params) {
                        Ok(()) => self.notice("工作流参数已保存".into()),
                        Err(e) => self.notice(format!("保存失败：{e}")),
                    }
                }
            }
            todo_ui::ProductionAction::WorkflowPickMedia { extension, node_id } => {
                if let Some(paths) = crate::adapters::desktop::workflow_bin_files() {
                    if let Some(host) = &mut self.production {
                        match host.add_workflow_media_files(&extension, &node_id, &paths) {
                            Ok(n) => self.notice(format!("已添加 {n} 个文件到素材容器")),
                            Err(e) => self.notice(format!("添加失败：{e}")),
                        }
                    }
                }
            }
            todo_ui::ProductionAction::WorkflowRemoveMedia {
                extension,
                node_id,
                path,
            } => {
                if let Some(host) = &mut self.production {
                    match host.remove_workflow_media_file(&extension, &node_id, &path) {
                        Ok(()) => self.notice("已从容器移除该文件".into()),
                        Err(e) => self.notice(format!("移除失败：{e}")),
                    }
                }
            }
            todo_ui::ProductionAction::None => {}
        }
        self.handle_assets_action(assets_action, assets_tab);
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
        // Re-read after UI so a surface opened this frame hides the native
        // popup before the next OS composition pass. The preview is a separate
        // HWND, so every editor overlay must participate in this gate.
        let overlay_blocks_preview = preview_overlay_open(ctx, self.workspace.close_prompt());
        let hole = hole.filter(|_| self.workspace.route() == Route::Editor);
        let overlays = hole
            .as_ref()
            .map(|hole| todo_ui::overlays_over(ctx, hole))
            .unwrap_or_default();
        self.preview.sync_popup(
            ctx,
            frame,
            hole,
            &overlays,
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
