use egui::Context;

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
    let mut open = true;
    egui::Window::new("导出")
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("输出 MP4 路径");
                ui.text_edit_singleline(&mut view.output);
                if ui.button("选择…").clicked() {
                    action = ExportAction::ChooseOutput;
                }
            });
            egui::ComboBox::from_label("预设")
                .selected_text(match view.preset {
                    ExportPresetChoice::Standard => "标准 1080p30",
                    ExportPresetChoice::High => "高质量",
                    ExportPresetChoice::Vertical => "竖屏 1080×1920",
                    ExportPresetChoice::Draft => "快速草稿",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut view.preset,
                        ExportPresetChoice::Standard,
                        "标准 1080p30",
                    );
                    ui.selectable_value(&mut view.preset, ExportPresetChoice::High, "高质量");
                    ui.selectable_value(
                        &mut view.preset,
                        ExportPresetChoice::Vertical,
                        "竖屏 1080×1920",
                    );
                    ui.selectable_value(&mut view.preset, ExportPresetChoice::Draft, "快速草稿");
                });
            if let Some(report) = &view.report {
                ui.separator();
                ui.label(report);
            }
            if let Some((done, total)) = view.progress {
                ui.add(
                    egui::ProgressBar::new(if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    })
                    .text(format!("{done} / {total} 帧")),
                );
            }
            ui.horizontal(|ui| {
                if view.running {
                    if ui.button("取消导出").clicked() {
                        action = ExportAction::Cancel;
                    }
                } else if ui.button("开始导出").clicked() && !view.output.trim().is_empty() {
                    action = ExportAction::Start {
                        output: view.output.clone(),
                        preset: view.preset,
                    };
                }
                if ui.button("关闭").clicked() {
                    set_open(ctx, false);
                }
            });
            if view.running {
                ui.label("导出进行中，时间轴仍可查看。");
            }
        });
    if !open {
        set_open(ctx, false);
    }
    action
}
