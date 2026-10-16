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
                Effect::UploadThumbnail { path, bytes } => {
                    if !self.thumbs.insert_bytes(path, &bytes) {
                        self.notice("缩略图解码失败".into());
                    }
                }
                Effect::ResetPresentation => {
                    self.controller.reset();
                    self.thumbs = ThumbCache::default();
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
