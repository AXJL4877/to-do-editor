//! "新建成片": pick a style, paste the script, adjust the quick settings and
//! start. Everything the manifest offers beyond the quick settings sits in a
//! collapsed "更多设置", grouped as the manifest groups it.
use super::{caption, ProductionAction};
use crate::theme::{button, Btn, BORDER, BORDER_STRONG, BRAND, DIM, MUTED, PANEL_2, TEXT, WARN};
use egui::{Id, RichText, Ui};
use std::collections::BTreeMap;
use todo_interaction::{
    when_visible, ExtensionCardView, ExtensionStatus, ParamInput, ParamValue, ParamView,
    ProductionPanelView,
};

pub(super) const FORM_ID: &str = "todo_ui_production_form";

#[derive(Debug, Clone, Default)]
pub(super) struct Form {
    extension: Option<String>,
    values: BTreeMap<String, ParamValue>,
    pub(super) script: String,
    /// Voice params switched to a typed custom id.
    custom_voice: BTreeMap<String, bool>,
}

fn text_of(value: &ParamValue) -> String {
    match value {
        ParamValue::Text(text) => text.clone(),
        ParamValue::Number(n) => format!("{n}"),
        ParamValue::Bool(b) => b.to_string(),
    }
}

fn status_badge(status: ExtensionStatus) -> (&'static str, egui::Color32) {
    match status {
        ExtensionStatus::Ready => ("可用", crate::theme::SUCCESS),
        ExtensionStatus::Checking => ("检查中", MUTED),
        ExtensionStatus::MissingKey => ("需要 Key", WARN),
        ExtensionStatus::RuntimeMissing | ExtensionStatus::SelfCheckFailed => {
            ("不可用", crate::theme::DANGER)
        }
    }
}

/// One selectable style tile.
fn style_tile(ui: &mut Ui, card: &ExtensionCardView, selected: bool) -> egui::Response {
    let (badge, badge_color) = status_badge(card.status);
    let frame = egui::Frame::NONE
        .fill(if selected {
            egui::Color32::from_rgb(0x1d, 0x30, 0x3d)
        } else {
            PANEL_2
        })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if selected { BRAND } else { BORDER },
        ))
        .inner_margin(egui::Margin::symmetric(10, 8));
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&card.name).strong().size(13.5).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(badge).size(11.0).color(badge_color));
            });
        });
        let summary = card.summary();
        if !summary.is_empty() {
            ui.label(RichText::new(summary).size(11.5).color(MUTED));
        }
    });
    let response = ui
        .interact(
            inner.response.rect,
            Id::new(("style_tile", &card.id)),
            egui::Sense::click(),
        )
        .on_hover_text(&card.description);
    if response.hovered() && !selected {
        ui.painter().rect_stroke(
            inner.response.rect,
            0.0,
            egui::Stroke::new(1.0_f32, BORDER_STRONG),
            egui::StrokeKind::Inside,
        );
    }
    response
}

fn label_cell(ui: &mut Ui, param: &ParamView) {
    let label = ui.add_sized(
        [76.0, 24.0],
        egui::Label::new(RichText::new(&param.label).size(12.0).color(MUTED)).truncate(),
    );
    if let Some(hint) = &param.hint {
        label.on_hover_text(hint);
    }
}

fn param_field(ui: &mut Ui, param: &ParamView, value: &mut ParamValue, custom: &mut bool) {
    ui.horizontal(|ui| {
        label_cell(ui, param);
        let width = ui.available_width().max(120.0);
        match (&param.input, value) {
            (ParamInput::Select(choices), ParamValue::Text(current)) => {
                let shown = choices
                    .iter()
                    .find(|c| &c.value == current)
                    .map_or_else(|| current.clone(), |c| c.label.clone());
                egui::ComboBox::from_id_salt(("param", &param.id))
                    .selected_text(shown)
                    .width(width)
                    .show_ui(ui, |ui| {
                        for choice in choices {
                            let response =
                                ui.selectable_value(current, choice.value.clone(), &choice.label);
                            if let Some(hint) = &choice.hint {
                                response.on_hover_text(hint);
                            }
                        }
                    });
            }
            (ParamInput::Voice(choices), ParamValue::Text(current)) => {
                let known = choices.iter().find(|c| &c.value == current);
                if known.is_none() && !current.is_empty() {
                    *custom = true;
                }
                let shown = if *custom {
                    "自定义音色".to_owned()
                } else {
                    known.map_or_else(|| current.clone(), |c| c.label.clone())
                };
                ui.vertical(|ui| {
                    egui::ComboBox::from_id_salt(("voice", &param.id))
                        .selected_text(shown)
                        .width(width)
                        .show_ui(ui, |ui| {
                            for choice in choices {
                                let picked = !*custom && current == &choice.value;
                                if ui.selectable_label(picked, &choice.label).clicked() {
                                    *current = choice.value.clone();
                                    *custom = false;
                                }
                            }
                            ui.separator();
                            if ui
                                .selectable_label(*custom, "自定义音色…")
                                .on_hover_text("填写声音复刻得到的 S_ 开头音色 ID")
                                .clicked()
                            {
                                *custom = true;
                            }
                        });
                    if *custom {
                        ui.add(
                            egui::TextEdit::singleline(current)
                                .desired_width(width)
                                .hint_text("音色 ID，例如 S_xxxxxx"),
                        );
                    }
                });
            }
            (
                ParamInput::Number {
                    min,
                    max,
                    step,
                    unit,
                },
                ParamValue::Number(n),
            ) => {
                let mut slider = egui::Slider::new(n, *min..=*max).step_by(*step);
                if let Some(unit) = unit {
                    slider = slider.suffix(format!(" {unit}"));
                }
                ui.add(slider);
            }
            (
                ParamInput::Text {
                    max_length,
                    placeholder,
                },
                ParamValue::Text(text),
            ) => {
                let mut edit = egui::TextEdit::singleline(text)
                    .char_limit(*max_length)
                    .desired_width(width);
                if let Some(p) = placeholder {
                    edit = edit.hint_text(p);
                }
                ui.add(edit);
            }
            (ParamInput::Bool, ParamValue::Bool(b)) => {
                ui.checkbox(b, "");
            }
            (_, value) => {
                ui.label(text_of(value));
            }
        }
    });
}

fn draw_params(ui: &mut Ui, card: &ExtensionCardView, form: &mut Form) {
    let selects: BTreeMap<String, String> = form
        .values
        .iter()
        .map(|(k, v)| (k.clone(), text_of(v)))
        .collect();
    let visible: Vec<&ParamView> = card
        .params
        .iter()
        .filter(|p| when_visible(&p.when, &selects))
        .collect();
    for param in visible.iter().filter(|p| p.quick) {
        if let Some(value) = form.values.get_mut(&param.id) {
            let custom = form.custom_voice.entry(param.id.clone()).or_default();
            param_field(ui, param, value, custom);
        }
    }
    let others: Vec<&&ParamView> = visible.iter().filter(|p| !p.quick).collect();
    if others.is_empty() {
        return;
    }
    ui.add_space(4.0);
    egui::CollapsingHeader::new(RichText::new("更多设置").size(12.5).color(MUTED))
        .id_salt(("more", &card.id))
        .default_open(false)
        .show(ui, |ui| {
            let mut groups: Vec<(&str, Vec<&ParamView>)> = Vec::new();
            for param in others {
                match groups.iter_mut().find(|(g, _)| *g == param.group) {
                    Some((_, list)) => list.push(param),
                    None => groups.push((param.group.as_str(), vec![param])),
                }
            }
            for (group, params) in groups {
                ui.add_space(4.0);
                ui.label(RichText::new(group).size(11.5).color(DIM));
                for param in params {
                    if let Some(value) = form.values.get_mut(&param.id) {
                        let custom = form.custom_voice.entry(param.id.clone()).or_default();
                        param_field(ui, param, value, custom);
                    }
                }
            }
            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("恢复默认设置").size(11.5).color(MUTED))
                        .frame(false),
                )
                .clicked()
            {
                form.values = card
                    .params
                    .iter()
                    .map(|p| (p.id.clone(), p.default.clone()))
                    .collect();
                form.custom_voice.clear();
            }
        });
}

/// A highlighted note box (warnings and blockers).
fn notice(ui: &mut Ui, text: &str, color: egui::Color32) {
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.5)))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(12.0).color(TEXT));
        });
}

pub(super) fn draw(ui: &mut Ui, view: &ProductionPanelView) -> ProductionAction {
    let ctx = ui.ctx().clone();
    let id = Id::new(FORM_ID);
    let mut form: Form = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut action = ProductionAction::None;
    if view.extensions.is_empty() {
        ui.add_space(20.0);
        ui.label(RichText::new("没有可用的成片风格").color(MUTED));
        let reason = view
            .service_problem
            .as_deref()
            .unwrap_or("扩展包缺失，请重新安装软件。");
        ui.label(RichText::new(reason).size(12.0).color(DIM));
        return action;
    }
    if form
        .extension
        .as_ref()
        .is_none_or(|id| !view.extensions.iter().any(|c| &c.id == id))
    {
        form.extension = view
            .extensions
            .iter()
            .find(|c| c.status == ExtensionStatus::Ready)
            .or(view.extensions.first())
            .map(|c| c.id.clone());
        form.values.clear();
    }

    caption(ui, Some(1), "选择风格");
    for card in &view.extensions {
        let selected = form.extension.as_deref() == Some(card.id.as_str());
        if style_tile(ui, card, selected).clicked() && !selected {
            form.extension = Some(card.id.clone());
            form.values.clear();
            form.custom_voice.clear();
        }
        ui.add_space(2.0);
    }
    let Some(card) = view
        .extensions
        .iter()
        .find(|c| Some(&c.id) == form.extension.as_ref())
    else {
        ctx.data_mut(|d| d.insert_temp(id, form));
        return action;
    };
    for param in &card.params {
        form.values
            .entry(param.id.clone())
            .or_insert_with(|| param.default.clone());
    }
    if let Some(blocker) = card.blocker() {
        ui.add_space(4.0);
        notice(ui, blocker, WARN);
        if card.status == ExtensionStatus::MissingKey {
            ui.add_space(4.0);
            if button(ui, "去填写 Key", Btn::Normal, true).clicked() {
                action = ProductionAction::OpenSettings;
            }
        }
    }

    if let Some(script) = &card.script {
        ui.add_space(10.0);
        caption(ui, Some(2), &script.label);
        ui.add(
            egui::TextEdit::multiline(&mut form.script)
                .hint_text(&script.placeholder)
                .char_limit(script.max_length)
                .desired_rows(9)
                .desired_width(f32::INFINITY),
        );
        let count = form.script.chars().filter(|c| !c.is_whitespace()).count();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("空一行表示这里必须换段")
                    .size(11.0)
                    .color(DIM),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{count} / {} 字", script.max_length))
                        .size(11.0)
                        .color(MUTED),
                );
            });
        });
    }

    if !card.params.is_empty() {
        ui.add_space(10.0);
        caption(ui, Some(3), "基本设置");
        draw_params(ui, card, &mut form);
    }

    ui.add_space(14.0);
    let has_script = card.script.as_ref().is_none_or(|s| s.optional)
        || !form.script.trim().is_empty();
    let ready = card.status == ExtensionStatus::Ready;
    let reason = if !view.project_open {
        Some("先打开一个工程，成片会放进它的时间轴")
    } else if !ready {
        card.blocker()
    } else if !has_script {
        Some("先填写文案")
    } else {
        None
    };
    let can_start = reason.is_none();
    let start = ui.add_enabled(
        can_start,
        egui::Button::new(
            RichText::new("开始生成")
                .size(14.0)
                .strong()
                .color(if can_start {
                    egui::Color32::from_rgb(0x08, 0x17, 0x25)
                } else {
                    MUTED
                }),
        )
        .fill(if can_start { BRAND } else { PANEL_2 })
        .min_size(egui::vec2(ui.available_width(), 36.0)),
    );
    if let Some(reason) = reason {
        ui.label(RichText::new(reason).size(11.5).color(MUTED));
    } else {
        ui.label(
            RichText::new("生成时可以继续编辑。配音、字幕和画面会按顺序自动放进时间轴。")
                .size(11.5)
                .color(DIM),
        );
    }
    if start.clicked() {
        action = ProductionAction::Start {
            extension: card.id.clone(),
            params: form.values.clone(),
            script: form.script.clone(),
        };
    }
    ctx.data_mut(|d| d.insert_temp(id, form));
    action
}
