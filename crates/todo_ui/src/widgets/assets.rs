//! The left column's other tabs: the motion library (every preset playing a
//! short loop, one click applies it to the selected clip) and the font
//! library (every font file and text style, rendered by the real text
//! renderer). The app fills the view and reloads the libraries on `Refresh`.
use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Sense, Stroke, Ui};
use todo_interaction::{
    AssetsView, FontCard, Intent, MotionCard, MotionKind, MotionOrigin, MotionPose, StyleCard,
};

use crate::theme::{button, Btn, BORDER, BORDER_STRONG, BRAND, DIM, FIELD, MUTED, PANEL, PANEL_2, TEXT, WARN};
use crate::thumbs::ThumbCache;

/// What the left column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftTab {
    #[default]
    Media,
    Motions,
    Fonts,
}

/// Panel requests the app handles (disk, the user's library).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AssetsAction {
    #[default]
    None,
    /// Read the motion and font libraries from disk again.
    Refresh,
    /// Show this folder in the file manager.
    OpenFolder(String),
    /// Copy a preset into the user's library to adjust it.
    SaveAsMine(String),
}

const TAB_ID: &str = "todo_ui_left_tab";
const DOCK_ID: &str = "todo_ui_assets_dock";
const FILTER_ID: &str = "todo_ui_motion_filter";
pub(super) const HEADER_H: f32 = 30.0;

pub fn left_tab(ctx: &egui::Context) -> LeftTab {
    ctx.data(|d| d.get_temp::<LeftTab>(Id::new(TAB_ID)))
        .unwrap_or_default()
}

pub fn set_left_tab(ctx: &egui::Context, tab: LeftTab) {
    ctx.data_mut(|d| d.insert_temp(Id::new(TAB_ID), tab));
}

/// Where the layout reserved the panel this frame, and which one.
pub(super) fn set_dock(ctx: &egui::Context, dock: Option<(Rect, LeftTab)>) {
    ctx.data_mut(|d| d.insert_temp(Id::new(DOCK_ID), dock));
}

pub fn dock(ctx: &egui::Context) -> Option<(Rect, LeftTab)> {
    ctx.data(|d| d.get_temp::<Option<(Rect, LeftTab)>>(Id::new(DOCK_ID)))
        .flatten()
}

/// Tabs over the left column.
pub(super) fn header(ui: &mut Ui, rect: Rect) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, PANEL);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0_f32, BORDER),
    );
    let tab = left_tab(ui.ctx());
    let mut x = rect.left() + 4.0;
    for (label, target) in [
        ("媒体", LeftTab::Media),
        ("动效", LeftTab::Motions),
        ("字体", LeftTab::Fonts),
    ] {
        let width = 52.0;
        let tab_rect = Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, rect.height() - 1.0));
        let response = ui.interact(tab_rect, Id::new(("todo_ui_left_tab", label)), Sense::click());
        let active = tab == target;
        let color = if active {
            TEXT
        } else if response.hovered() {
            TEXT.gamma_multiply(0.85)
        } else {
            MUTED
        };
        painter.text(tab_rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.5), color);
        if active {
            painter.line_segment(
                [
                    egui::pos2(tab_rect.left() + 10.0, rect.bottom() - 1.0),
                    egui::pos2(tab_rect.right() - 10.0, rect.bottom() - 1.0),
                ],
                Stroke::new(2.0_f32, BRAND),
            );
        }
        if response.clicked() && !active {
            set_left_tab(ui.ctx(), target);
        }
        x += width;
    }
}

/// Draw the motion or font panel into `ui`.
pub fn draw_dock(
    ui: &mut Ui,
    view: &AssetsView,
    tab: LeftTab,
    thumbs: &mut ThumbCache,
) -> (Vec<Intent>, AssetsAction) {
    let mut intents = Vec::new();
    let mut action = AssetsAction::None;
    egui::Frame::NONE
        .fill(PANEL)
        .inner_margin(egui::Margin::symmetric(6, 6))
        .show(ui, |ui| match tab {
            LeftTab::Motions => motions(ui, view, &mut intents, &mut action),
            LeftTab::Fonts => fonts(ui, view, thumbs, &mut action),
            LeftTab::Media => {}
        });
    (intents, action)
}

fn toolbar(ui: &mut Ui, title: String, folder: &str, folder_hint: &str, action: &mut AssetsAction) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(11.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, "刷新", Btn::Ghost, true)
                .on_hover_text("重新读取动效和字体文件夹")
                .clicked()
            {
                *action = AssetsAction::Refresh;
            }
            if button(ui, "文件夹", Btn::Ghost, !folder.is_empty())
                .on_hover_text(format!("{folder_hint}\n{folder}"))
                .clicked()
            {
                *action = AssetsAction::OpenFolder(folder.to_owned());
            }
        });
    });
}

// ------------------------------------------------------------ motions

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    Kind(MotionKind),
}

fn kind_label(kind: MotionKind) -> &'static str {
    match kind {
        MotionKind::Intro => "入场",
        MotionKind::Outro => "出场",
        MotionKind::Transition => "转场",
    }
}

fn motions(ui: &mut Ui, view: &AssetsView, intents: &mut Vec<Intent>, action: &mut AssetsAction) {
    toolbar(
        ui,
        format!("动效库 · {}", view.motions.len()),
        &view.motions_dir,
        "我的动效文件夹：放进去的 JSON 预设点「刷新」后出现",
        action,
    );
    let filter_id = Id::new(FILTER_ID);
    let mut filter = ui.ctx().data(|d| d.get_temp::<Filter>(filter_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        for (label, value) in [
            ("全部", Filter::All),
            ("入场", Filter::Kind(MotionKind::Intro)),
            ("出场", Filter::Kind(MotionKind::Outro)),
            ("转场", Filter::Kind(MotionKind::Transition)),
        ] {
            if button(ui, label, Btn::Tool { active: filter == value }, true).clicked() {
                filter = value;
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(filter_id, filter));
    ui.add_space(4.0);
    match &view.target {
        Some(target) => ui.label(
            egui::RichText::new(format!("点击动效应用到「{}」", target.name))
                .size(11.0)
                .color(BRAND),
        ),
        None => ui.label(
            egui::RichText::new("在时间线上选中画面或文字片段，点击动效即可应用")
                .size(11.0)
                .color(DIM),
        ),
    };
    ui.add_space(6.0);
    let cards: Vec<&MotionCard> = view
        .motions
        .iter()
        .filter(|card| filter == Filter::All || filter == Filter::Kind(card.kind))
        .collect();
    let time = ui.input(|i| i.time) as f32;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            let columns = ((width + 6.0) / 136.0).floor().max(1.0) as usize;
            let card_w = (width - 6.0 * (columns as f32 - 1.0)) / columns as f32;
            let mut last_kind = None;
            let mut row: Vec<&MotionCard> = Vec::new();
            let mut flush = |ui: &mut Ui, row: &mut Vec<&MotionCard>| {
                if row.is_empty() {
                    return;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for card in row.drain(..) {
                        motion_card(ui, card, view, card_w, time, intents, action);
                    }
                });
                ui.add_space(6.0);
            };
            for card in cards {
                if last_kind != Some(card.kind) {
                    flush(ui, &mut row);
                    ui.label(egui::RichText::new(kind_label(card.kind)).size(11.0).color(MUTED));
                    ui.add_space(3.0);
                    last_kind = Some(card.kind);
                }
                row.push(card);
                if row.len() == columns {
                    flush(ui, &mut row);
                }
            }
            flush(ui, &mut row);
            for warning in &view.warnings {
                ui.label(egui::RichText::new(warning).size(10.5).color(WARN));
            }
        });
    // The previews play while the panel shows.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

fn motion_card(
    ui: &mut Ui,
    card: &MotionCard,
    view: &AssetsView,
    width: f32,
    time: f32,
    intents: &mut Vec<Intent>,
    action: &mut AssetsAction,
) {
    let stage_h = (width * 9.0 / 16.0).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, stage_h + 40.0), Sense::click());
    let applicable = view.target.as_ref().and_then(|t| card.apply(t));
    let painter = ui.painter_at(rect);
    let stroke = if card.in_use {
        BRAND
    } else if response.hovered() {
        BORDER_STRONG
    } else {
        BORDER
    };
    painter.rect(rect, CornerRadius::same(6), PANEL_2, Stroke::new(1.0_f32, stroke), egui::StrokeKind::Inside);
    let stage = Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(width - 8.0, stage_h - 4.0));
    painter.rect_filled(stage, CornerRadius::same(4), FIELD);
    paint_motion(&ui.painter_at(stage), stage, card, time);
    let name: String = card.name.chars().take(10).collect();
    painter.text(
        egui::pos2(rect.left() + 7.0, stage.bottom() + 6.0),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(12.0),
        TEXT,
    );
    let meta = format!("{:.2}s · {}", card.duration, card.origin.label());
    painter.text(
        egui::pos2(rect.left() + 7.0, stage.bottom() + 22.0),
        Align2::LEFT_TOP,
        meta,
        FontId::proportional(10.0),
        if card.origin == MotionOrigin::Builtin { DIM } else { MUTED },
    );
    if card.in_use {
        painter.text(
            stage.right_top() + egui::vec2(-4.0, 3.0),
            Align2::RIGHT_TOP,
            "使用中",
            FontId::proportional(10.0),
            BRAND,
        );
    }
    let hint = match (&view.target, &applicable) {
        (Some(target), Some(_)) => format!("点击应用到「{}」", target.name),
        (Some(_), None) => "转场需要同一轨道上紧挨着的前一个片段".to_owned(),
        (None, _) => "先在时间线上选中一个片段".to_owned(),
    };
    let mut tip = format!("{}  `{}`", card.name, card.id);
    if !card.description.is_empty() {
        tip.push_str(&format!("\n{}", card.description));
    }
    if !card.tags.is_empty() {
        tip.push_str(&format!("\n标签：{}", card.tags.join("、")));
    }
    tip.push_str(&format!("\n{hint}"));
    let response = response.on_hover_text(tip);
    if response.clicked() {
        if let Some(intent) = applicable {
            intents.push(intent);
        }
    }
    response.context_menu(|ui| {
        if ui.button("复制 id").clicked() {
            ui.ctx().copy_text(card.id.clone());
            ui.close_menu();
        }
        if card.origin != MotionOrigin::User && ui.button("复制到我的动效库（可修改）").clicked() {
            *action = AssetsAction::SaveAsMine(card.id.clone());
            ui.close_menu();
        }
    });
}

fn pose_at(poses: &[MotionPose], u: f32) -> MotionPose {
    let rest = MotionPose {
        opacity: 1.0,
        dx: 0.0,
        dy: 0.0,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    if poses.is_empty() {
        return rest;
    }
    let f = u.clamp(0.0, 1.0) * (poses.len() - 1) as f32;
    let i = (f.floor() as usize).min(poses.len() - 1);
    let j = (i + 1).min(poses.len() - 1);
    let t = f - i as f32;
    let (a, b) = (poses[i], poses[j]);
    let mix = |x: f32, y: f32| x + (y - x) * t;
    MotionPose {
        opacity: mix(a.opacity, b.opacity),
        dx: mix(a.dx, b.dx),
        dy: mix(a.dy, b.dy),
        scale_x: mix(a.scale_x, b.scale_x),
        scale_y: mix(a.scale_y, b.scale_y),
    }
}

/// One loop: a beat before, the motion, a beat at rest.
fn paint_motion(painter: &egui::Painter, stage: Rect, card: &MotionCard, time: f32) {
    let d = (card.duration as f32).clamp(0.15, 3.0);
    let hold = 0.6;
    let t = time % (d + hold * 2.0);
    let u = ((t - hold) / d).clamp(0.0, 1.0);
    let incoming = egui::Color32::from_rgb(0x2a, 0x67, 0x98);
    let outgoing = egui::Color32::from_rgb(0x5c, 0x62, 0x6d);
    match (&card.outgoing, card.kind) {
        (Some(out), MotionKind::Transition) => {
            let before = t < hold;
            let after = t > hold + d;
            let draw_out = |p: &egui::Painter| {
                if !after {
                    paint_box(p, stage, pose_at(out, if before { 0.0 } else { u }), outgoing, "A");
                }
            };
            let draw_in = |p: &egui::Painter| {
                if !before {
                    paint_box(p, stage, pose_at(&card.poses, u), incoming, "B");
                }
            };
            if card.outgoing_on_top {
                draw_in(painter);
                draw_out(painter);
            } else {
                draw_out(painter);
                draw_in(painter);
            }
        }
        _ => paint_box(painter, stage, pose_at(&card.poses, u), incoming, "A"),
    }
}

fn paint_box(painter: &egui::Painter, stage: Rect, pose: MotionPose, color: Color32, label: &str) {
    let alpha = pose.opacity.clamp(0.0, 1.0);
    if alpha <= 0.01 {
        return;
    }
    let base = egui::vec2(stage.width() * 0.46, stage.height() * 0.5);
    let size = egui::vec2(base.x * pose.scale_x.max(0.0), base.y * pose.scale_y.max(0.0));
    let center = stage.center() + egui::vec2(pose.dx * stage.width(), pose.dy * stage.height());
    let rect = Rect::from_center_size(center, size);
    painter.rect_filled(rect, CornerRadius::same(3), color.gamma_multiply(alpha));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        label,
        FontId::proportional((size.y * 0.5).clamp(6.0, 22.0)),
        Color32::WHITE.gamma_multiply(alpha),
    );
}

// ------------------------------------------------------------ fonts

fn fonts(ui: &mut Ui, view: &AssetsView, thumbs: &mut ThumbCache, action: &mut AssetsAction) {
    toolbar(
        ui,
        format!("字体库 · {} 个字体 · {} 个花字", view.fonts.len(), view.styles.len()),
        &view.fonts_dir,
        "字体文件夹：放进 .ttf / .otf 后点「刷新」；来源和授权登记在 fonts.json",
        action,
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("字体文件").size(11.0).color(MUTED));
            ui.add_space(3.0);
            for font in &view.fonts {
                font_card(ui, font, thumbs);
                ui.add_space(6.0);
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new("花字样式").size(11.0).color(MUTED));
            ui.add_space(3.0);
            let width = ui.available_width();
            let columns = ((width + 6.0) / 150.0).floor().max(1.0) as usize;
            let card_w = (width - 6.0 * (columns as f32 - 1.0)) / columns as f32;
            for row in view.styles.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for style in row {
                        style_card(ui, style, card_w, thumbs);
                    }
                });
                ui.add_space(6.0);
            }
            for warning in &view.warnings {
                ui.label(egui::RichText::new(warning).size(10.5).color(WARN));
            }
        });
}

fn preview(ui: &Ui, thumbs: &mut ThumbCache, key: &str, rect: Rect) {
    ui.painter().rect_filled(rect, CornerRadius::same(4), FIELD);
    if !thumbs.paint(ui, key, rect.shrink(2.0)) {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            if thumbs.contains(key) { "无法渲染" } else { "渲染中…" },
            FontId::proportional(11.0),
            DIM,
        );
    }
}

fn font_card(ui: &mut Ui, font: &FontCard, thumbs: &mut ThumbCache) {
    let width = ui.available_width();
    let preview_h = (width * 150.0 / 720.0).clamp(36.0, 80.0);
    let lines = 3.0 + f32::from(!font.styles.is_empty());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, preview_h + 10.0 + lines * 15.0), Sense::hover());
    ui.painter()
        .rect(rect, CornerRadius::same(6), PANEL_2, Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Inside);
    let stage = Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(width - 8.0, preview_h));
    preview(ui, thumbs, &font.preview, stage);
    let painter = ui.painter_at(rect);
    let mut y = stage.bottom() + 5.0;
    let mut line = |text: String, size: f32, color: Color32| {
        painter.text(egui::pos2(rect.left() + 7.0, y), Align2::LEFT_TOP, text, FontId::proportional(size), color);
        y += 15.0;
    };
    line(font.name.clone(), 12.5, TEXT);
    line(
        match &font.family {
            Some(family) if family != &font.name => format!("{} · {family}", font.file),
            _ => font.file.clone(),
        },
        10.5,
        MUTED,
    );
    if font.license.is_empty() {
        line(format!("{} · 授权未登记，商用前请确认", font.source), 10.5, WARN);
    } else {
        line(format!("{} · {}", font.source, font.license), 10.5, DIM);
    }
    if !font.styles.is_empty() {
        line(format!("用于：{}", font.styles.join("、")), 10.5, DIM);
    }
    if !font.note.is_empty() {
        response.on_hover_text(&font.note);
    }
}

fn style_card(ui: &mut Ui, style: &StyleCard, width: f32, thumbs: &mut ThumbCache) {
    let preview_h = (width * 0.42).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, preview_h + 40.0), Sense::hover());
    ui.painter()
        .rect(rect, CornerRadius::same(6), PANEL_2, Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Inside);
    let stage = Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(width - 8.0, preview_h));
    preview(ui, thumbs, &style.preview, stage);
    let painter = ui.painter_at(rect);
    let name: String = style.name.chars().take(11).collect();
    painter.text(
        egui::pos2(rect.left() + 7.0, stage.bottom() + 5.0),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(12.0),
        TEXT,
    );
    let (meta, color) = if style.font_missing {
        ("字体缺失，用回退字体".to_owned(), WARN)
    } else {
        (style.font_file.clone(), DIM)
    };
    painter.text(
        egui::pos2(rect.left() + 7.0, stage.bottom() + 21.0),
        Align2::LEFT_TOP,
        meta,
        FontId::proportional(10.0),
        color,
    );
    response.on_hover_text(format!(
        "{}  `{}`\n字体：{}{}",
        style.name,
        style.id,
        style.font_file,
        if style.in_project { "\n工程里的样式" } else { "\n内置样式" }
    ));
}
