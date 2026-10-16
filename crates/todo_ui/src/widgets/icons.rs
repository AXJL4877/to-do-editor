//! Small vector icons for controls that should not be labeled with text.

use egui::epaint::{CubicBezierShape, EllipseShape, QuadraticBezierShape};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Select,
    Plus,
    Play,
    Pause,
    Eye,
    EyeOff,
    Speaker,
    SpeakerOff,
    Lock,
    Unlock,
    /// Blade cue for the razor tool.
    Razor,
}

pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.35_f32, color);
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;
    match icon {
        Icon::Select => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(-0.48 * s, -0.64 * s),
                    c + Vec2::new(-0.34 * s, 0.44 * s),
                    c + Vec2::new(0.35 * s, -0.06 * s),
                ],
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [
                    c + Vec2::new(-0.08 * s, 0.02 * s),
                    c + Vec2::new(0.30 * s, 0.62 * s),
                ],
                Stroke::new(2.2_f32, color),
            );
        }
        Icon::Plus => {
            painter.line_segment(
                [c - Vec2::new(0.5 * s, 0.0), c + Vec2::new(0.5 * s, 0.0)],
                stroke,
            );
            painter.line_segment(
                [c - Vec2::new(0.0, 0.5 * s), c + Vec2::new(0.0, 0.5 * s)],
                stroke,
            );
        }
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(-0.28 * s, -0.48 * s),
                    c + Vec2::new(-0.28 * s, 0.48 * s),
                    c + Vec2::new(0.48 * s, 0.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            let bar = Vec2::new(0.28 * s, 0.92 * s);
            painter.rect_filled(
                Rect::from_center_size(c - Vec2::new(0.24 * s, 0.0), bar),
                0.0,
                color,
            );
            painter.rect_filled(
                Rect::from_center_size(c + Vec2::new(0.24 * s, 0.0), bar),
                0.0,
                color,
            );
        }
        Icon::Eye | Icon::EyeOff => {
            painter.add(EllipseShape::stroke(
                c,
                Vec2::new(0.58 * s, 0.34 * s),
                stroke,
            ));
            painter.circle_filled(c, 0.14 * s, color);
            if matches!(icon, Icon::EyeOff) {
                painter.line_segment(
                    [
                        c + Vec2::new(-0.58 * s, 0.42 * s),
                        c + Vec2::new(0.58 * s, -0.42 * s),
                    ],
                    stroke,
                );
            }
        }
        Icon::Speaker | Icon::SpeakerOff => {
            let body = vec![
                c + Vec2::new(-0.48 * s, -0.20 * s),
                c + Vec2::new(-0.16 * s, -0.20 * s),
                c + Vec2::new(0.18 * s, -0.46 * s),
                c + Vec2::new(0.18 * s, 0.46 * s),
                c + Vec2::new(-0.16 * s, 0.20 * s),
                c + Vec2::new(-0.48 * s, 0.20 * s),
            ];
            painter.add(egui::Shape::closed_line(body, stroke));
            if matches!(icon, Icon::Speaker) {
                painter.add(QuadraticBezierShape::from_points_stroke(
                    [
                        c + Vec2::new(0.28 * s, -0.28 * s),
                        c + Vec2::new(0.52 * s, 0.0),
                        c + Vec2::new(0.28 * s, 0.28 * s),
                    ],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
                painter.add(QuadraticBezierShape::from_points_stroke(
                    [
                        c + Vec2::new(0.38 * s, -0.46 * s),
                        c + Vec2::new(0.72 * s, 0.0),
                        c + Vec2::new(0.38 * s, 0.46 * s),
                    ],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            } else {
                painter.line_segment(
                    [
                        c + Vec2::new(0.30 * s, -0.36 * s),
                        c + Vec2::new(0.60 * s, 0.36 * s),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        c + Vec2::new(0.60 * s, -0.36 * s),
                        c + Vec2::new(0.30 * s, 0.36 * s),
                    ],
                    stroke,
                );
            }
        }
        Icon::Lock | Icon::Unlock => {
            let body =
                Rect::from_center_size(c + Vec2::new(0.0, 0.16 * s), Vec2::new(0.74 * s, 0.54 * s));
            painter.rect_stroke(body, 1.0, stroke, egui::StrokeKind::Outside);
            painter.circle_filled(body.center() + Vec2::new(0.0, 0.05 * s), 0.07 * s, color);
            let left = Pos2::new(body.left() + 0.14 * s, body.top());
            let right = if matches!(icon, Icon::Lock) {
                Pos2::new(body.right() - 0.14 * s, body.top())
            } else {
                Pos2::new(body.right() + 0.06 * s, body.top() - 0.06 * s)
            };
            let lift = if matches!(icon, Icon::Lock) {
                0.42 * s
            } else {
                0.48 * s
            };
            painter.add(CubicBezierShape::from_points_stroke(
                [
                    left,
                    Pos2::new(left.x, left.y - lift),
                    Pos2::new(right.x, left.y - lift),
                    right,
                ],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
        }
        Icon::Razor => {
            painter.add(egui::Shape::closed_line(
                vec![
                    c + Vec2::new(-0.62 * s, -0.18 * s),
                    c + Vec2::new(0.18 * s, -0.62 * s),
                    c + Vec2::new(0.62 * s, 0.18 * s),
                    c + Vec2::new(-0.18 * s, 0.62 * s),
                ],
                stroke,
            ));
            painter.line_segment(
                [
                    c + Vec2::new(-0.26 * s, 0.14 * s),
                    c + Vec2::new(0.26 * s, -0.14 * s),
                ],
                stroke,
            );
            painter.circle_filled(c + Vec2::new(-0.26 * s, -0.10 * s), 0.07 * s, color);
            painter.circle_filled(c + Vec2::new(0.26 * s, 0.10 * s), 0.07 * s, color);
        }
    }
}

pub fn tool_button(ui: &mut Ui, icon: Icon, active: bool, tooltip: &str) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(32.0, 28.0), Sense::hover());
    chip(ui, ui.id().with(tooltip), rect, icon, active, tooltip)
}

/// Square icon control used in track headers.
pub fn chip(
    ui: &mut Ui,
    id: egui::Id,
    rect: Rect,
    icon: Icon,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click()).on_hover_text(tooltip);
    ui.painter().rect_filled(
        rect,
        0.0,
        if active {
            Color32::from_rgb(0x70, 0x4e, 0x24)
        } else if response.hovered() {
            Color32::from_gray(48)
        } else {
            Color32::from_gray(28)
        },
    );
    let color = if active {
        Color32::from_rgb(255, 224, 172)
    } else {
        crate::theme::MUTED
    };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(rect.height() * 0.72));
    paint(ui.painter(), icon_rect, icon, color);
    response
}

/// Primary play / pause control below the program monitor.
pub fn play_toggle(ui: &mut Ui, playing: bool, enabled: bool) -> egui::Response {
    let size = Vec2::new(40.0, 32.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let fill = if enabled {
        crate::theme::BRAND
    } else {
        Color32::from_rgb(0x3a, 0x4a, 0x55)
    };
    let ink = if enabled {
        Color32::from_rgb(0x08, 0x17, 0x25)
    } else {
        crate::theme::MUTED
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
        if playing { Icon::Pause } else { Icon::Play },
        ink,
    );
    response.on_hover_text(if playing { "暂停" } else { "播放" })
}
