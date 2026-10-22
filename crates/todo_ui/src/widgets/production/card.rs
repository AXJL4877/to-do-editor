//! "任务": one card per production — what it is, what is happening now, the
//! pipeline strip, problems with their fix, and the generated content.
use super::{content, flow, status_color, ProductionAction};
use crate::theme::{button, Btn, BORDER, DANGER, DIM, MUTED, PANEL_2, TEXT, WARN};
use crate::ThumbCache;
use egui::{Id, RichText, Ui};
use todo_interaction::{
    ProblemAction, ProblemView, ProductionNodeStatus, ProductionPanelView, ProductionView,
    StageView,
};

/// Key of the problem whose "may be charged" retry waits for confirmation.
const CONFIRM_ID: &str = "todo_ui_production_confirm";

pub(super) fn draw_all(
    ui: &mut Ui,
    view: &ProductionPanelView,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    if view.productions.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("还没有成片任务").size(13.0).color(MUTED));
        });
        return action;
    }
    for production in &view.productions {
        let next = draw_card(ui, production, thumbs);
        if next != ProductionAction::None {
            action = next;
        }
        ui.add_space(10.0);
    }
    action
}

/// One pipeline step, painted as a fixed-size chip so wrapping moves whole
/// chips to the next line instead of squeezing their text.
fn stage_chip(ui: &mut Ui, stage: &StageView) {
    let color = status_color(stage.status);
    let text = match &stage.count {
        Some(count) => format!("{} {count}", stage.label),
        None => stage.label.to_owned(),
    };
    let font = egui::FontId::proportional(11.5);
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let size = egui::vec2(galley.size().x + 22.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let active = matches!(
        stage.status,
        ProductionNodeStatus::Running | ProductionNodeStatus::Waiting
    );
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 0.0, color.gamma_multiply(0.16));
    }
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    // State dot: filled when done, ring while pending, bar when failed.
    let dot = egui::pos2(rect.left() + 9.0, rect.center().y);
    match stage.status {
        ProductionNodeStatus::Complete => {
            painter.circle_filled(dot, 3.0, color);
        }
        ProductionNodeStatus::Failed => {
            painter.rect_filled(
                egui::Rect::from_center_size(dot, egui::vec2(3.0, 9.0)),
                0.0,
                color,
            );
        }
        _ => {
            painter.circle_stroke(dot, 3.0, egui::Stroke::new(1.2_f32, color));
        }
    }
    painter.galley(
        egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response.on_hover_text(match stage.status {
        ProductionNodeStatus::Complete => "已完成",
        ProductionNodeStatus::Running => "进行中",
        ProductionNodeStatus::Waiting => "等待网络",
        ProductionNodeStatus::Attention => "需要处理",
        ProductionNodeStatus::Failed => "有失败项",
        ProductionNodeStatus::Queued => "排队中",
    });
}

fn problem_row(
    ui: &mut Ui,
    production: &ProductionView,
    problem: &ProblemView,
    action: &mut ProductionAction,
) {
    let confirm_key = format!("{}:{:?}", production.id, problem.key);
    let confirming = ui.ctx().data(|d| {
        d.get_temp::<String>(Id::new(CONFIRM_ID))
            .is_some_and(|k| k == confirm_key)
    });
    egui::Frame::NONE
        .fill(DANGER.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0_f32, DANGER.gamma_multiply(0.45)))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("{}没有完成", problem.title))
                    .size(12.5)
                    .color(TEXT),
            );
            ui.label(RichText::new(&problem.reason).size(12.0).color(MUTED));
            if confirming {
                ui.label(RichText::new("确定重试？").size(12.0).color(WARN));
                ui.horizontal(|ui| {
                    if button(ui, "确定重试", Btn::Danger, true).clicked() {
                        *action = ProductionAction::Retry {
                            production: production.id.clone(),
                            key: problem.key.clone(),
                        };
                        ui.ctx()
                            .data_mut(|d| d.remove::<String>(Id::new(CONFIRM_ID)));
                    }
                    if button(ui, "取消", Btn::Ghost, true).clicked() {
                        ui.ctx()
                            .data_mut(|d| d.remove::<String>(Id::new(CONFIRM_ID)));
                    }
                });
                return;
            }
            ui.horizontal(|ui| match problem.action {
                ProblemAction::Retry => {
                    if button(ui, "重试", Btn::Normal, true).clicked() {
                        *action = ProductionAction::Retry {
                            production: production.id.clone(),
                            key: problem.key.clone(),
                        };
                    }
                }
                ProblemAction::ConfirmRetry => {
                    if button(ui, "重试…", Btn::Normal, true).clicked() {
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(Id::new(CONFIRM_ID), confirm_key.clone()));
                    }
                }
                ProblemAction::OpenSettings => {
                    if button(ui, "打开 AI 设置", Btn::Normal, true).clicked() {
                        *action = ProductionAction::OpenSettings;
                    }
                    if button(ui, "改好后重试", Btn::Ghost, true).clicked() {
                        *action = ProductionAction::Retry {
                            production: production.id.clone(),
                            key: problem.key.clone(),
                        };
                    }
                }
            });
        });
}

fn draw_card(
    ui: &mut Ui,
    production: &ProductionView,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    egui::Frame::NONE
        .fill(PANEL_2)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&production.title)
                        .strong()
                        .size(14.0)
                        .color(TEXT),
                );
                ui.label(RichText::new(&production.style).size(11.5).color(DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new("···").color(MUTED), |ui| {
                        if production.deleted > 0
                            && ui
                                .button(format!("放回删掉的 {} 个片段", production.deleted))
                                .clicked()
                        {
                            action = ProductionAction::Restore {
                                production: production.id.clone(),
                            };
                            ui.close_menu();
                        }
                        let finished = production.status == ProductionNodeStatus::Complete;
                        let remove = if finished {
                            "从列表移除"
                        } else {
                            "停止生成并移除"
                        };
                        if ui
                            .button(RichText::new(remove).color(DANGER))
                            .clicked()
                        {
                            action = ProductionAction::Remove {
                                production: production.id.clone(),
                            };
                            ui.close_menu();
                        }
                    });
                    if ui
                        .add(
                            egui::Button::new(RichText::new("流程").size(12.0).color(MUTED))
                                .frame(false),
                        )
                        .clicked()
                    {
                        flow::open(ui.ctx(), production.id.clone());
                    }
                });
            });
            ui.add_space(2.0);
            let color = status_color(production.status);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, color);
                ui.label(RichText::new(&production.headline).size(12.5).color(TEXT));
            });
            ui.add(
                egui::ProgressBar::new(production.progress)
                    .desired_height(4.0)
                    .fill(color),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for stage in &production.stages {
                    stage_chip(ui, stage);
                }
            });
            for problem in &production.problems {
                ui.add_space(6.0);
                problem_row(ui, production, problem, &mut action);
            }
            for note in &production.notes {
                ui.add_space(4.0);
                ui.label(RichText::new(note).size(11.5).color(WARN));
            }
            if production.deleted > 0 && production.status == ProductionNodeStatus::Complete {
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("放回删掉的 {} 个片段", production.deleted))
                                .size(11.5)
                                .color(MUTED),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    action = ProductionAction::Restore {
                        production: production.id.clone(),
                    };
                }
            }
            if !production.segments.is_empty() {
                ui.add_space(6.0);
                let next = content::draw(ui, production, thumbs);
                if next != ProductionAction::None {
                    action = next;
                }
            }
        });
    action
}
