use egui::{Align, Layout, RichText, Ui};
use todo_interaction::{EditorViewModel, Intent};

use super::chrome;
use crate::theme::{button, Btn, BORDER, BRAND, COLORS, MUTED, TEXT};

/// Program-canvas presets shown in the toolbar.
const OUTPUT_PRESETS: &[(&str, u32, u32)] = &[
    ("1080p", 1920, 1080),
    ("4K", 3840, 2160),
    ("720p", 1280, 720),
    ("竖屏 1080", 1080, 1920),
    ("竖屏 4K", 2160, 3840),
];

const FPS_PRESETS: &[f64] = &[23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0];

fn output_label(width: u32, height: u32) -> String {
    OUTPUT_PRESETS
        .iter()
        .find(|(_, w, h)| *w == width && *h == height)
        .map(|(name, _, _)| (*name).to_string())
        .unwrap_or_else(|| format!("{width}×{height}"))
}

fn fps_label(fps: f64) -> String {
    if (fps - fps.round()).abs() < 1e-6 {
        format!("{} fps", fps.round() as i64)
    } else {
        format!("{fps:.3} fps")
    }
}

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
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            chrome::window_buttons(ui);
            ui.add_space(8.0);
            ui.label(RichText::new(&vm.project_name).strong().color(TEXT));
            ui.label(
                RichText::new(format!("{} / {}", vm.time_label, vm.duration_label))
                    .monospace()
                    .color(MUTED),
            );
            ui.add_space(6.0);
            let mut seconds = vm.sequence_duration.unwrap_or(vm.duration).max(1.0);
            let duration_edit = ui
                .add(
                    egui::DragValue::new(&mut seconds)
                        .speed(1.0)
                        .range(1.0..=86_400.0)
                        .suffix(" 秒")
                        .prefix("工作区 "),
                )
                .on_hover_text("工程工作区时长：决定时间轴上限（类似 PR / AE 合成时长）");
            let change = duration_edit
                .changed()
                .then_some(Intent::SetProjectDuration { seconds });
            super::controls::edit_response(ui, &duration_edit, change, &mut intents);

            ui.add_space(4.0);
            let mut fps = vm.timeline.fps;
            egui::ComboBox::from_id_salt("project_fps")
                .selected_text(fps_label(fps))
                .width(88.0)
                .show_ui(ui, |ui| {
                    for &preset in FPS_PRESETS {
                        let selected = (fps - preset).abs() < 1e-6;
                        if ui.selectable_label(selected, fps_label(preset)).clicked() {
                            fps = preset;
                            intents.push(Intent::SetProjectFps { fps: preset });
                        }
                    }
                })
                .response
                .on_hover_text("工程帧率：时间轴刻度与剪辑对齐基准");

            ui.add_space(4.0);
            egui::ComboBox::from_id_salt("project_output")
                .selected_text(output_label(vm.output_width, vm.output_height))
                .width(104.0)
                .show_ui(ui, |ui| {
                    for &(name, width, height) in OUTPUT_PRESETS {
                        let selected = vm.output_width == width && vm.output_height == height;
                        if ui.selectable_label(selected, name).clicked() {
                            intents.push(Intent::SetProjectOutput { width, height });
                        }
                    }
                })
                .response
                .on_hover_text("节目画布分辨率：预览输出框与导出画幅");
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
