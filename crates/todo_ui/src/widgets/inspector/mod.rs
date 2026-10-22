//! Independent properties pane. Its footprint is owned by workspace layout.
mod clip;
mod motion;
mod text;
mod transform;
use crate::theme::MUTED;
use egui::{RichText, Ui};
use todo_interaction::{ClipProperties, Intent};

pub(super) fn draw(ui: &mut Ui, properties: Option<&ClipProperties>) -> Vec<Intent> {
    // The pane header (tabs) is drawn by the workspace layout.
    let rect = ui.available_rect_before_wrap();
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt("inspector_content")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.set_width((rect.width() - 28.0).max(0.0));
                        if let Some(properties) = properties {
                            clip::draw(ui, properties)
                        } else {
                            ui.add_space(20.0);
                            ui.label(RichText::new("选择时间线中的片段").size(13.0).color(MUTED));
                            Vec::new()
                        }
                    })
                    .inner
                })
                .inner
            })
            .inner
        })
        .inner
}
