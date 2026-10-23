//! Built-in Agent panel: chat UI over a tool-calling runtime.
//! Drawn in the right dock (replaces the old workflow list).

use crate::theme::{
    button, heading, with_input_radius, Btn, BORDER, BRAND, DIM, FIELD, MUTED, PANEL_2, TEXT,
};
use egui::{Context, Id, RichText, ScrollArea, TextEdit, Ui};

const OPEN_ID: &str = "todo_ui_builtin_agent_open";
const COPY_TOKEN_ID: &str = "todo_ui_copy_agent_token";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    User,
    Agent,
    System,
    /// Tool call the agent decided to make.
    Tool,
    /// What the reviewer saw in the frames (legacy director).
    Reviewer,
    /// Edits the director applied (legacy director).
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
    /// Stop the running agent.
    Stop,
}

pub fn set_open(ctx: &Context, open: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), open));
}

#[allow(dead_code)] // Modal path retired; dock uses production::toggle.
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

/// Toolbar asks the host to copy the live Agent Bridge token.
pub fn request_copy_token(ctx: &Context) {
    ctx.data_mut(|data| data.insert_temp(Id::new(COPY_TOKEN_ID), true));
}

pub fn take_copy_token(ctx: &Context) -> bool {
    ctx.data_mut(|data| data.remove_temp::<bool>(Id::new(COPY_TOKEN_ID)))
        .unwrap_or(false)
}

/// Side-dock chat (primary Agent surface).
pub fn draw_dock(ui: &mut Ui, view: &mut AgentView) -> AgentAction {
    let mut action = AgentAction::None;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new("Agent").strong().size(13.0).color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            if view.busy {
                ui.spinner();
            }
        });
    });
    ui.add_space(4.0);
    let rect = ui.max_rect();
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        egui::Stroke::new(1.0_f32, BORDER),
    );

    let footer_h = 118.0_f32;
    let body_h = (ui.available_height() - footer_h).max(120.0);

    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: 8,
            bottom: 4,
        })
        .show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("agent_dock_messages")
                .max_height(body_h)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if view.messages.is_empty() {
                        ui.label(
                            RichText::new(
                                "用自然语言指挥成片。Agent 会精确调用工具：读工程、改时间轴、截帧、入轨状态、扩展包…",
                            )
                            .size(12.0)
                            .color(DIM),
                        );
                    }
                    for message in &view.messages {
                        paint_message(ui, message);
                        ui.add_space(6.0);
                    }
                    if let Some(status) = &view.status {
                        ui.horizontal(|ui| {
                            if view.busy {
                                ui.spinner();
                            }
                            ui.label(RichText::new(status).size(12.0).color(BRAND));
                        });
                    }
                    if let Some(approval) = &view.approval {
                        ui.add_space(6.0);
                        egui::Frame::NONE
                            .fill(PANEL_2)
                            .stroke(egui::Stroke::new(1.0_f32, BRAND))
                            .inner_margin(10.0)
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
                });
        });

    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        egui::Stroke::new(1.0_f32, BORDER),
    );
    egui::Frame::NONE
        .fill(FIELD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            with_input_radius(ui, |ui| {
                ui.add(
                    TextEdit::multiline(&mut view.input)
                        .desired_rows(3)
                        .hint_text("例如：读一下工程，再在 0/1/2 秒截帧给我看…")
                        .desired_width(f32::INFINITY),
                );
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if button(ui, "读工程", Btn::Ghost, !view.busy).clicked() {
                    action = AgentAction::ReadContext;
                }
                if button(ui, "截当前帧", Btn::Ghost, !view.busy).clicked() {
                    action = AgentAction::RenderCurrent;
                }
                if view.busy && button(ui, "停止", Btn::Ghost, true).clicked() {
                    action = AgentAction::Stop;
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
    let _ = rect;
    let _ = heading;
    action
}

fn paint_message(ui: &mut Ui, message: &AgentMessage) {
    let (label, color) = match message.role {
        AgentRole::User => ("你", BRAND),
        AgentRole::Agent => ("Agent", TEXT),
        AgentRole::System => ("系统", MUTED),
        AgentRole::Tool => ("工具", egui::Color32::from_rgb(0x6e, 0xc8, 0xff)),
        AgentRole::Reviewer => ("审片", egui::Color32::from_rgb(0xff, 0xc8, 0x57)),
        AgentRole::Edit => ("修改", egui::Color32::from_rgb(0x7f, 0xd4, 0x9b)),
    };
    egui::Frame::NONE
        .fill(PANEL_2)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(label).strong().size(11.0).color(color));
            ui.label(RichText::new(&message.text).size(12.0).color(TEXT));
        });
}

/// Legacy modal entry (kept for API compat; prefer [`draw_dock`]).
pub fn draw_modal(ctx: &Context, view: &mut AgentView) -> AgentAction {
    if !is_open(ctx) {
        return AgentAction::None;
    }
    // Modal path retired: close and open the dock instead.
    close(ctx);
    let _ = view;
    AgentAction::None
}
