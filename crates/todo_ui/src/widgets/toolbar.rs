use egui::{Align, Layout, RichText, Ui};
use todo_interaction::{EditorViewModel, Intent};

use super::{agent, ai_settings, chrome, export, production, project_settings};
use crate::theme::{button, Btn, BORDER, BRAND, COLORS, MUTED, TEXT};

pub fn draw(ui: &mut Ui, vm: &EditorViewModel) -> Vec<Intent> {
    let mut intents = Vec::new();
    let bar = ui.max_rect();
    ui.painter().rect_filled(bar, 0.0, COLORS.panel);
    ui.painter().line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        egui::Stroke::new(1.0_f32, BORDER),
    );
    chrome::wire_toolbar(ui, bar);
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new("TO DO").strong().size(14.0).color(BRAND));
        ui.separator();
        if button(ui, "工程", Btn::Ghost, true).clicked() {
            intents.push(Intent::Back);
        }
        if button(
            ui,
            if vm.dirty { "保存 *" } else { "保存" },
            Btn::Normal,
            true,
        )
        .clicked()
        {
            intents.push(Intent::Save);
        }
        if button(ui, "撤销", Btn::Normal, vm.can_undo).clicked() {
            intents.push(Intent::Undo);
        }
        if button(ui, "重做", Btn::Normal, vm.can_redo).clicked() {
            intents.push(Intent::Redo);
        }
        ui.separator();
        if button(ui, "文字", Btn::Normal, true).clicked() {
            intents.push(Intent::AddText);
        }
        if button(ui, "工程设置", Btn::Normal, true).clicked() {
            project_settings::set_open(ui, true);
        }
        if button(ui, "AI 设置", Btn::Normal, true).clicked() {
            ai_settings::set_open(ui, true);
        }
        let agent_kind = if agent::is_open(ui.ctx()) {
            Btn::Tool { active: true }
        } else {
            Btn::Normal
        };
        if button(ui, "Agent", agent_kind, true).clicked() {
            agent::toggle(ui.ctx());
        }
        let dock = production::is_open(ui.ctx());
        let kind = if dock {
            Btn::Tool { active: true }
        } else {
            Btn::Normal
        };
        if button(ui, "工作流", kind, true).clicked() {
            production::toggle(ui.ctx());
        }
        if button(ui, "导出", Btn::Normal, true).clicked() {
            export::set_open(ui.ctx(), true);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            chrome::window_buttons(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new(if vm.dirty { "未保存" } else { "已保存" })
                    .size(12.0)
                    .color(if vm.dirty { BRAND } else { MUTED }),
            );
            ui.add_space(10.0);
            let status = vm.status.as_deref().or(vm.clip_name.as_deref()).unwrap_or("");
            if !status.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(status).size(12.0).color(MUTED)).truncate(),
                );
                ui.add_space(8.0);
            }
            ui.label(
                RichText::new(format!("{} / {}", vm.time_label, vm.duration_label))
                    .monospace()
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(&vm.project_name).strong().size(13.0).color(TEXT));
        });
    });
    intents
}
