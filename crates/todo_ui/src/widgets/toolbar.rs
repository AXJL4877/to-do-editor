use egui::{Align, Layout, RichText, Ui};
use todo_interaction::{EditorViewModel, Intent};

use super::{chrome, project_settings};
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
    ui.horizontal(|ui| {
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
        .on_hover_text("保存工程 (Ctrl+S)")
        .clicked()
        {
            intents.push(Intent::Save);
        }
        if button(ui, "撤销", Btn::Normal, vm.can_undo)
            .on_hover_text("撤销 (Ctrl+Z)")
            .clicked()
        {
            intents.push(Intent::Undo);
        }
        if button(ui, "重做", Btn::Normal, vm.can_redo)
            .on_hover_text("重做 (Ctrl+Shift+Z / Ctrl+Y)")
            .clicked()
        {
            intents.push(Intent::Redo);
        }
        ui.separator();
        if button(ui, "文字", Btn::Normal, true)
            .on_hover_text("在播放头处添加花字，在右侧属性栏编辑")
            .clicked()
        {
            intents.push(Intent::AddText);
        }
        if button(ui, "工程设置", Btn::Normal, true)
            .on_hover_text("分辨率、帧率与工作区时长")
            .clicked()
        {
            project_settings::set_open(ui, true);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            chrome::window_buttons(ui);
            ui.add_space(8.0);
            ui.label(RichText::new(&vm.project_name).strong().color(TEXT));
            ui.label(
                RichText::new(format!("{} / {}", vm.time_label, vm.duration_label))
                    .monospace()
                    .color(MUTED),
            );
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(112.0);
        let status = vm.status.as_deref().unwrap_or_else(|| {
            vm.clip_name
                .as_deref()
                .unwrap_or("导入素材会自动落轨；也可追加或拖入时间线")
        });
        ui.add(egui::Label::new(egui::RichText::new(status).small().color(MUTED)).truncate())
            .on_hover_text(status);
        ui.add_space(6.0);
        ui.label(
            RichText::new(if vm.dirty {
                "● 未保存"
            } else {
                "已保存"
            })
            .small()
            .color(if vm.dirty { BRAND } else { MUTED }),
        );
    });
    intents
}
