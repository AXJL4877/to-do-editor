//! The AI production panel: pick an extension, fill its generated form and
//! the script, start; below, every production of the project with its
//! progress tree and retry buttons. Form state lives in egui memory.
use crate::ThumbCache;
use egui::{Color32, RichText, Ui};
use std::collections::BTreeMap;
use todo_interaction::{
    when_visible, ElementPatch, ExtensionCardView, ExtensionStatus, ParamInput, ParamValue,
    ParamView, ProductionEdit, ProductionNodeStatus, ProductionNodeView, ProductionPanelView,
    ProductionView,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ProductionAction {
    None,
    OpenSettings,
    Start {
        extension: String,
        params: BTreeMap<String, ParamValue>,
        script: String,
    },
    /// `key: None` retries every failed bake or run of the production.
    Retry {
        production: String,
        key: Option<String>,
    },
    Remove {
        production: String,
    },
    Edit {
        production: String,
        edit: ProductionEdit,
    },
    Upload {
        production: String,
        element: String,
    },
    Thumbnail {
        path: String,
    },
}

const OPEN_ID: &str = "todo_ui_production_open";
const FORM_ID: &str = "todo_ui_production_form";

#[derive(Debug, Clone, Default)]
struct Form {
    extension: Option<String>,
    values: BTreeMap<String, ParamValue>,
    script: String,
    more: bool,
}

pub fn set_open(ui: &Ui, open: bool) {
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), open));
}

pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new(OPEN_ID)))
        .unwrap_or(false)
}

pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), false));
}

fn status_text(status: ExtensionStatus) -> (&'static str, Color32) {
    match status {
        ExtensionStatus::MissingKey => ("缺少 key", Color32::from_rgb(220, 160, 60)),
        ExtensionStatus::RuntimeMissing => ("运行环境缺失", Color32::from_rgb(220, 90, 80)),
        ExtensionStatus::SelfCheckFailed => ("自检失败", Color32::from_rgb(220, 90, 80)),
        ExtensionStatus::Checking => ("自检中…", Color32::GRAY),
        ExtensionStatus::Ready => ("就绪", Color32::from_rgb(110, 190, 120)),
    }
}

fn text_of(value: &ParamValue) -> String {
    match value {
        ParamValue::Text(text) => text.clone(),
        ParamValue::Number(n) => format!("{n}"),
        ParamValue::Bool(b) => b.to_string(),
    }
}

fn param_field(ui: &mut Ui, param: &ParamView, value: &mut ParamValue) {
    let label = ui.label(&param.label);
    if let Some(hint) = &param.hint {
        label.on_hover_text(hint);
    }
    match (&param.input, value) {
        (ParamInput::Select(choices), ParamValue::Text(current)) => {
            let shown = choices
                .iter()
                .find(|c| &c.value == current)
                .map_or_else(|| current.clone(), |c| c.label.clone());
            egui::ComboBox::from_id_salt(("param", &param.id))
                .selected_text(shown)
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
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(current).desired_width(200.0));
                egui::ComboBox::from_id_salt(("voice", &param.id))
                    .selected_text("常用")
                    .show_ui(ui, |ui| {
                        for choice in choices {
                            ui.selectable_value(current, choice.value.clone(), &choice.label);
                        }
                    });
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
            let mut drag = egui::DragValue::new(n).range(*min..=*max).speed(*step);
            if let Some(unit) = unit {
                drag = drag.suffix(format!(" {unit}"));
            }
            ui.add(drag);
        }
        (
            ParamInput::Text {
                max_length,
                placeholder,
            },
            ParamValue::Text(text),
        ) => {
            let mut edit = egui::TextEdit::singleline(text).char_limit(*max_length);
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
    ui.end_row();
}

fn draw_form(
    ui: &mut Ui,
    card: &ExtensionCardView,
    form: &mut Form,
    ready: bool,
    project_open: bool,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    for param in &card.params {
        form.values
            .entry(param.id.clone())
            .or_insert_with(|| param.default.clone());
    }
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
    egui::Grid::new(("form", &card.id))
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for param in visible.iter().filter(|p| p.quick) {
                if let Some(value) = form.values.get_mut(&param.id) {
                    param_field(ui, param, value);
                }
            }
        });
    let others: Vec<&&ParamView> = visible.iter().filter(|p| !p.quick).collect();
    if !others.is_empty() {
        let label = if form.more {
            "收起更多参数"
        } else {
            "更多参数…"
        };
        ui.toggle_value(&mut form.more, label);
        if form.more {
            let mut groups: BTreeMap<&str, Vec<&ParamView>> = BTreeMap::new();
            for param in others {
                groups.entry(param.group.as_str()).or_default().push(param);
            }
            for (group, params) in groups {
                ui.label(RichText::new(group).strong());
                egui::Grid::new(("more", &card.id, group))
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for param in params {
                            if let Some(value) = form.values.get_mut(&param.id) {
                                param_field(ui, param, value);
                            }
                        }
                    });
            }
        }
    }
    if let Some(script) = &card.script {
        ui.add_space(6.0);
        ui.label(format!(
            "{}（{} / {} 字）",
            script.label,
            form.script.chars().count(),
            script.max_length
        ));
        ui.add(
            egui::TextEdit::multiline(&mut form.script)
                .hint_text(&script.placeholder)
                .char_limit(script.max_length)
                .desired_rows(8)
                .desired_width(f32::INFINITY),
        );
    }
    ui.add_space(6.0);
    let has_script = card.script.is_none() || !form.script.trim().is_empty();
    let can_start = ready && project_open && has_script;
    let start = ui.add_enabled(can_start, egui::Button::new("开始生成"));
    if !project_open {
        ui.small("先打开一个工程，成片会放进它的时间轴。");
    } else if !has_script {
        ui.small("请先填写文案。");
    }
    if start.clicked() {
        action = ProductionAction::Start {
            extension: card.id.clone(),
            params: form.values.clone(),
            script: form.script.clone(),
        };
    }
    action
}

fn node_status(status: ProductionNodeStatus) -> (&'static str, Color32) {
    match status {
        ProductionNodeStatus::Queued => ("排队", Color32::GRAY),
        ProductionNodeStatus::Running => ("生成中", Color32::from_rgb(110, 160, 230)),
        ProductionNodeStatus::Waiting => ("等待网络", Color32::from_rgb(220, 160, 60)),
        ProductionNodeStatus::Attention => ("需要处理", Color32::from_rgb(220, 160, 60)),
        ProductionNodeStatus::Complete => ("完成", Color32::from_rgb(110, 190, 120)),
        ProductionNodeStatus::Failed => ("失败", Color32::from_rgb(220, 90, 80)),
    }
}

fn draw_node(ui: &mut Ui, node: &ProductionNodeView) {
    let (text, color) = node_status(node.status);
    let title = RichText::new(format!("{} · {}", node.label, text)).color(color);
    if node.children.is_empty() {
        ui.label(title);
        if let Some(detail) = &node.detail {
            ui.small(detail);
        }
        return;
    }
    egui::CollapsingHeader::new(title)
        .id_salt(("node", &node.key))
        .default_open(node.status != ProductionNodeStatus::Complete)
        .show(ui, |ui| {
            if let Some(detail) = &node.detail {
                ui.small(detail);
            }
            for child in &node.children {
                draw_node(ui, child);
            }
        });
}

fn draw_production(
    ui: &mut Ui,
    production: &ProductionView,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&production.title).strong());
            ui.small(&production.extension);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("移出列表")
                    .on_hover_text("停止生成；已入轨的片段保留")
                    .clicked()
                {
                    action = ProductionAction::Remove {
                        production: production.id.clone(),
                    };
                }
                if ui.small_button("重试失败项").clicked() {
                    action = ProductionAction::Retry {
                        production: production.id.clone(),
                        key: None,
                    };
                }
            });
        });
        let fraction = if production.total == 0 {
            0.0
        } else {
            production.completed as f32 / production.total as f32
        };
        ui.add(
            egui::ProgressBar::new(fraction)
                .text(format!("{} / {}", production.completed, production.total)),
        );
        for line in &production.attention {
            ui.colored_label(Color32::from_rgb(220, 160, 60), line);
        }
        for failure in &production.failures {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(Color32::from_rgb(220, 90, 80), &failure.message);
                let label = if failure.possibly_billed {
                    "确认后重试"
                } else {
                    "重试"
                };
                let button = ui.small_button(label);
                let button = if failure.possibly_billed {
                    button.on_hover_text("该请求可能已经扣费；确认需要再试一次")
                } else {
                    button
                };
                if button.clicked() {
                    action = ProductionAction::Retry {
                        production: production.id.clone(),
                        key: Some(failure.key.clone()),
                    };
                }
            });
        }
        for node in &production.nodes {
            draw_node(ui, node);
        }
        draw_editors(ui, production, thumbs, &mut action);
    });
    action
}

fn draw_editors(
    ui: &mut Ui,
    production: &ProductionView,
    thumbs: &mut ThumbCache,
    action: &mut ProductionAction,
) {
    if production.segments.is_empty() {
        return;
    }
    ui.separator();
    ui.label(RichText::new("可编辑内容").strong());
    for segment in &production.segments {
        egui::CollapsingHeader::new(format!("段落 {}", segment.id))
            .id_salt(("segment-edit", &production.id, &segment.id))
            .default_open(false)
            .show(ui, |ui| {
                // Segment text uses a small persistent field in the panel form;
                // the actual write happens only when the user presses保存.
                let mut text = ui
                    .ctx()
                    .data(|data| {
                        data.get_temp::<String>(egui::Id::new((
                            "segment-text",
                            &production.id,
                            &segment.id,
                        )))
                    })
                    .unwrap_or_else(|| segment.text.clone());
                ui.add(egui::TextEdit::multiline(&mut text).desired_rows(3));
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new(("segment-text", &production.id, &segment.id)),
                        text.clone(),
                    );
                });
                if ui.small_button("保存段落文案").clicked() && text != segment.text {
                    *action = ProductionAction::Edit {
                        production: production.id.clone(),
                        edit: ProductionEdit::SetSegmentText {
                            segment: segment.id.clone(),
                            text,
                        },
                    };
                }
                for element in &segment.elements {
                    draw_element_editor(ui, production, element, thumbs, action);
                }
            });
    }
}

fn draw_element_editor(
    ui: &mut Ui,
    production: &ProductionView,
    element: &todo_interaction::ElementView,
    thumbs: &mut ThumbCache,
    action: &mut ProductionAction,
) {
    egui::CollapsingHeader::new(format!("{} · {}", element.label, element.id))
        .id_salt(("element-edit", &production.id, &element.id))
        .show(ui, |ui| {
            let mut draw = ui
                .ctx()
                .data(|data| {
                    data.get_temp::<String>(egui::Id::new((
                        "element-draw",
                        &production.id,
                        &element.id,
                    )))
                })
                .unwrap_or_else(|| element.draw.clone());
            ui.add(egui::TextEdit::multiline(&mut draw).desired_rows(2));
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(("element-draw", &production.id, &element.id)),
                    draw.clone(),
                );
            });
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("保存描述").clicked() && draw != element.draw {
                    *action = ProductionAction::Edit {
                        production: production.id.clone(),
                        edit: ProductionEdit::SetElement {
                            element: element.id.clone(),
                            patch: ElementPatch {
                                draw: Some(draw),
                                ..Default::default()
                            },
                        },
                    };
                }
                if ui.small_button("再来一张").clicked() {
                    *action = ProductionAction::Edit {
                        production: production.id.clone(),
                        edit: ProductionEdit::AddVariant {
                            element: element.id.clone(),
                        },
                    };
                }
                if ui.small_button("上传图片").clicked() {
                    *action = ProductionAction::Upload {
                        production: production.id.clone(),
                        element: element.id.clone(),
                    };
                }
            });
            ui.horizontal_wrapped(|ui| {
                for variant in &element.variants {
                    let picked = Some(&variant.id) == element.picked.as_ref();
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(84.0, 96.0), egui::Sense::click());
                    if let Some(path) = variant.file.as_deref() {
                        if !thumbs.contains(path) {
                            *action = ProductionAction::Thumbnail {
                                path: path.to_owned(),
                            };
                        }
                        if !thumbs.paint(ui, path, rect.shrink(4.0)) {
                            ui.painter().rect_filled(
                                rect.shrink(4.0),
                                4.0,
                                Color32::from_rgb(48, 52, 60),
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "加载中…",
                                egui::FontId::proportional(11.0),
                                Color32::LIGHT_GRAY,
                            );
                        }
                    } else {
                        ui.painter().rect_filled(
                            rect.shrink(4.0),
                            4.0,
                            Color32::from_rgb(48, 52, 60),
                        );
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "无图片",
                            egui::FontId::proportional(11.0),
                            Color32::LIGHT_GRAY,
                        );
                    }
                    let label = if picked {
                        format!("✓ {}", variant.id)
                    } else if variant.locked {
                        format!("🔒 {}", variant.id)
                    } else {
                        variant.id.clone()
                    };
                    let label_rect = egui::Rect::from_min_max(
                        egui::pos2(rect.left(), rect.bottom() - 18.0),
                        egui::pos2(rect.right(), rect.bottom()),
                    );
                    let mut label_ui = ui.new_child(egui::UiBuilder::new().max_rect(label_rect));
                    label_ui.centered_and_justified(|ui| {
                        ui.small(label);
                    });
                    if response.clicked() && !variant.locked {
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

pub fn draw(ui: &mut Ui, view: &ProductionPanelView, thumbs: &mut ThumbCache) -> ProductionAction {
    let mut action = ProductionAction::None;
    let id = egui::Id::new(FORM_ID);
    let mut form: Form = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    if form.extension.is_none() {
        form.extension = view.extensions.first().map(|c| c.id.clone());
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for card in &view.extensions {
                    let (status, color) = status_text(card.status);
                    let selected = form.extension.as_deref() == Some(card.id.as_str());
                    let text = RichText::new(format!("{} · {}", card.name, status)).color(color);
                    if ui
                        .selectable_label(selected, text)
                        .on_hover_text(&card.description)
                        .clicked()
                        && !selected
                    {
                        form.extension = Some(card.id.clone());
                        form.values.clear();
                    }
                }
            });
            if view.extensions.is_empty() {
                ui.label("没有可用的扩展。");
            }
            if let Some(card) = view
                .extensions
                .iter()
                .find(|c| Some(&c.id) == form.extension.as_ref())
            {
                ui.small(&card.description);
                if let Some(problem) = &card.problem {
                    ui.colored_label(Color32::from_rgb(220, 90, 80), problem);
                }
                if card.status == ExtensionStatus::MissingKey
                    && ui.button("去 AI 设置填写 key").clicked()
                {
                    action = ProductionAction::OpenSettings;
                }
                ui.separator();
                let started = draw_form(
                    ui,
                    card,
                    &mut form,
                    card.status == ExtensionStatus::Ready,
                    view.project_open,
                );
                if started != ProductionAction::None {
                    action = started;
                }
            }
            if !view.productions.is_empty() {
                ui.separator();
                ui.heading("本工程的成片");
                for production in &view.productions {
                    let next = draw_production(ui, production, thumbs);
                    if next != ProductionAction::None {
                        action = next;
                    }
                }
            }
        });
    ui.ctx().data_mut(|d| d.insert_temp(id, form));
    action
}

pub fn draw_modal(
    ctx: &egui::Context,
    view: &ProductionPanelView,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    if !is_open(ctx) {
        return ProductionAction::None;
    }
    let mut action = ProductionAction::None;
    let mut open = true;
    egui::Window::new("AI 成片")
        .open(&mut open)
        .resizable(true)
        .default_width(520.0)
        .default_height(640.0)
        .show(ctx, |ui| {
            action = draw(ui, view, thumbs);
        });
    if !open {
        close(ctx);
    }
    action
}
