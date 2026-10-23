//! Resizable workspace panes. Layout preferences belong to the UI, not the project.

use egui::{CursorIcon, Id, Rect, Sense, Ui, UiBuilder};
use todo_interaction::{EditorViewModel, Intent};

use super::{
    assets, inspector, library, monitor, production, project_settings, shortcuts, timeline,
    toolbar, MonitorHole,
};
use crate::format::band_heights;
use crate::peaks::PeaksCache;
use crate::theme::{BG, BORDER, BRAND, DIM, MUTED, PANEL, TEXT, TOOLBAR_H};
use crate::thumbs::ThumbCache;

const LIBRARY_ID: &str = "todo_ui_library_width";
const HEIGHT_ID: &str = "todo_ui_monitor_height";
const INSPECTOR_ID: &str = "todo_ui_inspector_width";
/// The AI production tab needs more room than clip properties; it keeps
/// its own preferred width.
const DOCK_ID: &str = "todo_ui_production_dock_width";
const SIDE_HEADER_H: f32 = 34.0;
const SPLIT: f32 = 7.0;
pub(super) const MONITOR_MIN: f32 = 96.0;

pub fn draw_editor(
    ui: &mut Ui,
    vm: &EditorViewModel,
    peaks: &PeaksCache,
    thumbs: &mut ThumbCache,
) -> (Vec<Intent>, Option<MonitorHole>) {
    let mut intents = Vec::new();
    let settings_was_open = project_settings::is_open(ui.ctx());
    let workflow_was_open = production::workflow_fullscreen_open(ui.ctx());
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    ui.painter().rect_filled(full, 0.0, BG);

    let toolbar_h = TOOLBAR_H.min(full.height());
    let upper_top = full.top() + toolbar_h;
    let available_h = (full.height() - toolbar_h - SPLIT).max(0.0);
    // Keep one library row and the search/actions visible at the smallest split.
    let min_upper = (MONITOR_MIN + 80.0).min(available_h * 0.55);
    let max_upper = (available_h - 160.0_f32.min(available_h * 0.45)).max(min_upper);
    let default_h = band_heights(full.height()).1.clamp(min_upper, max_upper);
    // Preferred sizes survive temporary window shrinks; only the drawn rect is clamped.
    let mut preferred_h = remembered(ui, HEIGHT_ID, default_h);
    let mut upper_h = preferred_h.clamp(min_upper, max_upper);

    let available_w = (full.width() - SPLIT * 2.0).max(0.0);
    let tab = production::side_tab(ui.ctx());
    let docked = tab == production::SideTab::Production;
    let (side_id, min_w, max_w, max_share, default_w) = if docked {
        (DOCK_ID, 320.0_f32, 560.0_f32, 0.42_f32, 400.0_f32)
    } else {
        (INSPECTOR_ID, 220.0, 400.0, 0.38, 280.0)
    };
    let min_inspector = min_w.min(available_w * 0.25);
    let max_inspector = max_w.min(available_w * max_share).max(min_inspector);
    let default_inspector = default_w.clamp(min_inspector, max_inspector);
    let mut preferred_inspector = remembered(ui, side_id, default_inspector);
    let mut inspector_w = preferred_inspector.clamp(min_inspector, max_inspector);
    let inspector_split = Rect::from_min_size(
        egui::pos2(full.right() - inspector_w - SPLIT, upper_top),
        egui::vec2(SPLIT, upper_h),
    );
    let inspector_response = ui.interact(
        inspector_split,
        Id::new(INSPECTOR_ID),
        Sense::click_and_drag(),
    );
    if inspector_response.double_clicked() {
        preferred_inspector = default_inspector;
        inspector_w = preferred_inspector.clamp(min_inspector, max_inspector);
    } else if inspector_response.dragged() {
        preferred_inspector =
            (inspector_w - inspector_response.drag_delta().x).clamp(min_inspector, max_inspector);
        inspector_w = preferred_inspector;
    }
    let min_library = 180.0_f32.min(available_w * 0.22);
    let max_library =
        (available_w - inspector_w - 320.0_f32.min(available_w * 0.35)).max(min_library);
    let default_w = (full.width() * 0.25)
        .clamp(min_library, max_library)
        .min(340.0);
    let mut preferred_library = remembered(ui, LIBRARY_ID, default_w);
    let mut library_w = preferred_library.clamp(min_library, max_library);

    let horizontal = Rect::from_min_size(
        egui::pos2(full.left(), upper_top + upper_h),
        egui::vec2(full.width(), SPLIT),
    );
    let height_response = ui.interact(horizontal, Id::new(HEIGHT_ID), Sense::click_and_drag());
    if height_response.double_clicked() {
        preferred_h = default_h;
        upper_h = preferred_h.clamp(min_upper, max_upper);
    } else if height_response.dragged() {
        preferred_h = (upper_h + height_response.drag_delta().y).clamp(min_upper, max_upper);
        upper_h = preferred_h;
    }
    let vertical = Rect::from_min_size(
        egui::pos2(full.left() + library_w, upper_top),
        egui::vec2(SPLIT, upper_h),
    );
    let width_response = ui.interact(vertical, Id::new(LIBRARY_ID), Sense::click_and_drag());
    if width_response.double_clicked() {
        preferred_library = default_w;
        library_w = preferred_library.clamp(min_library, max_library);
    } else if width_response.dragged() {
        preferred_library =
            (library_w + width_response.drag_delta().x).clamp(min_library, max_library);
        library_w = preferred_library;
    }
    // A minimized/zero first frame must not initialize collapsed preferences.
    if full.is_finite()
        && full.is_positive()
        && !ui.input(|input| input.viewport().minimized.unwrap_or(false))
    {
        ui.ctx().data_mut(|data| {
            data.insert_persisted(Id::new(HEIGHT_ID), preferred_h);
            data.insert_persisted(Id::new(LIBRARY_ID), preferred_library);
            data.insert_persisted(Id::new(side_id), preferred_inspector);
        });
    }

    let toolbar_rect = Rect::from_min_size(full.min, egui::vec2(full.width(), toolbar_h));
    intents.extend(toolbar::draw(
        &mut pane(ui, toolbar_rect, "editor_toolbar"),
        vm,
    ));
    let settings_intents = project_settings::draw(ui.ctx(), vm);
    let settings_blocks_input = settings_was_open
        || project_settings::is_open(ui.ctx())
        || workflow_was_open
        || production::is_blocking_input(ui.ctx());
    let library_rect = Rect::from_min_size(
        egui::pos2(full.left(), upper_top),
        egui::vec2(library_w, upper_h),
    );
    // Media, motions or fonts; the app draws the motion and font panels.
    let left_header = Rect::from_min_size(
        library_rect.min,
        egui::vec2(library_rect.width(), assets::HEADER_H.min(library_rect.height())),
    );
    assets::header(ui, left_header);
    let left_body = Rect::from_min_max(egui::pos2(library_rect.left(), left_header.bottom()), library_rect.max);
    // Fresh each frame so a prior Media-tab shortcut does not suppress the
    // global shortcut path after switching left tabs.
    library::clear_library_keys_handled(ui.ctx());
    match assets::left_tab(ui.ctx()) {
        assets::LeftTab::Media => {
            assets::set_dock(ui.ctx(), None);
            intents.extend(library::draw(
                &mut pane(ui, left_body, "editor_library"),
                vm,
                thumbs,
            ));
        }
        tab => {
            ui.painter().rect_filled(left_body, 0.0, PANEL);
            assets::set_dock(ui.ctx(), Some((left_body, tab)));
        }
    }
    let monitor_rect = Rect::from_min_max(
        egui::pos2(library_rect.right() + SPLIT, upper_top),
        egui::pos2(full.right() - inspector_w - SPLIT, upper_top + upper_h),
    );
    let (monitor_intents, hole) = monitor::draw(&mut pane(ui, monitor_rect, "editor_monitor"), vm);
    intents.extend(monitor_intents);
    let inspector_rect = Rect::from_min_max(
        egui::pos2(full.right() - inspector_w, upper_top),
        egui::pos2(full.right(), upper_top + upper_h),
    );
    ui.painter().rect_filled(inspector_rect, 0.0, PANEL);
    let header = Rect::from_min_size(
        inspector_rect.min,
        egui::vec2(
            inspector_rect.width(),
            SIDE_HEADER_H.min(inspector_rect.height()),
        ),
    );
    side_header(ui, header, tab);
    let side_body = Rect::from_min_max(
        egui::pos2(inspector_rect.left(), header.bottom()),
        inspector_rect.max,
    );
    if docked {
        production::set_dock(ui.ctx(), Some(side_body));
    } else {
        production::set_dock(ui.ctx(), None);
        intents.extend(inspector::draw(
            &mut pane(ui, side_body, "editor_inspector"),
            vm.properties.as_ref(),
        ));
    }
    if let Some(hole) = hole {
        intents.push(Intent::PreviewViewport {
            width: f64::from(hole.w),
            height: f64::from(hole.h),
        });
    }
    let timeline_rect = Rect::from_min_max(
        egui::pos2(full.left(), upper_top + upper_h + SPLIT),
        full.max,
    );
    intents.extend(timeline::draw(
        &mut pane(ui, timeline_rect, "editor_timeline"),
        &vm.timeline,
        peaks,
    ));
    paint_splitter(
        ui,
        Rect::from_min_size(
            egui::pos2(full.left(), upper_top + upper_h),
            egui::vec2(full.width(), SPLIT),
        ),
        height_response,
        false,
    );
    paint_splitter(
        ui,
        Rect::from_min_size(
            egui::pos2(full.left() + library_w, upper_top),
            egui::vec2(SPLIT, upper_h),
        ),
        width_response,
        true,
    );
    if !settings_blocks_input {
        intents.extend(shortcuts::collect(ui.ctx(), vm));
    }
    paint_splitter(
        ui,
        Rect::from_min_size(
            egui::pos2(inspector_rect.left() - SPLIT, upper_top),
            egui::vec2(SPLIT, upper_h),
        ),
        inspector_response,
        true,
    );
    if settings_blocks_input {
        // Some timeline gestures read raw input directly rather than widget
        // responses. They must not reach the controller through a modal.
        (settings_intents, hole)
    } else {
        (intents, hole)
    }
}

/// Tabs over the right-hand column: clip properties or the Agent dock.
fn side_header(ui: &mut Ui, rect: Rect, tab: production::SideTab) {
    let painter = ui.painter_at(rect);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0_f32, BORDER),
    );
    let mut x = rect.left() + 4.0;
    let badge = production::dock_badge(ui.ctx());
    for (label, target) in [
        ("属性", production::SideTab::Properties),
        ("Agent", production::SideTab::Production),
    ] {
        let galley =
            ui.painter()
                .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), TEXT);
        let width = galley.size().x + 24.0;
        let tab_rect = Rect::from_min_size(
            egui::pos2(x, rect.top()),
            egui::vec2(width, rect.height() - 1.0),
        );
        let response = ui.interact(
            tab_rect,
            Id::new(("todo_ui_side_tab", label)),
            Sense::click(),
        );
        let active = tab == target;
        let color = if active {
            TEXT
        } else if response.hovered() {
            TEXT.gamma_multiply(0.85)
        } else {
            MUTED
        };
        painter.text(
            tab_rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5),
            color,
        );
        if active {
            painter.line_segment(
                [
                    egui::pos2(tab_rect.left() + 8.0, rect.bottom() - 1.0),
                    egui::pos2(tab_rect.right() - 8.0, rect.bottom() - 1.0),
                ],
                egui::Stroke::new(2.0_f32, BRAND),
            );
        }
        if target == production::SideTab::Production {
            if let Some(color) = badge {
                painter.circle_filled(
                    egui::pos2(tab_rect.right() - 7.0, tab_rect.top() + 10.0),
                    3.0,
                    color,
                );
            }
        }
        if response.clicked() && !active {
            production::set_side_tab(ui.ctx(), target);
        }
        x += width;
    }
}

fn remembered(ui: &Ui, key: &str, fallback: f32) -> f32 {
    ui.ctx()
        .data_mut(|data| data.get_persisted::<f32>(Id::new(key)))
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(fallback)
}

fn pane(ui: &mut Ui, rect: Rect, key: &str) -> Ui {
    let mut child = ui.new_child(UiBuilder::new().id_salt(key).max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child
}

fn paint_splitter(ui: &Ui, rect: Rect, response: egui::Response, vertical: bool) {
    let active = response.hovered() || response.dragged();
    if active {
        ui.ctx().set_cursor_icon(if vertical {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        });
    }
    ui.painter()
        .rect_filled(rect, 0.0, if active { PANEL } else { BG });
    let line = if vertical {
        [rect.center_top(), rect.center_bottom()]
    } else {
        [rect.left_center(), rect.right_center()]
    };
    ui.painter().line_segment(
        line,
        egui::Stroke::new(1.0_f32, if active { BRAND } else { BORDER }),
    );
    let grip = if vertical {
        egui::vec2(0.0, 12.0)
    } else {
        egui::vec2(12.0, 0.0)
    };
    ui.painter().line_segment(
        [rect.center() - grip, rect.center() + grip],
        egui::Stroke::new(3.0_f32, if active { BRAND } else { DIM }),
    );
}

#[cfg(test)]
mod tests;
