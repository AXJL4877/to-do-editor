//! "查看和修改内容" inside a production card: each segment's script and the
//! pictures made for it. Changes go out as production edits; the pipeline
//! redoes only what they touch.
use super::{status_color, ProductionAction};
use crate::theme::{button, Btn, BORDER, BRAND, DIM, FIELD, MUTED, TEXT};
use crate::ThumbCache;
use egui::{Id, RichText, Ui};
use todo_interaction::{
    ElementPatch, ElementView, ProductionEdit, ProductionNodeStatus, ProductionView, SegmentView,
};

const THUMB: f32 = 72.0;

/// Text being edited, kept in egui memory until saved.
fn draft(ui: &Ui, key: (&str, &str, &str), original: &str) -> String {
    ui.ctx()
        .data(|d| d.get_temp::<String>(Id::new(key)))
        .unwrap_or_else(|| original.to_owned())
}

fn keep_draft(ui: &Ui, key: (&str, &str, &str), text: &str) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(Id::new(key), text.to_owned()));
}

fn forget_draft(ui: &Ui, key: (&str, &str, &str)) {
    ui.ctx().data_mut(|d| d.remove::<String>(Id::new(key)));
}

/// Paint a picture (or a placeholder) into a square cell.
fn thumb(
    ui: &mut Ui,
    rect: egui::Rect,
    file: Option<&str>,
    thumbs: &mut ThumbCache,
    action: &mut ProductionAction,
) {
    let inner = rect.shrink(2.0);
    match file {
        Some(path) => {
            if !thumbs.contains(path) && *action == ProductionAction::None {
                *action = ProductionAction::Thumbnail {
                    path: path.to_owned(),
                };
            }
            if !thumbs.paint(ui, path, inner) {
                ui.painter().rect_filled(inner, 0.0, FIELD);
                ui.painter().text(
                    inner.center(),
                    egui::Align2::CENTER_CENTER,
                    "载入中",
                    egui::FontId::proportional(10.5),
                    DIM,
                );
            }
        }
        None => {
            ui.painter().rect_filled(inner, 0.0, FIELD);
            ui.painter().text(
                inner.center(),
                egui::Align2::CENTER_CENTER,
                "生成中",
                egui::FontId::proportional(10.5),
                DIM,
            );
        }
    }
}

pub(super) fn draw(
    ui: &mut Ui,
    production: &ProductionView,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    egui::CollapsingHeader::new(RichText::new("查看和修改内容").size(12.5).color(MUTED))
        .id_salt(("production-content", &production.id))
        .default_open(false)
        .show(ui, |ui| {
            for segment in &production.segments {
                segment_editor(ui, production, segment, thumbs, &mut action);
            }
        });
    action
}

fn segment_editor(
    ui: &mut Ui,
    production: &ProductionView,
    segment: &SegmentView,
    thumbs: &mut ThumbCache,
    action: &mut ProductionAction,
) {
    let header = RichText::new(&segment.label)
        .size(12.5)
        .color(status_color(segment.status));
    egui::CollapsingHeader::new(header)
        .id_salt(("segment", &production.id, &segment.id))
        .default_open(false)
        .show(ui, |ui| {
            let key = ("segment-text", production.id.as_str(), segment.id.as_str());
            let mut text = draft(ui, key, &segment.text);
            crate::theme::with_input_radius(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
            });
            let changed = text != segment.text;
            keep_draft(ui, key, &text);
            ui.horizontal(|ui| {
                if button(ui, "保存文案", Btn::Normal, changed)
                    .clicked()
                {
                    *action = ProductionAction::Edit {
                        production: production.id.clone(),
                        edit: ProductionEdit::SetSegmentText {
                            segment: segment.id.clone(),
                            text: text.clone(),
                        },
                    };
                    forget_draft(ui, key);
                }
                if changed && button(ui, "撤销修改", Btn::Ghost, true).clicked() {
                    forget_draft(ui, key);
                }
            });
            if segment.elements.is_empty() {
                return;
            }
            ui.add_space(4.0);
            ui.label(RichText::new("画面").size(11.5).color(DIM));
            for element in &segment.elements {
                element_row(ui, production, element, thumbs, action);
            }
        });
}

fn element_row(
    ui: &mut Ui,
    production: &ProductionView,
    element: &ElementView,
    thumbs: &mut ThumbCache,
    action: &mut ProductionAction,
) {
    let open_id = Id::new(("element-open", &production.id, &element.id));
    let mut open = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(open_id))
        .unwrap_or(false);
    let picked = element
        .variants
        .iter()
        .find(|v| Some(&v.id) == element.picked.as_ref());
    let row = ui
        .horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
            thumb(
                ui,
                rect,
                picked.and_then(|v| v.file.as_deref()),
                thumbs,
                action,
            );
            ui.vertical(|ui| {
                ui.label(RichText::new(&element.label).size(12.5).color(TEXT));
                let state = match element.status {
                    ProductionNodeStatus::Complete => "已完成",
                    ProductionNodeStatus::Failed => "没有完成",
                    ProductionNodeStatus::Waiting => "等待网络",
                    ProductionNodeStatus::Running => "生成中",
                    _ => "排队中",
                };
                ui.label(
                    RichText::new(state)
                        .size(11.0)
                        .color(status_color(element.status)),
                );
            });
        })
        .response;
    let row = ui.interact(row.rect, open_id.with("row"), egui::Sense::click());
    if row.hovered() {
        ui.painter().rect_stroke(
            row.rect.expand(1.0),
            0.0,
            egui::Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Outside,
        );
    }
    if row.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
    }
    if !open {
        return;
    }
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("画面描述").size(11.5).color(MUTED));
            let key = ("element-draw", production.id.as_str(), element.id.as_str());
            let mut draw = draft(ui, key, &element.draw);
            crate::theme::with_input_radius(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut draw)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
            });
            let changed = draw != element.draw;
            keep_draft(ui, key, &draw);
            ui.horizontal_wrapped(|ui| {
                if button(ui, "按新描述重画", Btn::Normal, changed).clicked() {
                    *action = ProductionAction::Edit {
                        production: production.id.clone(),
                        edit: ProductionEdit::SetElement {
                            element: element.id.clone(),
                            patch: ElementPatch {
                                draw: Some(draw.clone()),
                                ..Default::default()
                            },
                        },
                    };
                    forget_draft(ui, key);
                }
                if button(ui, "再来一张", Btn::Normal, true)
                    .clicked()
                {
                    *action = ProductionAction::Edit {
                        production: production.id.clone(),
                        edit: ProductionEdit::AddVariant {
                            element: element.id.clone(),
                        },
                    };
                }
                if button(ui, "用自己的图片", Btn::Normal, true).clicked() {
                    *action = ProductionAction::Upload {
                        production: production.id.clone(),
                        element: element.id.clone(),
                    };
                }
            });
            if element.variants.len() > 1 {
                ui.add_space(4.0);
            }
            ui.horizontal_wrapped(|ui| {
                for variant in &element.variants {
                    let chosen = Some(&variant.id) == element.picked.as_ref();
                    let (rect, response) = ui
                        .allocate_exact_size(egui::vec2(THUMB, THUMB + 16.0), egui::Sense::click());
                    let image = egui::Rect::from_min_size(rect.min, egui::vec2(THUMB, THUMB));
                    thumb(ui, image, variant.file.as_deref(), thumbs, action);
                    if chosen {
                        ui.painter().rect_stroke(
                            image,
                            0.0,
                            egui::Stroke::new(2.0_f32, BRAND),
                            egui::StrokeKind::Inside,
                        );
                    }
                    ui.painter().text(
                        egui::pos2(rect.center().x, rect.bottom() - 7.0),
                        egui::Align2::CENTER_CENTER,
                        if chosen {
                            "使用中".to_owned()
                        } else {
                            variant.label.clone()
                        },
                        egui::FontId::proportional(10.5),
                        if chosen { BRAND } else { MUTED },
                    );
                    if response.clicked() && !chosen && variant.file.is_some() {
                        *action = ProductionAction::Edit {
                            production: production.id.clone(),
                            edit: ProductionEdit::PickVariant {
                                element: element.id.clone(),
                                variant: variant.id.clone(),
                            },
                        };
                    }
                }
            });
        });
}
