use egui::{RichText, Ui};
use todo_interaction::{EditorViewModel, Intent, MediaScaleMode};

use crate::theme::{button, Btn, BORDER, MUTED, PANEL, TEXT};

/// Program-canvas presets for the project settings dialog.
const OUTPUT_PRESETS: &[(&str, u32, u32)] = &[
    ("1080p", 1920, 1080),
    ("4K", 3840, 2160),
    ("720p", 1280, 720),
    ("竖屏 1080", 1080, 1920),
    ("竖屏 4K", 2160, 3840),
];

const FPS_PRESETS: &[f64] = &[23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0];

const OPEN_ID: &str = "todo_ui_project_settings_open";

pub(crate) fn set_open(ui: &Ui, open: bool) {
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), open));
}

/// True while the project-settings modal is open (native preview must hide under it).
pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new(OPEN_ID)))
        .unwrap_or(false)
}

/// Dismiss presentation state when leaving an editor or showing the close prompt.
pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), false));
}

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

/// Modal for sequence resolution, frame rate, and workspace duration.
pub(crate) fn draw(ctx: &egui::Context, vm: &EditorViewModel) -> Vec<Intent> {
    let open = ctx
        .data(|data| data.get_temp::<bool>(egui::Id::new(OPEN_ID)))
        .unwrap_or(false);
    if !open {
        return Vec::new();
    }
    let mut intents = Vec::new();
    let mut keep_open = true;
    let response = egui::Modal::new(egui::Id::new("todo_project_settings"))
        .frame(
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(egui::Stroke::new(1.0_f32, BORDER))
                .inner_margin(20)
                .corner_radius(egui::CornerRadius::ZERO),
        )
        .show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                RichText::new("工程设置")
                    .strong()
                    .size(18.0)
                    .color(TEXT),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("节目画布、帧率、落轨缩放与工作区时长")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(16.0);

            ui.label(RichText::new("分辨率").size(12.0).color(MUTED));
            ui.add_space(4.0);
            egui::ComboBox::from_id_salt("project_settings_output")
                .selected_text(output_label(vm.output_width, vm.output_height))
                .width(200.0)
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

            ui.add_space(12.0);
            ui.label(RichText::new("帧率").size(12.0).color(MUTED));
            ui.add_space(4.0);
            let mut fps = vm.timeline.fps;
            egui::ComboBox::from_id_salt("project_settings_fps")
                .selected_text(fps_label(fps))
                .width(200.0)
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

            ui.add_space(12.0);
            ui.label(RichText::new("落轨缩放").size(12.0).color(MUTED));
            ui.add_space(4.0);
            let scale_label = match vm.media_scale {
                MediaScaleMode::Pixel => "像素匹配",
                MediaScaleMode::MatchFrame => "匹配画布分辨率",
            };
            egui::ComboBox::from_id_salt("project_settings_media_scale")
                .selected_text(scale_label)
                .width(200.0)
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        (MediaScaleMode::Pixel, "像素匹配"),
                        (MediaScaleMode::MatchFrame, "匹配画布分辨率"),
                    ] {
                        let selected = vm.media_scale == mode;
                        if ui.selectable_label(selected, label).clicked() {
                            intents.push(Intent::SetMediaScaleMode { mode });
                        }
                    }
                })
                .response
                .on_hover_text(
                    "像素匹配：源像素对画布像素（1080 进 4K 不铺满）\n匹配画布分辨率：等比铺满节目画幅",
                );

            ui.add_space(12.0);
            ui.label(RichText::new("工作区时长").size(12.0).color(MUTED));
            ui.add_space(4.0);
            let mut seconds = vm.sequence_duration.unwrap_or(vm.duration).max(1.0);
            let duration_edit = ui
                .add(
                    egui::DragValue::new(&mut seconds)
                        .speed(1.0)
                        .range(1.0..=86_400.0)
                        .suffix(" 秒"),
                )
                .on_hover_text("工程工作区时长：决定时间轴上限（类似 PR / AE 合成时长）");
            let change = duration_edit
                .changed()
                .then_some(Intent::SetProjectDuration { seconds });
            super::controls::edit_response(ui, &duration_edit, change, &mut intents);

            if let Some(status) = vm.status.as_deref() {
                ui.add_space(12.0);
                ui.label(RichText::new(status).size(12.0).color(MUTED));
            }

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                if button(ui, "完成", Btn::Primary, true).clicked() {
                    keep_open = false;
                }
            });
        });
    if response.should_close() {
        keep_open = false;
    }
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), keep_open));
    intents
}

#[cfg(test)]
mod tests;
