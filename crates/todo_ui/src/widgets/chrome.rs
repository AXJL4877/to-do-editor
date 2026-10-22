//! Frameless window chrome: drag strip and min/max/close.

use egui::{Id, PointerButton, RichText, Sense, Ui, UiBuilder, ViewportCommand};

use crate::theme::{button, Btn, BORDER, MUTED, PANEL};

const CHROME_H: f32 = 36.0;

/// Top drag strip with window controls for screens without a toolbar.
pub fn draw_strip(ui: &mut Ui) {
    let full = ui.max_rect();
    let rect = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), CHROME_H));
    ui.painter().rect_filled(rect, 0.0, PANEL);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0_f32, BORDER),
    );

    wire_drag(ui, rect, Id::new("todo_chrome_drag"));

    let mut bar = ui.new_child(UiBuilder::new().id_salt("todo_chrome_bar").max_rect(rect));
    bar.set_clip_rect(rect.intersect(ui.clip_rect()));
    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_space(6.0);
        window_buttons(ui);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("TO DO").strong().size(13.0).color(MUTED));
        });
    });
    ui.advance_cursor_after_rect(rect);
}

/// Wire drag onto an existing toolbar rect (draw window buttons after this so they win hits).
pub fn wire_toolbar(ui: &mut Ui, toolbar_rect: egui::Rect) {
    // Salt with the current Ui id so Background toolbar and Foreground
    // overlays (workflow fullscreen) never share a widget id in one frame.
    wire_drag(ui, toolbar_rect, ui.id().with("todo_toolbar_drag"));
}

pub fn window_buttons(ui: &mut Ui) {
    if button(ui, "×", Btn::Ghost, true)
        .on_hover_text("关闭")
        .clicked()
    {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
    let max_label = if maximized { "❐" } else { "□" };
    if button(ui, max_label, Btn::Ghost, true)
        .on_hover_text(if maximized { "还原" } else { "最大化" })
        .clicked()
    {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if button(ui, "—", Btn::Ghost, true)
        .on_hover_text("最小化")
        .clicked()
    {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn wire_drag(ui: &mut Ui, rect: egui::Rect, id: Id) {
    let drag = ui.interact(rect, id, Sense::click_and_drag());
    if drag.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}
