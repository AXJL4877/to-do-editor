use egui::{RichText, Ui};
use todo_interaction::{EditorViewModel, Intent, MediaScaleMode};

use crate::theme::{
    button, heading, label_muted, modal_frame, with_input_radius, Btn, BORDER, FIELD, MUTED,
    PANEL_2, RADIUS,
};

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
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(420.0);
            egui::Frame::NONE
                .fill(PANEL_2)
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(heading("工程设置"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("×").size(16.0).color(MUTED))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::NONE)
                                        .corner_radius(egui::CornerRadius::same(RADIUS)),
                                )
                                .clicked()
                            {
                                keep_open = false;
                            }
                        });
                    });
                });
            ui.painter().hline(
                ui.max_rect().x_range(),
                ui.cursor().top(),
                egui::Stroke::new(1.0_f32, BORDER),
            );

            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    with_input_radius(ui, |ui| {
                        ui.label(label_muted("分辨率"));
                        ui.add_space(4.0);
                        egui::ComboBox::from_id_salt("project_settings_output")
                            .selected_text(output_label(vm.output_width, vm.output_height))
                            .width(280.0)
                            .show_ui(ui, |ui| {
                                for &(name, width, height) in OUTPUT_PRESETS {
                                    let selected =
                                        vm.output_width == width && vm.output_height == height;
                                    if ui.selectable_label(selected, name).clicked() {
                                        intents.push(Intent::SetProjectOutput { width, height });
                                    }
                                }
                            });

                        ui.add_space(14.0);
                        ui.label(label_muted("帧率"));
                        ui.add_space(4.0);
                        let mut fps = vm.timeline.fps;
                        egui::ComboBox::from_id_salt("project_settings_fps")
                            .selected_text(fps_label(fps))
                            .width(280.0)
                            .show_ui(ui, |ui| {
                                for &preset in FPS_PRESETS {
                                    let selected = (fps - preset).abs() < 1e-6;
                                    if ui.selectable_label(selected, fps_label(preset)).clicked() {
                                        fps = preset;
                                        intents.push(Intent::SetProjectFps { fps: preset });
                                    }
                                }
                            });

                        ui.add_space(14.0);
                        ui.label(label_muted("落轨缩放"));
                        ui.add_space(4.0);
                        let scale_label = match vm.media_scale {
                            MediaScaleMode::Pixel => "像素匹配",
                            MediaScaleMode::MatchFrame => "匹配画布",
                        };
                        egui::ComboBox::from_id_salt("project_settings_media_scale")
                            .selected_text(scale_label)
                            .width(280.0)
                            .show_ui(ui, |ui| {
                                for (mode, label) in [
                                    (MediaScaleMode::Pixel, "像素匹配"),
                                    (MediaScaleMode::MatchFrame, "匹配画布"),
                                ] {
                                    let selected = vm.media_scale == mode;
                                    if ui.selectable_label(selected, label).clicked() {
                                        intents.push(Intent::SetMediaScaleMode { mode });
                                    }
                                }
                            });

                        ui.add_space(14.0);
                        ui.label(label_muted("工作区时长"));
                        ui.add_space(4.0);
                        let mut seconds = vm.sequence_duration.unwrap_or(vm.duration).max(1.0);
                        let duration_edit = ui.add(
                            egui::DragValue::new(&mut seconds)
                                .speed(1.0)
                                .range(1.0..=86_400.0)
                                .suffix(" 秒"),
                        );
                        let change = duration_edit
                            .changed()
                            .then_some(Intent::SetProjectDuration { seconds });
                        super::controls::edit_response(ui, &duration_edit, change, &mut intents);
                    });

                    if let Some(status) = vm.status.as_deref() {
                        ui.add_space(12.0);
                        ui.label(RichText::new(status).size(12.0).color(MUTED));
                    }
                });

            ui.painter().hline(
                ui.max_rect().x_range(),
                ui.cursor().top(),
                egui::Stroke::new(1.0_f32, BORDER),
            );
            egui::Frame::NONE
                .fill(FIELD)
                .inner_margin(egui::Margin::symmetric(20, 12))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button(ui, "完成", Btn::Primary, true).clicked() {
                            // Commit any in-progress DragValue edit, then persist.
                            intents.push(Intent::CommitEdit);
                            intents.push(Intent::Save);
                            keep_open = false;
                        }
                    });
                });
        });
    if !keep_open || response.should_close() {
        if response.should_close() {
            intents.push(Intent::CommitEdit);
            intents.push(Intent::Save);
        }
        set_open_ctx(ctx, false);
    }
    intents
}

fn set_open_ctx(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), open));
}

#[cfg(test)]
#[path = "project_settings/tests.rs"]
mod tests;
