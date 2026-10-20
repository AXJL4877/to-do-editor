//! Right-hand dock: workflow package list. Fullscreen canvas opens with `~`
//! or by clicking a list row. Form/card (成片新建/任务) stay in the crate but
//! are not drawn here.
#[allow(dead_code)]
mod card;
#[allow(dead_code)]
mod content;
#[allow(dead_code)]
mod flow;
#[allow(dead_code)]
mod form;
mod workflow;

use crate::theme::{BORDER, BRAND, DANGER, MUTED, PANEL_2, TEXT, WARN};
use crate::ThumbCache;
use egui::{Color32, Id, Rect, RichText, Ui};
use std::collections::BTreeMap;
use todo_interaction::{
    ExtensionStatus, ParamValue, ProductionEdit, ProductionNodeStatus, ProductionPanelView,
    WorkflowEntryView,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ProductionAction {
    None,
    OpenSettings,
    Start {
        extension: String,
        params: BTreeMap<String, ParamValue>,
        script: String,
    },
    /// `key: None` retries every failed composition of the production.
    Retry {
        production: String,
        key: Option<String>,
    },
    /// Stop generating and drop the card; placed clips stay.
    Remove {
        production: String,
    },
    /// Put back the generated clips the user deleted from the timeline.
    Restore {
        production: String,
    },
    Edit {
        production: String,
        edit: ProductionEdit,
    },
    Upload {
        production: String,
        element: String,
    },
    Thumbnail {
        path: String,
    },
    /// Scaffold a blank user workflow package.
    WorkflowCreate,
    /// Write the open graph (positions + inspector edits) to disk.
    WorkflowSave {
        extension: String,
        positions: BTreeMap<String, (f32, f32)>,
        edits: BTreeMap<String, BTreeMap<String, String>>,
    },
    /// Write inspector params into the extension graph.json.
    WorkflowSaveParams {
        extension: String,
        node_id: String,
        /// Field id → JSON-ish string (numbers as decimal, selects as value).
        params: BTreeMap<String, String>,
    },
    /// Open a file dialog and attach files to a media_bin node.
    WorkflowPickMedia {
        extension: String,
        node_id: String,
    },
    /// Drop one file entry from a media_bin node.
    WorkflowRemoveMedia {
        extension: String,
        node_id: String,
        path: String,
    },
}

/// Which pane the right-hand column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SideTab {
    #[default]
    Properties,
    Production,
}

const TAB_ID: &str = "todo_ui_side_tab";
const DOCK_ID: &str = "todo_ui_production_dock_rect";
const BADGE_ID: &str = "todo_ui_production_dock_badge";
const SELECTED_WF: &str = "todo_ui_workflow_selected";

pub fn side_tab(ctx: &egui::Context) -> SideTab {
    ctx.data(|data| data.get_temp::<SideTab>(Id::new(TAB_ID)))
        .unwrap_or_default()
}

pub fn set_side_tab(ctx: &egui::Context, tab: SideTab) {
    ctx.data_mut(|data| data.insert_temp(Id::new(TAB_ID), tab));
}

/// Toolbar entry: show the dock, or go back to properties if it is showing.
pub fn toggle(ctx: &egui::Context) {
    let next = match side_tab(ctx) {
        SideTab::Production => SideTab::Properties,
        SideTab::Properties => SideTab::Production,
    };
    set_side_tab(ctx, next);
}

pub fn is_open(ctx: &egui::Context) -> bool {
    side_tab(ctx) == SideTab::Production
}

pub fn close(ctx: &egui::Context) {
    set_side_tab(ctx, SideTab::Properties);
}

/// Where the layout reserved the dock this frame (`None`: not showing).
pub(crate) fn set_dock(ctx: &egui::Context, rect: Option<Rect>) {
    ctx.data_mut(|data| data.insert_temp(Id::new(DOCK_ID), rect));
}

pub fn dock_rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|data| data.get_temp::<Option<Rect>>(Id::new(DOCK_ID)))
        .flatten()
}

/// A dot on the tab while something runs or needs the user.
pub(crate) fn dock_badge(ctx: &egui::Context) -> Option<Color32> {
    ctx.data(|data| data.get_temp::<Option<Color32>>(Id::new(BADGE_ID)))
        .flatten()
}

pub(super) fn status_color(status: ProductionNodeStatus) -> Color32 {
    match status {
        ProductionNodeStatus::Queued => MUTED,
        ProductionNodeStatus::Running => BRAND,
        ProductionNodeStatus::Waiting | ProductionNodeStatus::Attention => WARN,
        ProductionNodeStatus::Complete => crate::theme::SUCCESS,
        ProductionNodeStatus::Failed => DANGER,
    }
}

#[allow(dead_code)]
pub(super) fn caption(ui: &mut Ui, step: Option<usize>, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if let Some(step) = step {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 9.0, BORDER);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                step.to_string(),
                egui::FontId::proportional(11.0),
                TEXT,
            );
        }
        ui.label(RichText::new(text).strong().size(13.0).color(TEXT));
    });
    ui.add_space(2.0);
}

fn entry_status_color(status: ExtensionStatus) -> Color32 {
    match status {
        ExtensionStatus::Ready => crate::theme::SUCCESS,
        ExtensionStatus::Checking => MUTED,
        ExtensionStatus::MissingKey => WARN,
        ExtensionStatus::RuntimeMissing | ExtensionStatus::SelfCheckFailed => DANGER,
    }
}

pub fn selected_workflow(ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<Option<String>>(Id::new(SELECTED_WF)))
        .flatten()
}

pub fn set_selected_workflow(ctx: &egui::Context, id: Option<String>) {
    ctx.data_mut(|d| d.insert_temp(Id::new(SELECTED_WF), id));
}

/// Draw the dock into `ui` (its rect is [`dock_rect`]).
pub fn draw_dock(
    ui: &mut Ui,
    view: &ProductionPanelView,
    _thumbs: &mut ThumbCache,
) -> ProductionAction {
    let ctx = ui.ctx().clone();
    let badge = if view
        .productions
        .iter()
        .any(|p| p.status == ProductionNodeStatus::Failed)
    {
        Some(DANGER)
    } else if view.productions.iter().any(|p| {
        matches!(
            p.status,
            ProductionNodeStatus::Running | ProductionNodeStatus::Waiting
        )
    }) {
        Some(BRAND)
    } else {
        None
    };
    ctx.data_mut(|data| data.insert_temp(Id::new(BADGE_ID), badge));

    let mut action = ProductionAction::None;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new("工作流").strong().size(13.0).color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("AI 设置").size(12.0).color(MUTED))
                        .frame(false),
                )
                .on_hover_text("服务地址、Key 与模型")
                .clicked()
            {
                action = ProductionAction::OpenSettings;
            }
        });
    });
    ui.add_space(4.0);
    let rect = ui.max_rect();
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), ui.cursor().top()),
            egui::pos2(rect.right(), ui.cursor().top()),
        ],
        egui::Stroke::new(1.0_f32, BORDER),
    );
    egui::ScrollArea::vertical()
        .id_salt("workflow_dock_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    left: 12,
                    right: 14,
                    top: 8,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if let Some(problem) = &view.service_problem {
                        ui.label(RichText::new(problem).size(12.0).color(WARN));
                        ui.add_space(8.0);
                    }
                    if ui
                        .add_sized(
                            [ui.available_width(), 32.0],
                            egui::Button::new(RichText::new("新建工作流").size(13.0)),
                        )
                        .clicked()
                    {
                        action = ProductionAction::WorkflowCreate;
                    }
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("按 ~ 全屏打开 · Esc 关闭")
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.add_space(8.0);
                    if view.workflows.is_empty() {
                        ui.label(RichText::new("还没有工作流").size(12.5).color(MUTED));
                        ui.label(
                            RichText::new("点上方「新建工作流」开始，或安装带 graph 的扩展包。")
                                .size(11.5)
                                .color(MUTED),
                        );
                    } else {
                        let selected = selected_workflow(&ctx);
                        for entry in &view.workflows {
                            let next = draw_entry(ui, entry, selected.as_deref() == Some(&entry.extension_id));
                            if next != ProductionAction::None {
                                action = next;
                            }
                        }
                    }
                });
        });
    action
}

fn draw_entry(ui: &mut Ui, entry: &WorkflowEntryView, selected: bool) -> ProductionAction {
    let frame = egui::Frame::NONE
        .fill(if selected {
            egui::Color32::from_rgb(0x1d, 0x30, 0x3d)
        } else {
            PANEL_2
        })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if selected { BRAND } else { BORDER },
        ))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(egui::CornerRadius::same(6));
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&entry.name).strong().size(13.5).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tag = if entry.origin_user { "我的" } else { "内置" };
                ui.label(RichText::new(tag).size(11.0).color(MUTED));
                ui.colored_label(entry_status_color(entry.status), "●");
            });
        });
        if !entry.summary.is_empty() {
            ui.label(RichText::new(&entry.summary).size(11.5).color(MUTED));
        }
    });
    let response = ui.interact(
        inner.response.rect,
        Id::new(("workflow_entry", &entry.extension_id)),
        egui::Sense::click(),
    );
    if response.clicked() {
        set_selected_workflow(ui.ctx(), Some(entry.extension_id.clone()));
        workflow::open(ui.ctx(), entry.extension_id.clone());
    }
    ui.add_space(6.0);
    ProductionAction::None
}

/// Draw fullscreen canvas + handle `~` / Esc. Call every editor frame.
pub fn draw_workflow_overlay(
    ctx: &egui::Context,
    panel: &ProductionPanelView,
    graph: Option<&todo_interaction::WorkflowPanelView>,
    title: &str,
) -> ProductionAction {
    workflow::handle_hotkeys(ctx, panel);
    let Some(extension_id) = workflow::open_id(ctx) else {
        return ProductionAction::None;
    };
    let Some(graph) = graph else {
        return ProductionAction::None;
    };
    workflow::draw_fullscreen(ctx, &extension_id, title, graph)
}

pub use workflow::{
    is_blocking_input, is_open as workflow_fullscreen_open, open as open_workflow,
    open_id as workflow_open_id,
};

#[cfg(test)]
mod tests;
