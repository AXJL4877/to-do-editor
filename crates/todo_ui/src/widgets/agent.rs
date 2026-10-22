//! Built-in Agent panel: a thin UI over the director. The runtime owns the
//! director, its requests, rendering and the approval decision.

use crate::theme::{
    button, heading, modal_frame, with_input_radius, Btn, BORDER, BRAND, DIM, FIELD, MUTED,
    PANEL_2, RADIUS, TEXT,
};
use egui::{Context, Id, RichText, ScrollArea, TextEdit};

const OPEN_ID: &str = "todo_ui_builtin_agent_open";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    User,
    Agent,
    System,
    /// What the reviewer saw in the frames.
    Reviewer,
    /// Edits the director applied.
    Edit,
}

/// The cut waiting for the user's decision.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentApproval {
    pub summary: String,
    pub remaining: Vec<String>,
    pub frames: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessage {
    pub role: AgentRole,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentView {
    pub messages: Vec<AgentMessage>,
    pub input: String,
    pub status: Option<String>,
    pub frames: Vec<String>,
    pub busy: bool,
    pub approval: Option<AgentApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAction {
    None,
    Send(String),
    ReadContext,
    RenderCurrent,
    /// Accept the cut: go on to export.
    Approve,
    /// Stop the running director.
    Stop,
}

pub fn set_open(ctx: &Context, open: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), open));
}

pub fn toggle(ctx: &Context) {
    set_open(ctx, !is_open(ctx));
}

pub fn is_open(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(Id::new(OPEN_ID)))
        .unwrap_or(false)
}

pub fn close(ctx: &Context) {
    set_open(ctx, false);
}

pub fn draw_modal(ctx: &Context, view: &mut AgentView) -> AgentAction {
    if !is_open(ctx) {
        return AgentAction::None;
    }
    let mut action = AgentAction::None;
    let mut keep_open = true;
    let response = egui::Modal::new(Id::new("todo_builtin_agent_modal"))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(520.0);
            egui::Frame::NONE
                .fill(PANEL_2)
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(heading("编辑 Agent"));
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
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .id_salt("builtin_agent_messages")
                        .max_height(360.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            if view.messages.is_empty() {
                                ui.label(RichText::new("还没有对话").size(12.0).color(DIM));
                            }
                            for message in &view.messages {
                                let (label, color) = match message.role {
                                    AgentRole::User => ("你", BRAND),
                                    AgentRole::Agent => ("导演", TEXT),
                                    AgentRole::System => ("系统", MUTED),
                                    AgentRole::Reviewer => {
                                        ("审片", egui::Color32::from_rgb(0xff, 0xc8, 0x57))
                                    }
                                    AgentRole::Edit => {
                                        ("修改", egui::Color32::from_rgb(0x7f, 0xd4, 0x9b))
                                    }
                                };
                                egui::Frame::NONE
                                    .fill(PANEL_2)
                                    .inner_margin(8.0)
                                    .corner_radius(egui::CornerRadius::ZERO)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new(label).strong().size(11.0).color(color),
                                        );
                                        ui.label(
                                            RichText::new(&message.text).size(12.0).color(TEXT),
                                        );
                                    });
                                ui.add_space(5.0);
                            }
                        });

                    if let Some(status) = &view.status {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if view.busy {
                                ui.spinner();
                            }
                            ui.label(RichText::new(status).size(12.0).color(BRAND));
                            if view.busy {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if button(ui, "停止", Btn::Ghost, true).clicked() {
                                            action = AgentAction::Stop;
                                        }
                                    },
                                );
                            }
                        });
                    }

                    if let Some(approval) = &view.approval {
                        ui.add_space(8.0);
                        egui::Frame::NONE
                            .fill(PANEL_2)
                            .stroke(egui::Stroke::new(1.0_f32, BRAND))
                            .inner_margin(10.0)
                            .corner_radius(egui::CornerRadius::ZERO)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(
                                    RichText::new("等你审批").strong().size(13.0).color(BRAND),
                                );
                                ui.label(
                                    RichText::new(&approval.summary).size(12.0).color(TEXT),
                                );
                                for item in &approval.remaining {
                                    ui.label(
                                        RichText::new(format!("· {item}")).size(12.0).color(MUTED),
                                    );
                                }
                                ui.add_space(6.0);
                                if button(ui, "通过并导出", Btn::Primary, true).clicked() {
                                    action = AgentAction::Approve;
                                }
                            });
                    }

                    ui.add_space(10.0);
                    with_input_radius(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut view.input)
                                .desired_rows(3)
                                .hint_text("描述你想怎么改…")
                                .desired_width(f32::INFINITY),
                        );
                    });
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
                    ui.horizontal(|ui| {
                        if button(ui, "读取工程", Btn::Ghost, !view.busy).clicked() {
                            action = AgentAction::ReadContext;
                        }
                        if button(ui, "渲染当前帧", Btn::Ghost, !view.busy).clicked() {
                            action = AgentAction::RenderCurrent;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if button(
                                ui,
                                "发送",
                                Btn::Primary,
                                !view.busy && !view.input.trim().is_empty(),
                            )
                            .clicked()
                            {
                                let prompt = std::mem::take(&mut view.input);
                                view.messages.push(AgentMessage {
                                    role: AgentRole::User,
                                    text: prompt.clone(),
                                });
                                action = AgentAction::Send(prompt);
                            }
                        });
                    });
                });
        });
    if !keep_open || response.should_close() {
        close(ctx);
    }
    action
}
