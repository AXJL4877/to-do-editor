use egui::{Context, RichText};

use crate::theme::{
    button, heading, label_muted, modal_frame, with_input_radius, Btn, BORDER, FIELD, MUTED,
    PANEL_2, RADIUS, SUCCESS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPresetChoice {
    Standard,
    High,
    Vertical,
    Draft,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportView {
    pub output: String,
    pub preset: ExportPresetChoice,
    pub report: Option<String>,
    pub progress: Option<(usize, usize)>,
    pub running: bool,
}

impl Default for ExportView {
    fn default() -> Self {
        Self {
            output: String::new(),
            preset: ExportPresetChoice::Standard,
            report: None,
            progress: None,
            running: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportAction {
    None,
    ChooseOutput,
    Start {
        output: String,
        preset: ExportPresetChoice,
    },
    Cancel,
}

const OPEN_ID: &str = "todo_ui_export_open";

pub fn set_open(ctx: &Context, open: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), open));
}

pub fn is_open(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new(OPEN_ID)))
        .unwrap_or(false)
}

pub fn draw_modal(ctx: &Context, view: &mut ExportView) -> ExportAction {
    if !is_open(ctx) {
        return ExportAction::None;
    }
    let mut action = ExportAction::None;
    let mut keep_open = true;
    let response = egui::Modal::new(egui::Id::new("todo_export_modal"))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(480.0);
            egui::Frame::NONE
                .fill(PANEL_2)
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(heading("导出"));
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
                    ui.label(label_muted("输出路径"));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        with_input_radius(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut view.output)
                                    .desired_width(320.0)
                                    .hint_text("选择保存位置")
                                    .margin(egui::Margin::symmetric(10, 6)),
                            );
                        });
                        if button(ui, "浏览…", Btn::Normal, !view.running).clicked() {
                            action = ExportAction::ChooseOutput;
                        }
                    });

                    ui.add_space(14.0);
                    ui.label(label_muted("预设"));
                    ui.add_space(4.0);
                    with_input_radius(ui, |ui| {
                        egui::ComboBox::from_id_salt("export_preset")
                            .selected_text(match view.preset {
                                ExportPresetChoice::Standard => "标准 1080p",
                                ExportPresetChoice::High => "高质量",
                                ExportPresetChoice::Vertical => "竖屏",
                                ExportPresetChoice::Draft => "草稿",
                            })
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut view.preset,
                                    ExportPresetChoice::Standard,
                                    "标准 1080p",
                                );
                                ui.selectable_value(
                                    &mut view.preset,
                                    ExportPresetChoice::High,
                                    "高质量",
                                );
                                ui.selectable_value(
                                    &mut view.preset,
                                    ExportPresetChoice::Vertical,
                                    "竖屏",
                                );
                                ui.selectable_value(
                                    &mut view.preset,
                                    ExportPresetChoice::Draft,
                                    "草稿",
                                );
                            });
                    });

                    if let Some((done, total)) = view.progress {
                        ui.add_space(12.0);
                        ui.add(
                            egui::ProgressBar::new(if total == 0 {
                                0.0
                            } else {
                                done as f32 / total as f32
                            })
                            .text(format!("{done} / {total}")),
                        );
                    }
                    if let Some(report) = &view.report {
                        ui.add_space(8.0);
                        ui.label(RichText::new(report).size(12.0).color(SUCCESS));
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
                        if view.running {
                            if button(ui, "取消", Btn::Danger, true).clicked() {
                                action = ExportAction::Cancel;
                            }
                        } else if button(
                            ui,
                            "开始导出",
                            Btn::Primary,
                            !view.output.trim().is_empty(),
                        )
                        .clicked()
                        {
                            action = ExportAction::Start {
                                output: view.output.clone(),
                                preset: view.preset,
                            };
                        }
                        ui.add_space(6.0);
                        if button(ui, "关闭", Btn::Ghost, true).clicked() {
                            keep_open = false;
                        }
                    });
                });
        });
    if !keep_open || response.should_close() {
        set_open(ctx, false);
    }
    action
}
