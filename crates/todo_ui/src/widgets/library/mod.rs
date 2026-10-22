//! VS Code–style media explorer: one indented tree for bins and media.

mod icons;

use egui::{Color32, Sense, Ui};
use todo_interaction::{
    EditorFocus, EditorViewModel, ExplorerNode, Intent, LibraryItemView, MediaKind,
};

use crate::format::clock;
use crate::theme::{button, Btn, BRAND, DIM, FIELD, MUTED, PANEL, PANEL_2, TEXT};
use crate::thumbs::ThumbCache;

const ROW_H: f32 = 22.0;
const INDENT: f32 = 16.0;
const GUIDE: Color32 = Color32::from_rgb(0x3a, 0x3d, 0x44);

#[derive(Debug, Clone)]
pub(super) struct MediaDrag {
    /// All media moved by this drag (multi-select). First id is the grab row.
    pub media_ids: Vec<String>,
}

impl MediaDrag {
    pub fn primary(&self) -> &str {
        self.media_ids
            .first()
            .map(String::as_str)
            .unwrap_or("")
    }

    fn for_row(item_id: &str, selected_ids: &[String], row_selected: bool) -> Self {
        let media_ids = if row_selected && selected_ids.len() > 1 {
            let mut ids = selected_ids.to_vec();
            // Keep the grabbed row first so timeline/hover still keys off it.
            if let Some(pos) = ids.iter().position(|id| id == item_id) {
                ids.swap(0, pos);
            }
            ids
        } else {
            vec![item_id.to_owned()]
        };
        Self { media_ids }
    }
}

#[derive(Debug, Clone)]
struct FolderDrag {
    folder_id: String,
}

#[derive(Debug, Clone)]
enum DropPreview {
    Media { name: String, kind: MediaKind },
    Folder { name: String },
}

#[derive(Clone, Default)]
struct RenameDraft {
    id: String,
    name: String,
}

fn rename_key() -> egui::Id {
    egui::Id::new("todo_ui_library_rename_folder")
}

fn click_modifiers(ui: &Ui) -> (bool, bool) {
    ui.input(|i| {
        let cmd = i.modifiers.ctrl || i.modifiers.command;
        let shift = i.modifiers.shift;
        // Windows Explorer: Shift = range, Ctrl = toggle; Ctrl+Shift still ranges.
        (cmd && !shift, shift)
    })
}

fn select_media(id: String, ui: &Ui) -> Intent {
    let (additive, range) = click_modifiers(ui);
    Intent::LibrarySelect {
        id,
        additive,
        range,
    }
}

pub(super) fn draw(ui: &mut Ui, vm: &EditorViewModel, thumbs: &mut ThumbCache) -> Vec<Intent> {
    egui::Frame::NONE
        .fill(PANEL)
        .inner_margin(egui::Margin::symmetric(6, 6))
        .show(ui, |ui| draw_contents(ui, vm, thumbs))
        .inner
}

fn draw_contents(ui: &mut Ui, vm: &EditorViewModel, thumbs: &mut ThumbCache) -> Vec<Intent> {
    let mut intents = Vec::new();
    let background = ui.interact(
        ui.max_rect(),
        ui.id().with("library_background"),
        Sense::click(),
    );
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("媒体库").size(11.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(
                ui,
                if vm.importing { "…" } else { "+" },
                Btn::Ghost,
                !vm.importing,
            )
            .on_hover_text("导入到当前料箱")
            .clicked()
            {
                intents.push(Intent::Focus(EditorFocus::Library));
                intents.push(Intent::ImportPressed);
            }
            if button(ui, "新料箱", Btn::Ghost, true)
                .on_hover_text("在当前料箱下新建")
                .clicked()
            {
                intents.push(Intent::Focus(EditorFocus::Library));
                intents.push(Intent::LibraryCreateFolder {
                    name: next_folder_name(vm),
                });
            }
        });
    });

    let mut query = vm.library.query.clone();
    let search = crate::theme::with_input_radius(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut query)
                .hint_text("搜索…")
                .margin(egui::Margin::symmetric(6, 4))
                .background_color(FIELD)
                .desired_width(f32::INFINITY),
        )
    });
    if search.gained_focus() {
        intents.push(Intent::Focus(EditorFocus::Library));
    }
    if search.changed() {
        intents.push(Intent::LibraryQuery(query));
    }
    ui.add_space(4.0);

    let list_h = (ui.available_height() - 4.0).max(40.0);
    egui::ScrollArea::vertical()
        .id_salt("media_explorer")
        .auto_shrink([false, false])
        .max_height(list_h)
        .show(ui, |ui| {
            intents.extend(draw_root(ui, vm));
            if vm.library.explorer.is_empty() {
                ui.add_space(8.0);
                ui.weak(if vm.library.searching {
                    "无匹配"
                } else {
                    "空 — 导入或新建料箱"
                });
            } else {
                for node in &vm.library.explorer {
                    intents.extend(draw_node(ui, vm, node, thumbs, &[]));
                }
            }
        });

    if background.clicked() {
        ui.ctx().data_mut(|d| d.remove::<RenameDraft>(rename_key()));
        intents.push(Intent::Focus(EditorFocus::Library));
        intents.push(Intent::LibrarySelectFolder(None));
    }

    // F2 renames the selected media bin (same as Explorer).
    if vm.focus == EditorFocus::Library
        && !ui.ctx().wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::F2))
    {
        if let Some(id) = vm.library.selected_folder_id.clone() {
            if let Some(name) = folder_name(vm, &id) {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        rename_key(),
                        RenameDraft {
                            id: id.clone(),
                            name,
                        },
                    );
                });
                intents.push(Intent::Focus(EditorFocus::Library));
            }
        }
    }
    intents
}

/// A permanent destination for importing or moving items back out of a bin.
fn draw_root(ui: &mut Ui, vm: &EditorViewModel) -> Vec<Intent> {
    let mut intents = Vec::new();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::hover());
    let response = ui.interact(rect, ui.id().with("library_root"), Sense::click());
    let selected = vm.library.selected_id.is_none() && vm.library.selected_folder_id.is_none();
    paint_row(ui, rect, selected, response.hovered());
    let x0 = rect.left() + 4.0;
    icons::folder(ui.painter(), icon_rect(rect, x0 + 14.0), true);
    paint_name(ui, rect, x0 + 34.0, "项目根目录", MUTED, rect.right() - 6.0);
    if response.clicked() {
        intents.push(Intent::LibrarySelectFolder(None));
    }
    intents.extend(drop_on_folder(ui, &response, None));
    if let Some(preview) = drop_preview(vm, &response, None) {
        draw_drop_preview(ui, preview, 0, &[]);
    }
    response.on_hover_text("导入到根目录");
    intents
}

fn folder_name(vm: &EditorViewModel, id: &str) -> Option<String> {
    fn find(nodes: &[ExplorerNode], id: &str) -> Option<String> {
        for node in nodes {
            match node {
                ExplorerNode::Folder {
                    id: fid,
                    name,
                    children,
                    ..
                } => {
                    if fid == id {
                        return Some(name.clone());
                    }
                    if let Some(found) = find(children, id) {
                        return Some(found);
                    }
                }
                ExplorerNode::Media { .. } => {}
            }
        }
        None
    }
    find(&vm.library.explorer, id)
}

fn next_folder_name(vm: &EditorViewModel) -> String {
    let base = "新建料箱";
    let names: Vec<String> = collect_folder_names(&vm.library.explorer);
    if !names.iter().any(|n| n == base) {
        return base.into();
    }
    for index in 2..100 {
        let candidate = format!("{base} {index}");
        if !names.contains(&candidate) {
            return candidate;
        }
    }
    base.into()
}

fn collect_folder_names(nodes: &[ExplorerNode]) -> Vec<String> {
    let mut out = Vec::new();
    for node in nodes {
        if let ExplorerNode::Folder { name, children, .. } = node {
            out.push(name.clone());
            out.extend(collect_folder_names(children));
        }
    }
    out
}

fn draw_node(
    ui: &mut Ui,
    vm: &EditorViewModel,
    node: &ExplorerNode,
    thumbs: &mut ThumbCache,
    guides: &[f32],
) -> Vec<Intent> {
    match node {
        ExplorerNode::Folder {
            id,
            name,
            depth,
            expanded,
            selected,
            children,
        } => {
            let mut intents = Vec::new();
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::hover());
            let response = ui.interact(rect, ui.id().with(("folder", id)), Sense::click_and_drag());
            paint_row(ui, rect, *selected, response.hovered());
            paint_guides(ui, rect, guides);

            let x0 = rect.left() + 4.0 + *depth as f32 * INDENT;
            icons::chevron(
                ui.painter(),
                egui::Rect::from_min_max(
                    egui::pos2(x0, rect.top()),
                    egui::pos2(x0 + 14.0, rect.bottom()),
                ),
                *expanded,
                MUTED,
            );
            icons::folder(ui.painter(), icon_rect(rect, x0 + 14.0), *expanded);

            let renaming = ui
                .ctx()
                .data(|d| d.get_temp::<RenameDraft>(rename_key()))
                .filter(|draft| draft.id == *id);
            let is_renaming = renaming.is_some();
            if let Some(mut draft) = renaming {
                let edit_rect = egui::Rect::from_min_max(
                    egui::pos2(x0 + 34.0, rect.top() + 1.0),
                    egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0),
                );
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt(("rename_folder", id))
                        .max_rect(edit_rect),
                );
                child.set_clip_rect(edit_rect);
                let edit = crate::theme::with_input_radius(&mut child, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.name)
                            .desired_width(edit_rect.width())
                            .margin(egui::Margin::symmetric(4, 1))
                            .font(egui::FontId::proportional(12.0)),
                    )
                });
                edit.request_focus();
                let commit = edit.lost_focus()
                    && !ui.input(|i| i.key_pressed(egui::Key::Escape));
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if enter || commit {
                    let trimmed = draft.name.trim();
                    if !trimmed.is_empty() && trimmed != name {
                        intents.push(Intent::LibraryRenameFolder {
                            id: id.clone(),
                            name: trimmed.to_owned(),
                        });
                    }
                    ui.ctx().data_mut(|d| d.remove::<RenameDraft>(rename_key()));
                } else if cancel {
                    ui.ctx().data_mut(|d| d.remove::<RenameDraft>(rename_key()));
                } else {
                    ui.ctx().data_mut(|d| d.insert_temp(rename_key(), draft));
                }
            } else {
                paint_name(ui, rect, x0 + 34.0, name, TEXT, rect.right() - 6.0);
            }

            response.dnd_set_drag_payload(FolderDrag {
                folder_id: id.clone(),
            });
            intents.extend(drop_on_folder(ui, &response, Some(id.as_str())));

            let preview = drop_preview(vm, &response, Some(id.as_str()));

            if !is_renaming && response.clicked() {
                ui.ctx().data_mut(|d| d.remove::<RenameDraft>(rename_key()));
                intents.push(Intent::Focus(EditorFocus::Library));
                intents.push(Intent::LibrarySelectFolder(Some(id.clone())));
                let on_chevron = response
                    .interact_pointer_pos()
                    .is_some_and(|pos| (x0..x0 + 14.0).contains(&pos.x));
                if on_chevron || !expanded {
                    intents.push(Intent::LibraryToggleFolder(id.clone()));
                }
            }
            if !is_renaming && (response.secondary_clicked() || response.drag_started()) {
                intents.push(Intent::Focus(EditorFocus::Library));
                intents.push(Intent::LibrarySelectFolder(Some(id.clone())));
            }

            response.context_menu(|ui| {
                if ui.button("重命名 (F2)").clicked() {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            rename_key(),
                            RenameDraft {
                                id: id.clone(),
                                name: name.clone(),
                            },
                        );
                    });
                    intents.push(Intent::Focus(EditorFocus::Library));
                    intents.push(Intent::LibrarySelectFolder(Some(id.clone())));
                    ui.close_menu();
                }
                if ui
                    .button("删除料箱 (Del)")
                    .on_hover_text("删除料箱，素材移到上一级")
                    .clicked()
                {
                    intents.push(Intent::LibraryDeleteFolder { id: id.clone() });
                    ui.close_menu();
                }
            });

            if *expanded {
                let mut next_guides = guides.to_vec();
                next_guides.push(x0 + 5.0);
                for child in children {
                    intents.extend(draw_node(ui, vm, child, thumbs, &next_guides));
                }
            }
            if let Some(preview) = preview {
                let mut preview_guides = guides.to_vec();
                preview_guides.push(x0 + 5.0);
                draw_drop_preview(ui, preview, *depth + 1, &preview_guides);
            }
            intents
        }
        ExplorerNode::Media { depth, item } => {
            draw_media_leaf(ui, vm, item, *depth, guides, thumbs)
        }
    }
}

fn paint_row(ui: &Ui, rect: egui::Rect, selected: bool, hovered: bool) {
    if selected {
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(0x2b, 0x3c, 0x49));
    } else if hovered {
        ui.painter().rect_filled(rect, 0.0, PANEL_2);
    }
}

fn icon_rect(row: egui::Rect, left: f32) -> egui::Rect {
    egui::Rect::from_center_size(
        egui::pos2(left + 8.0, row.center().y),
        egui::vec2(16.0, 16.0),
    )
}

fn paint_name(ui: &Ui, rect: egui::Rect, x: f32, name: &str, color: Color32, right: f32) {
    let mut job = egui::text::LayoutJob::simple(
        name.to_owned(),
        egui::FontId::proportional(12.0),
        color,
        (right - x).max(0.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let y = rect.center().y - galley.size().y / 2.0;
    ui.painter().galley(egui::pos2(x, y), galley, color);
}

fn paint_guides(ui: &Ui, rect: egui::Rect, guides: &[f32]) {
    for &x in guides {
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, GUIDE),
        );
    }
}

fn draw_media_leaf(
    ui: &mut Ui,
    vm: &EditorViewModel,
    entry: &LibraryItemView,
    depth: usize,
    guides: &[f32],
    thumbs: &mut ThumbCache,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let item = &entry.item;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::hover());
    let response = ui.interact(
        rect,
        ui.id().with(("media", &item.id)),
        Sense::click_and_drag(),
    );
    paint_row(ui, rect, entry.selected, response.hovered());
    paint_guides(ui, rect, guides);

    let x0 = rect.left() + 4.0 + depth as f32 * INDENT;
    icons::media(ui.painter(), icon_rect(rect, x0 + 14.0), item.kind);

    let name_x = x0 + 34.0;
    let dur = if matches!(item.kind, todo_interaction::MediaKind::Image) {
        clock(item.place_duration)
    } else {
        clock(item.duration)
    };
    let dur_w = 36.0_f32;
    paint_name(
        ui,
        rect,
        name_x,
        &item.name,
        TEXT,
        rect.right() - dur_w - 8.0,
    );

    ui.painter().text(
        egui::pos2(rect.right() - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        dur,
        egui::FontId::monospace(10.0),
        DIM,
    );

    // Tiny hover thumb preview only — keep row compact.
    let _ = thumbs;

    response.dnd_set_drag_payload(MediaDrag::for_row(
        &item.id,
        &vm.library.selected_ids,
        entry.selected,
    ));
    if response.clicked() {
        intents.push(Intent::Focus(EditorFocus::Library));
        intents.push(select_media(item.id.clone(), ui));
    } else if response.drag_started() {
        // Keep a multi-selection when dragging an already-selected row.
        intents.push(Intent::Focus(EditorFocus::Library));
        if !entry.selected {
            intents.push(Intent::LibrarySelect {
                id: item.id.clone(),
                additive: false,
                range: false,
            });
        }
    }
    if response.double_clicked() {
        intents.push(Intent::OpenSourceMonitor {
            media_id: item.id.clone(),
        });
    }
    if response.secondary_clicked() {
        // Right-click keeps multi-selection if the row is already selected.
        intents.push(Intent::Focus(EditorFocus::Library));
        if !entry.selected {
            intents.push(Intent::LibrarySelect {
                id: item.id.clone(),
                additive: false,
                range: false,
            });
        }
    }
    response.clone().on_hover_text(if item.path.is_some() {
        "拖到时间线 · 双击打开"
    } else {
        "源文件不可用"
    });
    response.context_menu(|ui| {
        if ui.button("追加到序列").clicked() {
            intents.push(Intent::PlaceMedia {
                media_id: item.id.clone(),
                track_id: None,
                time: None,
            });
            ui.close_menu();
        }
        if ui.button("放到播放头").clicked() {
            intents.push(Intent::PlaceMedia {
                media_id: item.id.clone(),
                track_id: None,
                time: Some(vm.current_time),
            });
            ui.close_menu();
        }
        ui.separator();
        if ui
            .add_enabled(entry.uses == 0, egui::Button::new("从项目中清除"))
            .clicked()
        {
            intents.push(Intent::RemoveMedia {
                media_id: item.id.clone(),
                cascade: false,
            });
            ui.close_menu();
        }
        if ui
            .add_enabled(
                entry.uses > 0 && !entry.locked_references,
                egui::Button::new("清除并删除时间线引用…"),
            )
            .clicked()
        {
            intents.push(Intent::RemoveMedia {
                media_id: item.id.clone(),
                cascade: true,
            });
            ui.close_menu();
        }
    });
    intents
}

fn drop_on_folder(ui: &Ui, response: &egui::Response, folder_id: Option<&str>) -> Vec<Intent> {
    let media_drag = response.dnd_hover_payload::<MediaDrag>();
    let folder_drag = response.dnd_hover_payload::<FolderDrag>();
    if media_drag.is_none() && folder_drag.is_none() {
        return Vec::new();
    }
    // `Response::hovered()` is deliberately false while another widget is
    // being dragged. `contains_pointer()` is egui's drag-and-drop hit test.
    if response.contains_pointer() {
        let valid = folder_drag
            .as_ref()
            .is_none_or(|folder| folder_id != Some(folder.folder_id.as_str()));
        let accent = if valid { BRAND } else { crate::theme::DANGER };
        let count = media_drag
            .as_ref()
            .map(|m| m.media_ids.len().max(1))
            .unwrap_or(1);
        // The tree is sorted by name, so dropping on a row means moving into
        // that bin rather than silently changing an invisible sibling order.
        // Keep the target visible even when the row is already selected.
        ui.painter().rect_filled(
            response.rect,
            0.0,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 24),
        );
        ui.painter().rect_stroke(
            response.rect.shrink(1.0),
            0.0,
            egui::Stroke::new(1.5_f32, accent),
            egui::StrokeKind::Inside,
        );
        let label = if !valid {
            "不可".to_owned()
        } else if count > 1 {
            format!("放入×{count}")
        } else {
            "放入".into()
        };
        let label_w = if count > 1 { 52.0 } else { 38.0 };
        let label_rect = egui::Rect::from_min_size(
            egui::pos2(response.rect.right() - label_w - 4.0, response.rect.top() + 3.0),
            egui::vec2(label_w, response.rect.height() - 6.0),
        );
        ui.painter().rect_filled(label_rect, 0.0, PANEL_2);
        ui.painter().text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.0),
            accent,
        );
    }
    if let Some(media) = response.dnd_release_payload::<MediaDrag>() {
        let dest = folder_id.map(str::to_owned);
        return media
            .media_ids
            .iter()
            .filter(|id| !id.is_empty())
            .map(|media_id| Intent::LibraryMoveMedia {
                media_id: media_id.clone(),
                folder_id: dest.clone(),
            })
            .collect();
    }
    if let Some(folder) = response.dnd_release_payload::<FolderDrag>() {
        if folder_id != Some(folder.folder_id.as_str()) {
            return vec![Intent::LibraryMoveFolder {
                id: folder.folder_id.clone(),
                parent_id: folder_id.map(str::to_owned),
            }];
        }
    }
    Vec::new()
}

fn drop_preview(
    vm: &EditorViewModel,
    response: &egui::Response,
    folder_id: Option<&str>,
) -> Option<DropPreview> {
    // During an egui drag the target response is not `hovered()`, even though
    // the pointer is inside it. Use the drag-aware hit test so the ghost row
    // appears while the pointer crosses the tree.
    if !response.contains_pointer() {
        return None;
    }
    if let Some(media) = response.dnd_hover_payload::<MediaDrag>() {
        let primary = media.primary();
        let item = vm
            .library
            .items
            .iter()
            .find(|entry| entry.item.id == primary)?;
        let name = if media.media_ids.len() > 1 {
            format!("{} 等 {} 项", item.item.name, media.media_ids.len())
        } else {
            item.item.name.clone()
        };
        return Some(DropPreview::Media {
            name,
            kind: item.item.kind,
        });
    }
    let folder = response.dnd_hover_payload::<FolderDrag>()?;
    if folder_id == Some(folder.folder_id.as_str()) {
        return None;
    }
    Some(DropPreview::Folder {
        name: find_folder_name(&vm.library.explorer, &folder.folder_id)
            .unwrap_or("料箱")
            .to_owned(),
    })
}

fn find_folder_name<'a>(nodes: &'a [ExplorerNode], id: &str) -> Option<&'a str> {
    for node in nodes {
        if let ExplorerNode::Folder {
            id: folder_id,
            name,
            children,
            ..
        } = node
        {
            if folder_id == id {
                return Some(name);
            }
            if let Some(name) = find_folder_name(children, id) {
                return Some(name);
            }
        }
    }
    None
}

fn draw_drop_preview(ui: &mut Ui, preview: DropPreview, depth: usize, guides: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::hover());
    let ghost = Color32::from_rgba_unmultiplied(BRAND.r(), BRAND.g(), BRAND.b(), 18);
    let text = Color32::from_rgba_unmultiplied(TEXT.r(), TEXT.g(), TEXT.b(), 155);
    ui.painter().rect_filled(rect, 0.0, ghost);
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        0.0,
        egui::Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(BRAND.r(), BRAND.g(), BRAND.b(), 130),
        ),
        egui::StrokeKind::Inside,
    );
    paint_guides(ui, rect, guides);
    let x0 = rect.left() + 4.0 + depth as f32 * INDENT;
    match preview {
        DropPreview::Media { name, kind } => {
            icons::media(ui.painter(), icon_rect(rect, x0 + 14.0), kind);
            paint_name(ui, rect, x0 + 34.0, &name, text, rect.right() - 48.0);
        }
        DropPreview::Folder { name } => {
            icons::folder(ui.painter(), icon_rect(rect, x0 + 14.0), false);
            paint_name(ui, rect, x0 + 34.0, &name, text, rect.right() - 48.0);
        }
    }
    ui.painter().text(
        egui::pos2(rect.right() - 7.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "预览",
        egui::FontId::proportional(10.0),
        BRAND,
    );
}

#[cfg(test)]
mod tests;
