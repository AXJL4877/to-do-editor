//! Independent properties pane. Its footprint is owned by workspace layout.
mod clip;
mod text;
mod transform;
use crate::theme::{BORDER, BRAND, MUTED, PANEL, TEXT};
use egui::{RichText, Ui};
use todo_interaction::{ClipProperties, Intent};

pub(super) fn draw(ui: &mut Ui, properties: Option<&ClipProperties>) -> Vec<Intent> {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, 0.0, PANEL);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new("属性").strong().size(11.0).color(TEXT));
    });
    let y = rect.top() + 29.0;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() + 12.0, y),
            egui::pos2(rect.left() + 44.0, y),
        ],
        egui::Stroke::new(2.0_f32, BRAND),
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), y + 2.0),
            egui::pos2(rect.right(), y + 2.0),
        ],
        egui::Stroke::new(1.0_f32, BORDER),
    );
    ui.add_space(14.0);
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
                            ui.label(RichText::new("选择时间线中的片段").color(MUTED));
                            ui.label(RichText::new("在此查看和调整片段属性").small().color(MUTED));
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
