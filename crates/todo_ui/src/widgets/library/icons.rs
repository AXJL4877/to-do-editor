//! Compact explorer icons painted as vectors, independent of installed fonts.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};
use todo_interaction::MediaKind;

pub(super) fn chevron(painter: &Painter, rect: Rect, expanded: bool, color: Color32) {
    let center = rect.center();
    let points = if expanded {
        vec![
            center + egui::vec2(-3.0, -1.5),
            center + egui::vec2(0.0, 1.5),
            center + egui::vec2(3.0, -1.5),
        ]
    } else {
        vec![
            center + egui::vec2(-1.5, -3.0),
            center + egui::vec2(1.5, 0.0),
            center + egui::vec2(-1.5, 3.0),
        ]
    };
    painter.add(Shape::line(points, Stroke::new(1.4_f32, color)));
}

pub(super) fn folder(painter: &Painter, rect: Rect, expanded: bool) {
    let color = Color32::from_rgb(0xd6, 0xb4, 0x78);
    let point = |x, y| point(rect, x, y);
    painter.add(Shape::closed_line(
        vec![
            point(1.0, 4.0),
            point(1.0, 2.5),
            point(6.0, 2.5),
            point(7.5, 4.0),
            point(14.5, 4.0),
            point(14.5, 13.0),
            point(1.0, 13.0),
        ],
        Stroke::new(1.15_f32, color),
    ));
    if expanded {
        painter.add(Shape::convex_polygon(
            vec![
                point(3.0, 6.5),
                point(15.0, 6.5),
                point(13.0, 13.0),
                point(1.0, 13.0),
            ],
            Color32::from_rgb(0x60, 0x50, 0x36),
            Stroke::new(1.15_f32, color),
        ));
    } else {
        painter.line_segment(
            [point(1.0, 5.5), point(14.5, 5.5)],
            Stroke::new(1.0_f32, color),
        );
    }
}

pub(super) fn media(painter: &Painter, rect: Rect, kind: MediaKind) {
    let point = |x, y| point(rect, x, y);
    match kind {
        MediaKind::Video => {
            let color = Color32::from_rgb(0x7f, 0xb7, 0xdb);
            painter.rect_stroke(
                Rect::from_min_max(point(1.0, 2.5), point(15.0, 13.5)),
                1.0,
                Stroke::new(1.1_f32, color),
                StrokeKind::Inside,
            );
            for x in [2.5, 12.0] {
                for y in [4.0, 7.0, 10.0] {
                    painter.rect_filled(
                        Rect::from_min_max(point(x, y), point(x + 1.5, y + 1.5)),
                        0.0,
                        color,
                    );
                }
            }
            painter.add(Shape::convex_polygon(
                vec![point(6.0, 5.0), point(6.0, 11.0), point(10.5, 8.0)],
                color,
                Stroke::NONE,
            ));
        }
        MediaKind::Audio => {
            let color = Color32::from_rgb(0x80, 0xc6, 0xaf);
            let stroke = Stroke::new(1.4_f32, color);
            painter.add(Shape::line(
                vec![
                    point(5.5, 11.5),
                    point(5.5, 3.5),
                    point(13.0, 2.0),
                    point(13.0, 10.0),
                ],
                stroke,
            ));
            painter.line_segment([point(5.5, 6.0), point(13.0, 4.5)], stroke);
            painter.circle_filled(point(3.5, 12.0), 2.2, color);
            painter.circle_filled(point(11.0, 10.5), 2.2, color);
        }
        MediaKind::Image => {
            let color = Color32::from_rgb(0xbb, 0xa2, 0xdc);
            painter.rect_stroke(
                Rect::from_min_max(point(1.0, 2.5), point(15.0, 13.5)),
                1.0,
                Stroke::new(1.1_f32, color),
                StrokeKind::Inside,
            );
            painter.circle_filled(point(5.0, 6.0), 1.4, color);
            painter.add(Shape::line(
                vec![
                    point(2.5, 12.0),
                    point(7.5, 7.5),
                    point(10.0, 10.0),
                    point(12.0, 8.5),
                    point(14.0, 11.0),
                ],
                Stroke::new(1.2_f32, color),
            ));
        }
    }
}

fn point(rect: Rect, x: f32, y: f32) -> Pos2 {
    rect.min + egui::vec2(x, y) * (rect.width().min(rect.height()) / 16.0)
}
