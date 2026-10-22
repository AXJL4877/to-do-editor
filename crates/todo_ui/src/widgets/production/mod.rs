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

use crate::theme::{button, Btn, BORDER, BRAND, DANGER, MUTED, PANEL_2, TEXT, WARN};
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
    /// Open a local file with the OS default app (e.g. system video player).
    OpenExternal {
        path: String,
    },
    /// 运行整条工作流（空 script 用图里的文案）。
    /// `edits` 是检查器里还没保存的临时改动，运行按它生效。
    WorkflowRun {
        extension: String,
        script: String,
        edits: BTreeMap<String, BTreeMap<String, String>>,
    },
    /// 停止正在跑的工作流（取消在途请求，保留已完成的节点）。
    WorkflowStop {
        extension: String,
    },
    /// Scaffold a blank user workflow package.
    WorkflowCreate,
    /// Delete a user workflow package from disk.
    WorkflowDelete {
        extension: String,
    },
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
    /// Canvas/toolbar import: pick files, then create media_bin or append to target.
    WorkflowImportMediaPick {
        extension: String,
        x: f32,
        y: f32,
        /// Drop onto this media_bin/relay when set; otherwise create a new media_bin.
        target_node: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// OS drop / already-picked paths onto the canvas.
    WorkflowImportMedia {
        extension: String,
        paths: Vec<std::path::PathBuf>,
        x: f32,
        y: f32,
        target_node: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Drop one file entry from a media_bin node.
    WorkflowRemoveMedia {
        extension: String,
        node_id: String,
        path: String,
    },
    /// Clear disk cache for image_gen (etc.) so the next run regenerates.
    WorkflowClearNodeCache {
        extension: String,
        node_id: String,
    },
    /// Insert a catalog node onto the canvas.
    WorkflowAddNode {
        extension: String,
        node_type: String,
        x: f32,
        y: f32,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Clone a node (no edges).
    WorkflowDuplicateNode {
        extension: String,
        node_id: String,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Delete a node and its incident edges.
    WorkflowDeleteNode {
        extension: String,
        node_id: String,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Paste a previously copied node.
    WorkflowPasteNode {
        extension: String,
        node_type: String,
        label: Option<String>,
        /// JSON object of params (`{}` if empty).
        params_json: String,
        x: f32,
        y: f32,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Wire an output port to an input port.
    WorkflowConnect {
        extension: String,
        from: String,
        to: String,
        out: Option<String>,
        input: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Remove one edge.
    WorkflowDisconnect {
        extension: String,
        from: String,
        to: String,
        out: Option<String>,
        input: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Jump the editor playhead (subtitle preview on the program canvas).
    SeekPlayhead {
        time: f64,
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
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    let _ = thumbs;
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
            if button(ui, "AI 设置", Btn::Ghost, true).clicked() {
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
                            [ui.available_width(), 34.0],
                            egui::Button::new(
                                RichText::new("新建工作流")
                                    .size(13.0)
                                    .color(egui::Color32::from_rgb(0x08, 0x14, 0x1c)),
                            )
                            .fill(BRAND)
                            .stroke(egui::Stroke::new(1.0_f32, BRAND))
                            .corner_radius(egui::CornerRadius::ZERO),
                        )
                        .clicked()
                    {
                        action = ProductionAction::WorkflowCreate;
                    }
                    ui.add_space(12.0);
                    if view.workflows.is_empty() {
                        ui.label(RichText::new("还没有工作流").size(13.0).color(MUTED));
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
    let mut action = ProductionAction::None;
    let tag = if entry.origin_user { "我的" } else { "内置" };
    let fill = if selected {
        egui::Color32::from_rgb(0x1d, 0x30, 0x3d)
    } else {
        PANEL_2
    };
    let stroke = if selected { BRAND } else { BORDER };
    let row_w = ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let open_w = if entry.origin_user {
            (row_w - 58.0).max(80.0)
        } else {
            row_w
        };
        let response = ui.add_sized(
            [open_w, 40.0],
            egui::Button::new(
                RichText::new(format!("{}  ·  {}", entry.name, tag))
                    .size(13.5)
                    .color(TEXT),
            )
            .fill(fill)
            .stroke(egui::Stroke::new(1.0_f32, stroke))
            .corner_radius(egui::CornerRadius::ZERO),
        );
        let status = entry_status_color(entry.status);
        let dot = egui::pos2(response.rect.right() - 14.0, response.rect.center().y);
        ui.painter().circle_filled(dot, 3.5, status);
        if response.clicked() {
            set_selected_workflow(ui.ctx(), Some(entry.extension_id.clone()));
            workflow::open(ui.ctx(), entry.extension_id.clone());
        }
        if entry.origin_user
            && button(ui, "删除", Btn::Danger, true).clicked()
        {
            action = ProductionAction::WorkflowDelete {
                extension: entry.extension_id.clone(),
            };
        }
    });
    ui.add_space(6.0);
    action
}

/// Draw fullscreen canvas + handle `~` / Esc. Call every editor frame.
pub fn draw_workflow_overlay(
    ctx: &egui::Context,
    panel: &ProductionPanelView,
    graph: Option<&todo_interaction::WorkflowPanelView>,
    title: &str,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    workflow::handle_hotkeys(ctx, panel);
    let Some(extension_id) = workflow::open_id(ctx) else {
        return ProductionAction::None;
    };
    match graph {
        Some(graph) => workflow::draw_fullscreen(ctx, &extension_id, title, graph, thumbs),
        None => workflow::draw_unavailable(ctx, &extension_id, title),
    }
}

pub use workflow::{
    close as close_workflow, is_blocking_input, is_open as workflow_fullscreen_open,
    open as open_workflow, open_id as workflow_open_id, terminal_hole as workflow_terminal_hole,
};

#[cfg(test)]
mod tests;
