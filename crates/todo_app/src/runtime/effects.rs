use super::*;
use crate::adapters::desktop;
use std::collections::VecDeque;
use todo_application::{AppUpdate, Effect};

impl App {
    pub(super) fn handle(&mut self, update: AppUpdate, ctx: &egui::Context) {
        self.controller.reconcile(&self.workspace);
        self.controller
            .select_inserted(&self.workspace, &update.new_clips);
        let changed = !update.effects.is_empty() || !update.new_clips.is_empty();
        let mut effects = VecDeque::from(update.effects);
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::Project { ticket, task } => {
                    if let Err(event) = self.workers.project(ticket, task) {
                        effects.extend(self.workspace.on_event(*event).effects);
                    }
                }
                Effect::Media { ticket, task } => {
                    if let Err(event) = self.workers.media(ticket, task) {
                        effects.extend(self.workspace.on_event(*event).effects);
                    }
                }
                Effect::Production(task) => match &mut self.production {
                    Some(host) => {
                        if let Some(problem) = host.execute(task) {
                            self.notice(problem);
                        }
                    }
                    None => self.notice("AI 成片服务没有启动".into()),
                },
                Effect::CancelExport => self.workers.cancel_export(),
                Effect::Preview(commands) => {
                    if let Err(message) = self.preview.apply(commands) {
                        self.notice(format!("预览：{message}"));
                    }
                }
                Effect::ChooseMediaFiles => {
                    if let Some(paths) = desktop::media_files() {
                        effects.extend(
                            self.workspace
                                .dispatch(AppAction::ImportFiles(paths))
                                .effects,
                        );
                    }
                }
                Effect::ChooseProjectFile => {
                    if let Some(path) = desktop::project_file() {
                        effects.extend(
                            self.workspace
                                .dispatch(AppAction::ImportProjectFile(path))
                                .effects,
                        );
                    }
                }
                Effect::ChooseExportPath => {
                    if let Some(path) = desktop::export_file() {
                        self.export.output = path.to_string_lossy().into_owned();
                    }
                }
                Effect::ChooseProductionUpload {
                    production,
                    element,
                } => {
                    let Some(source) = desktop::image_file() else {
                        continue;
                    };
                    let Some(project) = self
                        .workspace
                        .editor()
                        .map(|editor| editor.project_name().to_owned())
                    else {
                        continue;
                    };
                    let Some(host) = self.production.as_ref() else {
                        self.notice("AI 成片服务没有启动".into());
                        continue;
                    };
                    match host.copy_variant(&project, &production, &source) {
                        Ok(file) => {
                            let update = self.workspace.edit_production(
                                &production,
                                &[todo_application::ProductionEdit::UploadVariant {
                                    element,
                                    file,
                                }],
                            );
                            effects.extend(update.effects);
                        }
                        Err(message) => self.notice(format!("上传图片失败：{message}")),
                    }
                }
                Effect::UploadThumbnail { path, bytes } => {
                    if !self.thumbs.insert_bytes(path, &bytes) {
                        self.notice("缩略图解码失败".into());
                    }
                }
                Effect::ResetPresentation => {
                    self.proxies
                        .set_sources(self.workspace.generation(), Vec::new());
                    self.preview.clear_edit_sources();
                    self.proxy_project = None;
                    todo_ui::close_project_settings(ctx);
                    self.controller.reset();
                    self.thumbs = ThumbCache::default();
                    // A project just opened: pick up its productions.
                    effects.extend(self.resume_productions());
                }
                Effect::CloseWindow => {
                    self.close_approved = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if changed {
            ctx.request_repaint();
        }
    }
}
