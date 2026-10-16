//! Project media UI and its typed drag payload. Source files are never changed here.

use egui::{Color32, Sense, Ui};
use todo_interaction::{EditorFocus, EditorViewModel, Intent};

use crate::format::clock;
use crate::theme::{button, Btn, BORDER, BRAND, DIM, FIELD, MUTED, PANEL, PANEL_2, TEXT};
use crate::thumbs::ThumbCache;

#[derive(Debug, Clone)]
pub(super) struct MediaDrag {
    pub media_id: String,
}

pub(super) fn draw(ui: &mut Ui, vm: &EditorViewModel, thumbs: &mut ThumbCache) -> Vec<Intent> {
    egui::Frame::NONE
        .fill(PANEL)
        .inner_margin(10)
        .show(ui, |ui| draw_contents(ui, vm, thumbs))
        .inner
}

fn draw_contents(ui: &mut Ui, vm: &EditorViewModel, thumbs: &mut ThumbCache) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
    ui.spacing_mut().button_padding = egui::vec2(7.0, 4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("媒体库")
                .strong()
                .size(11.0)
                .color(TEXT),
        );
        ui.label(
            egui::RichText::new(format!("{:02}", vm.library.items.len()))
                .small()
                .color(MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(
                ui,
                if vm.importing {
                    "导入中..."
                } else {
                    "+ 导入"
                },
                Btn::Ghost,
                !vm.importing,
            )
            .on_hover_text("导入视频、音频或图片 (Ctrl+I)")
            .clicked()
            {
                intents.push(Intent::Focus(EditorFocus::Library));
                intents.push(Intent::ImportPressed);
            }
        });
    });
    let mut query = vm.library.query.clone();
    let search = ui.add(
        egui::TextEdit::singleline(&mut query)
            .hint_text("搜索素材...")
            .margin(egui::Margin::symmetric(9, 7))
            .background_color(FIELD)
            .desired_width(f32::INFINITY),
    );
    if search.gained_focus() {
        intents.push(Intent::Focus(EditorFocus::Library));
    }
    if search.changed() {
        intents.push(Intent::LibraryQuery(query));
    }

    let selected = vm
        .library
        .items
        .iter()
        .find(|entry| Some(&entry.item.id) == vm.library.selected_id.as_ref());
    ui.horizontal_wrapped(|ui| {
        if button(ui, "追加", Btn::Ghost, selected.is_some())
            .on_hover_text("将所选素材追加到序列末尾")
            .clicked()
        {
            if let Some(entry) = selected {
                intents.push(Intent::PlaceMedia {
                    media_id: entry.item.id.clone(),
                    track_id: None,
                    time: None,
                });
            }
        }
        if button(ui, "放到播放头", Btn::Ghost, selected.is_some())
            .on_hover_text("放到播放头处的空白区间")
            .clicked()
        {
            if let Some(entry) = selected {
                intents.push(Intent::PlaceMedia {
                    media_id: entry.item.id.clone(),
                    track_id: None,
                    time: Some(vm.current_time),
                });
            }
        }
    });
    ui.separator();
    let list_height = (ui.available_height() - 24.0).max(0.0);
    egui::ScrollArea::vertical()
        .id_salt("project_media_list")
        .auto_shrink([false, false])
        .max_height(list_height)
        .show(ui, |ui| {
            let mut visible = 0;
            for entry in &vm.library.items {
                let item = &entry.item;
                visible += 1;
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 64.0),
                    Sense::click_and_drag(),
                );
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    if entry.selected {
                        Color32::from_rgb(0x2c, 0x3c, 0x46)
                    } else if response.hovered() {
                        PANEL_2
                    } else {
                        PANEL
                    },
                );
                if entry.selected {
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                        0.0,
                        BRAND,
                    );
                }
                ui.painter().line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    egui::Stroke::new(1.0_f32, BORDER),
                );
                let visual = item.visual;
                let (kind_label, glyph, glyph_color) = match item.kind {
                    todo_interaction::MediaKind::Video => ("视频", "V", BRAND),
                    todo_interaction::MediaKind::Audio => ("音频", "A", crate::theme::AUDIO_TEXT),
                    todo_interaction::MediaKind::Image => ("图片", "图", BRAND),
                };
                let thumb = egui::Rect::from_min_size(
                    rect.min + egui::vec2(8.0, 8.0),
                    egui::vec2(72.0, 48.0),
                );
                ui.painter().rect_filled(thumb, 0.0, FIELD);
                let showed = visual
                    .then_some(item.path.as_deref())
                    .flatten()
                    .is_some_and(|path| thumbs.paint(ui, path, thumb.shrink(1.0)));
                if !showed {
                    ui.painter().text(
                        thumb.center(),
                        egui::Align2::CENTER_CENTER,
                        glyph,
                        egui::FontId::monospace(12.0),
                        glyph_color,
                    );
                }
                let text_x = 90.0_f32;
                let mut name = egui::text::LayoutJob::simple(
                    item.name.clone(),
                    egui::FontId::proportional(12.0),
                    TEXT,
                    (rect.width() - text_x - 12.0).max(10.0),
                );
                name.wrap.max_rows = 1;
                name.wrap.break_anywhere = true;
                let name = ui.painter().layout_job(name);
                ui.painter()
                    .galley(rect.min + egui::vec2(text_x, 12.0), name, TEXT);
                let size = match (item.width, item.height) {
                    (Some(w), Some(h)) if visual => format!("{w}×{h}"),
                    _ => kind_label.into(),
                };
                ui.painter().text(
                    rect.min + egui::vec2(text_x, 42.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{size}  ·  引用 {}", entry.uses),
                    egui::FontId::proportional(10.0),
                    MUTED,
                );
                ui.painter().text(
                    rect.right_bottom() + egui::vec2(-8.0, -19.0),
                    egui::Align2::RIGHT_CENTER,
                    clock(item.duration),
                    egui::FontId::monospace(10.0),
                    DIM,
                );
                response.dnd_set_drag_payload(MediaDrag {
                    media_id: item.id.clone(),
                });
                if response.clicked() || response.drag_started() || response.secondary_clicked() {
                    intents.push(Intent::Focus(EditorFocus::Library));
                    intents.push(Intent::LibrarySelect(item.id.clone()));
                }
                if response.double_clicked() {
                    intents.push(Intent::PlaceMedia {
                        media_id: item.id.clone(),
                        track_id: None,
                        time: None,
                    });
                }
                response.clone().on_hover_text(format!(
                    "{}\n时间线引用 {}\n拖入空白轨道，或双击追加。",
                    item.path.as_deref().unwrap_or("源文件不可用"),
                    entry.uses
                ));
                response.context_menu(|ui| {
                    if ui.button("追加到序列").clicked() {
                        intents.push(Intent::PlaceMedia {
                            media_id: item.id.clone(),
                            track_id: None,
                            time: None,
                        });
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(entry.uses == 0, egui::Button::new("移除未使用素材"))
                        .on_hover_text("从媒体库移除条目，保留源文件")
                        .clicked()
                    {
                        intents.push(Intent::RemoveMedia {
                            media_id: item.id.clone(),
                            cascade: false,
                        });
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            entry.uses > 0 && !entry.locked_references,
                            egui::Button::new("移除素材及全部时间线引用..."),
                        )
                        .on_hover_text("移除媒体库条目及所有时间线引用，可撤销；源文件保留。")
                        .clicked()
                    {
                        intents.push(Intent::RemoveMedia {
                            media_id: item.id.clone(),
                            cascade: true,
                        });
                        ui.close_menu();
                    }
                    if entry.locked_references {
                        ui.weak("请先解锁被引用的轨道，再移除引用。");
                    }
                });
            }
            if visible == 0 {
                ui.add_space(16.0);
                ui.weak(if vm.library.items.is_empty() {
                    "导入文件以建立媒体库。"
                } else {
                    "没有匹配的素材。"
                });
            }
        });
    ui.add(
        egui::Label::new(
            egui::RichText::new("拖入时间线 · 双击追加")
                .small()
                .color(MUTED),
        )
        .truncate(),
    )
    .on_hover_text("将素材拖入时间线，或双击追加");
    intents
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::{MediaItem, Project};

    #[test]
    fn media_row_drag_carries_the_identifier() {
        let mut vm = crate::test_view(
            Project {
                version: 1,
                fps: 30.0,
                playhead: 0.0,
                width: 1920,
                height: 1080,
                duration: None,
                tracks: Vec::new(),
                text_styles: Vec::new(),
                media: vec![MediaItem {
                    id: "legacy".into(),
                    name: "legacy.mp4".into(),
                    path: Some("D:/media/legacy.mp4".into()),
                    duration: 8.0,
                    has_audio: Some(false),
                    width: None,
                    height: None,
                    kind: None,
                }],
            },
            "test",
        );
        // The presentation layer owns filtering. Paint each supplied result
        // even when its label does not contain the displayed search query.
        vm.library.query = "provided search results".into();
        let ctx = egui::Context::default();
        let mut pointer = egui::pos2(40.0, 160.0);
        for step in 0..3 {
            let events = match step {
                1 => vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::PointerButton {
                        pos: pointer,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                2 => vec![egui::Event::PointerMoved(pointer + egui::vec2(20.0, 0.0))],
                _ => Vec::new(),
            };
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(350.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        draw(ui, &vm, &mut ThumbCache::default());
                    });
                },
            );
            if step == 0 {
                pointer = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "legacy.mp4" => {
                            Some(text.pos + egui::vec2(5.0, 5.0))
                        }
                        _ => None,
                    })
                    .expect("media row was painted");
            }
        }
        let payload = egui::DragAndDrop::payload::<MediaDrag>(&ctx).expect("media drag payload");
        assert_eq!(payload.media_id, "legacy");
    }
}
