//! Workflow graph UI removed. These helpers keep API compatibility for the
//! host (open-id queries / input blocking) while the right dock is Agent chat.

use super::set_selected_workflow;
use egui::Id;

const OPEN_ID: &str = "todo_ui_workflow_panel_open";
const TERMINAL_HOLE_ID: &str = "todo_ui_workflow_terminal_hole";

pub fn open(ctx: &egui::Context, extension_id: String) {
    // Graph UI is gone — remember selection only; never show fullscreen.
    set_selected_workflow(ctx, Some(extension_id));
}

pub fn close(ctx: &egui::Context) {
    set_terminal_hole(ctx, None);
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), None::<String>));
}

pub fn open_id(ctx: &egui::Context) -> Option<String> {
    ctx.data(|data| data.get_temp::<Option<String>>(Id::new(OPEN_ID)))
        .flatten()
}

pub fn is_open(ctx: &egui::Context) -> bool {
    open_id(ctx).is_some()
}

pub fn terminal_hole(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|data| data.get_temp::<Option<egui::Rect>>(Id::new(TERMINAL_HOLE_ID)))
        .flatten()
}

fn set_terminal_hole(ctx: &egui::Context, hole: Option<egui::Rect>) {
    ctx.data_mut(|data| data.insert_temp(Id::new(TERMINAL_HOLE_ID), hole));
}

pub fn is_blocking_input(ctx: &egui::Context) -> bool {
    is_open(ctx)
}
