use egui::{Align, Layout, RichText, Sense, Ui, Vec2};

use super::chrome;
use crate::theme::{
    button, heading, modal_frame, Btn, BORDER, BRAND, DANGER, FIELD, MUTED, PANEL, PANEL_2,
    RADIUS_LG, TEXT,
};
use todo_interaction::{CloseChoice, Intent};

pub fn draw_projects(
    ui: &mut Ui,
    names: &[String],
    draft: &mut String,
    error: Option<&str>,
) -> Vec<Intent> {
    let mut actions = Vec::new();
    chrome::draw_strip(ui);
    let max_w = 520.0_f32.min(ui.available_width() - 48.0).max(320.0);

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space(40.0);

        ui.label(RichText::new("TO DO").size(13.0).color(BRAND).strong());
        ui.add_space(6.0);
        ui.label(RichText::new("工程").size(28.0).color(TEXT).strong());

        ui.add_space(28.0);

        egui::Frame::NONE
            .fill(PANEL)
            .stroke(egui::Stroke::new(1.0_f32, BORDER))
            .inner_margin(egui::Margin::same(20))
            .corner_radius(egui::CornerRadius::same(RADIUS_LG))
            .show(ui, |ui| {
                ui.set_width(max_w);
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    ui.label(RichText::new("工程名").size(12.0).color(MUTED));
                    ui.add_space(6.0);
                    crate::theme::with_input_radius(ui, |ui| {
                        let edit = egui::TextEdit::singleline(draft)
                            .desired_width(max_w - 40.0)
                            .margin(egui::Margin::symmetric(10, 8))
                            .text_color(TEXT)
                            .background_color(FIELD)
                            .frame(true);
                        ui.add(edit);
                    });

                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if button(ui, "新建工程", Btn::Primary, !draft.trim().is_empty()).clicked()
                        {
                            actions.push(Intent::CreateProject {
                                name: draft.trim().to_string(),
                            });
                        }
                        if button(ui, "刷新", Btn::Normal, true).clicked() {
                            actions.push(Intent::RefreshProjects);
                        }
                        if button(ui, "导入", Btn::Ghost, true).clicked() {
                            actions.push(Intent::ChooseProjectFile);
                        }
                    });

                    if let Some(error) = error {
                        ui.add_space(12.0);
                        ui.label(RichText::new(error).size(12.0).color(DANGER));
                    }
                });
            });

        ui.add_space(28.0);

        ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
            ui.set_width(max_w);
            ui.label(RichText::new("工程列表").size(12.0).color(MUTED));
            ui.add_space(10.0);

            if names.is_empty() {
                egui::Frame::NONE
                    .fill(PANEL_2)
                    .stroke(egui::Stroke::new(1.0_f32, BORDER))
                    .inner_margin(egui::Margin::symmetric(16, 20))
                    .corner_radius(egui::CornerRadius::same(RADIUS_LG))
                    .show(ui, |ui| {
                        ui.set_width(max_w - 2.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("暂无工程").size(13.0).color(MUTED));
                        });
                    });
            } else {
                for name in names {
                    let row = egui::Frame::NONE
                        .fill(PANEL)
                        .stroke(egui::Stroke::new(1.0_f32, BORDER))
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .corner_radius(egui::CornerRadius::same(RADIUS_LG))
                        .show(ui, |ui| {
                            ui.set_width(max_w - 2.0);
                            ui.horizontal(|ui| {
                                let open = ui.add(
                                    egui::Label::new(
                                        RichText::new(name.as_str()).size(14.0).color(TEXT),
                                    )
                                    .sense(Sense::click()),
                                );
                                if open.clicked() {
                                    actions.push(Intent::OpenProject { name: name.clone() });
                                }
                                if open.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if button(ui, "删除", Btn::Danger, true).clicked() {
                                        actions.push(Intent::DeleteProject { name: name.clone() });
                                    }
                                    if button(ui, "打开", Btn::Normal, true).clicked() {
                                        actions.push(Intent::OpenProject { name: name.clone() });
                                    }
                                });
                            });
                        });
                    let _ = row.response.on_hover_cursor(egui::CursorIcon::Default);
                    ui.add_space(8.0);
                }
            }
        });

        ui.add_space(24.0);
        ui.allocate_at_least(Vec2::new(max_w, 1.0), Sense::hover());
    });

    actions
}

pub fn draw_close_prompt(ctx: &egui::Context, open: bool, saving: bool) -> Vec<Intent> {
    if !open {
        return Vec::new();
    }
    let mut intents = Vec::new();
    let response = egui::Modal::new(egui::Id::new("todo_close_prompt"))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(400.0);
            egui::Frame::NONE
                .fill(PANEL_2)
                .inner_margin(egui::Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    ui.label(heading("保存更改？"));
                    if saving {
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("正在保存…").size(12.0).color(MUTED));
                        });
                    }
                });
            ui.painter().hline(
                ui.max_rect().x_range(),
                ui.cursor().top(),
                egui::Stroke::new(1.0_f32, BORDER),
            );
            egui::Frame::NONE
                .fill(FIELD)
                .inner_margin(egui::Margin::symmetric(20, 12))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button(ui, "取消", Btn::Ghost, !saving).clicked() {
                            intents.push(Intent::ConfirmClose(CloseChoice::Cancel));
                        }
                        ui.add_space(6.0);
                        if button(ui, "放弃更改", Btn::Danger, !saving).clicked() {
                            intents.push(Intent::ConfirmClose(CloseChoice::Discard));
                        }
                        ui.add_space(6.0);
                        if button(ui, "保存并关闭", Btn::Primary, !saving).clicked() {
                            intents.push(Intent::ConfirmClose(CloseChoice::Save));
                        }
                    });
                });
        });
    if !saving && intents.is_empty() && response.should_close() {
        intents.push(Intent::ConfirmClose(CloseChoice::Cancel));
    }
    intents
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        saving: bool,
        events: Vec<egui::Event>,
    ) -> (Vec<Intent>, egui::FullOutput) {
        let mut intents = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| intents = draw_close_prompt(ctx, true, saving),
        );
        (intents, output)
    }

    #[test]
    fn close_prompt_exposes_save_discard_cancel_as_intents() {
        for (label, expected) in [
            ("保存并关闭", CloseChoice::Save),
            ("放弃更改", CloseChoice::Discard),
            ("取消", CloseChoice::Cancel),
        ] {
            let ctx = egui::Context::default();
            frame(&ctx, false, vec![]);
            let (_, output) = frame(&ctx, false, vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(shape) if shape.galley.text() == label => {
                        Some(shape.pos + shape.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing close choice: {label}"));
            let pointer = |pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            };
            frame(
                &ctx,
                false,
                vec![egui::Event::PointerMoved(pos), pointer(true)],
            );
            let (intents, _) = frame(&ctx, false, vec![pointer(false)]);
            assert!(
                intents.iter().any(
                    |intent| matches!(intent, Intent::ConfirmClose(choice) if *choice == expected)
                ),
                "{intents:?}"
            );
        }
    }

    #[test]
    fn saving_close_prompt_ignores_escape() {
        let ctx = egui::Context::default();
        frame(&ctx, true, vec![]);
        let (intents, _) = frame(
            &ctx,
            true,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(intents.is_empty());
    }

    #[test]
    fn escape_from_close_prompt_cancels_the_close_request() {
        let ctx = egui::Context::default();
        frame(&ctx, false, vec![]);
        frame(&ctx, false, vec![]);
        let (intents, _) = frame(
            &ctx,
            false,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(
            matches!(
                intents.as_slice(),
                [Intent::ConfirmClose(CloseChoice::Cancel)]
            ),
            "{intents:?}"
        );
    }
}
