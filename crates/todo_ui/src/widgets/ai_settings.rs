use egui::{Color32, RichText, Ui};
use todo_interaction::{AiSettingsView, AvailableModelView, KeySource, ModelCapability};

use crate::theme::{button, Btn, BORDER, BRAND, DANGER, DIM, MUTED, PANEL, PANEL_2, TEXT};

/// Draws the settings form and reports whether the user pressed Save or Test.
/// Persistence and service restart remain application effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiSettingsAction {
    None,
    Save,
    TestConnection,
}

const OPEN_ID: &str = "todo_ui_ai_settings_open";

pub fn set_open(ui: &Ui, open: bool) {
    set_open_ctx(ui.ctx(), open);
}

pub fn set_open_ctx(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), open));
}

pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new(OPEN_ID)))
        .unwrap_or(false)
}

pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), false));
}

fn field(ui: &mut Ui, label: &str, value: &mut String, hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [84.0, 28.0],
            egui::Label::new(RichText::new(label).size(12.0).color(MUTED)),
        );
        let mut edit = egui::TextEdit::singleline(value).desired_width(340.0);
        if let Some(hint) = hint {
            edit = edit.hint_text(hint);
        }
        ui.add(edit);
    });
}

fn model_field(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut String,
    models: &[AvailableModelView],
    capability: ModelCapability,
    hint: &str,
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [84.0, 28.0],
            egui::Label::new(RichText::new(label).size(12.0).color(MUTED)),
        );
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(220.0)
                .hint_text(hint),
        );
        let available: Vec<&AvailableModelView> = models
            .iter()
            .filter(|model| model.capabilities.contains(&capability))
            .collect();
        if available.is_empty() {
            return;
        }
        egui::ComboBox::from_id_salt(("model", id))
            .selected_text(format!("发现 {} 个", available.len()))
            .width(92.0)
            .show_ui(ui, |ui| {
                for model in available {
                    if ui.selectable_label(value == &model.id, &model.id).clicked() {
                        *value = model.id.clone();
                    }
                }
            });
    });
}

fn model_catalog(ui: &mut Ui, models: &[AvailableModelView], status: Option<&str>) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_sized([84.0, 24.0], egui::Label::new(""));
        ui.label(RichText::new("模型能力").size(12.0).color(MUTED));
        if let Some(status) = status {
            ui.label(RichText::new(status).size(11.0).color(BRAND));
        }
    });
    for (capability, label) in [
        (ModelCapability::Text, "文本"),
        (ModelCapability::Image, "图片"),
        (ModelCapability::Video, "视频"),
    ] {
        let names: Vec<&str> = models
            .iter()
            .filter(|model| model.capabilities.contains(&capability))
            .map(|model| model.id.as_str())
            .collect();
        if names.is_empty() {
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([84.0, 22.0], egui::Label::new(""));
            ui.label(RichText::new(format!("{label}：")).size(11.0).color(MUTED));
            for name in names {
                ui.label(
                    RichText::new(name)
                        .size(11.0)
                        .color(TEXT)
                        .background_color(PANEL),
                );
            }
        });
    }
}

fn key_color(source: KeySource) -> Color32 {
    match source {
        KeySource::Environment => BRAND,
        KeySource::Keychain => Color32::from_rgb(0x77, 0xd4, 0xa5),
        KeySource::Missing => DANGER,
    }
}

fn key_card(
    ui: &mut Ui,
    key: Option<&str>,
    source: KeySource,
    entry: &mut String,
    clear: &mut bool,
) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            [84.0, 24.0],
            egui::Label::new(RichText::new("API Key").size(12.0).color(MUTED)),
        );
        ui.label(
            RichText::new(key.unwrap_or("未设置"))
                .size(12.0)
                .color(key_color(source)),
        );
    });
    ui.horizontal(|ui| {
        ui.add_sized([84.0, 24.0], egui::Label::new(""));
        ui.label(
            RichText::new(match source {
                KeySource::Environment => "来源：环境变量（只读）",
                KeySource::Keychain => "来源：Windows 凭据管理器",
                KeySource::Missing => "来源：还没有配置",
            })
            .size(11.0)
            .color(MUTED),
        );
    });
    ui.horizontal(|ui| {
        ui.add_sized([84.0, 28.0], egui::Label::new(""));
        ui.add_enabled_ui(source != KeySource::Environment, |ui| {
            ui.add(
                egui::TextEdit::singleline(entry)
                    .password(true)
                    .desired_width(250.0)
                    .hint_text("输入新的 API Key（留空不修改）"),
            );
            if button(ui, "清除", Btn::Danger, true).clicked() {
                *clear = true;
            }
        });
    });
}

fn service_card(ui: &mut Ui, title: &str, subtitle: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .fill(PANEL_2)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .inner_margin(14.0)
        .corner_radius(egui::CornerRadius::same(10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong().size(15.0).color(TEXT));
                ui.label(RichText::new(subtitle).size(11.0).color(MUTED));
            });
            ui.add_space(10.0);
            add(ui);
        });
}

pub fn draw(ui: &mut Ui, view: &mut AiSettingsView) -> AiSettingsAction {
    let mut action = AiSettingsAction::None;
    ui.vertical(|ui| {
        ui.label(RichText::new("AI 设置").strong().size(22.0).color(TEXT));
        ui.label(
            RichText::new("配置生成内容、服务地址和密钥来源，保存后新任务立即使用")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(16.0);

        service_card(ui, "虎妞", "对话与图片生成", |ui| {
            field(ui, "服务地址", &mut view.huniu_base_url, Some("https://…"));
            model_field(
                ui,
                "chat",
                "对话模型",
                &mut view.huniu_chat_model,
                &view.huniu_models,
                ModelCapability::Text,
                "默认：geminiflash3.1",
            );
            model_field(
                ui,
                "image",
                "生图模型",
                &mut view.huniu_image_model,
                &view.huniu_models,
                ModelCapability::Image,
                "例如 gpt-image-2",
            );
            key_card(
                ui,
                view.huniu_key.as_deref(),
                view.huniu_key_source,
                &mut view.huniu_key_entry,
                &mut view.huniu_clear_key,
            );
            model_catalog(ui, &view.huniu_models, view.huniu_model_status.as_deref());
        });

        ui.add_space(10.0);
        service_card(ui, "火山", "配音与字幕时间线", |ui| {
            field(ui, "服务地址", &mut view.volc_base_url, Some("https://…"));
            field(
                ui,
                "默认音色",
                &mut view.volc_speaker,
                Some("zh_female_mizai_uranus_bigtts"),
            );
            key_card(
                ui,
                view.volc_key.as_deref(),
                view.volc_key_source,
                &mut view.volc_key_entry,
                &mut view.volc_clear_key,
            );
        });

        ui.add_space(14.0);
        ui.label(
            RichText::new("测试连接会发送一次最小虎妞对话请求，不会生成图片或配音")
                .size(11.0)
                .color(DIM),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if button(ui, "测试连接", Btn::Ghost, true).clicked() {
                action = AiSettingsAction::TestConnection;
            }
            if button(ui, "保存设置", Btn::Primary, true).clicked() {
                action = AiSettingsAction::Save;
                set_open(ui, false);
            }
            if button(ui, "取消", Btn::Normal, true).clicked() {
                set_open(ui, false);
            }
        });
    });
    action
}

pub fn draw_modal(ctx: &egui::Context, view: &mut AiSettingsView) -> AiSettingsAction {
    if !is_open(ctx) {
        return AiSettingsAction::None;
    }
    let mut action = AiSettingsAction::None;
    let response = egui::Modal::new(egui::Id::new("todo_ai_settings_modal"))
        .frame(
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(egui::Stroke::new(1.0_f32, BORDER))
                .inner_margin(22.0)
                .corner_radius(egui::CornerRadius::same(12)),
        )
        .show(ctx, |ui| {
            ui.set_width(520.0);
            egui::ScrollArea::vertical()
                .max_height(600.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    action = draw(ui, view);
                });
        });
    if response.should_close() {
        close(ctx);
    }
    action
}
