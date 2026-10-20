use egui::{Color32, RichText, Ui};
use todo_interaction::{AiSettingsView, AvailableModelView, KeySource, ModelCapability};

use crate::theme::{button, Btn, BORDER, BRAND, DANGER, DIM, MUTED, PANEL, PANEL_2, SUCCESS, TEXT};

/// Draws the settings form and reports what the app should do. Persistence,
/// service restart and the requests themselves remain app effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiSettingsAction {
    None,
    /// Save the form (and restart the service when its address changed).
    Save,
    /// Read the gateway's model list; free, sent when the dialog opens.
    ListModels,
    /// Save, then send one tiny chat request: a real, billed call, so only
    /// on the user's click.
    TestConnection,
}

const OPEN_ID: &str = "todo_ui_ai_settings_open";
const AUTO_TEST_ID: &str = "todo_ui_ai_settings_auto_test_sent";

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
        let available: Vec<&AvailableModelView> = models
            .iter()
            .filter(|model| model.capabilities.contains(&capability))
            .collect();
        if available.is_empty() {
            ui.add(
                egui::TextEdit::singleline(value)
                    .desired_width(340.0)
                    .hint_text(hint),
            );
            return;
        }
        egui::ComboBox::from_id_salt(("model", id))
            .selected_text(value.as_str())
            .width(340.0)
            .show_ui(ui, |ui| {
                for model in available {
                    ui.selectable_value(value, model.id.clone(), &model.id);
                }
            });
    });
}

/// "连接" row: the explicit test button, the model catalogue state and the
/// last test result.
fn connection_row(ui: &mut Ui, view: &AiSettingsView) -> bool {
    let mut test = false;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            [84.0, 28.0],
            egui::Label::new(RichText::new("连接").size(12.0).color(MUTED)),
        );
        let label = if view.huniu_testing {
            "测试中…"
        } else {
            "测试连接"
        };
        test = button(ui, label, Btn::Normal, !view.huniu_testing)
            .on_hover_text(
                "保存后发送一条很短的对话请求，确认 Key 和对话模型可用（会消耗极少额度）",
            )
            .clicked();
        if let Some((ok, message)) = &view.huniu_connection {
            ui.label(
                RichText::new(message)
                    .size(11.5)
                    .color(if *ok { SUCCESS } else { DANGER }),
            );
        }
    });
    let models = &view.huniu_models;
    let status = view.huniu_model_status.as_deref();
    if status.is_some() || !models.is_empty() {
        ui.horizontal(|ui| {
            ui.add_sized([84.0, 18.0], egui::Label::new(""));
            if let Some(status) = status {
                ui.label(RichText::new(status).size(11.0).color(MUTED));
            }
            if !models.is_empty() {
                let count = |capability| {
                    models
                        .iter()
                        .filter(|model| model.capabilities.contains(&capability))
                        .count()
                };
                ui.label(
                    RichText::new(format!(
                        "（语言 {} · 图片 {} · 视频 {} · 多模态 {}）",
                        count(ModelCapability::Language),
                        count(ModelCapability::Image),
                        count(ModelCapability::Video),
                        count(ModelCapability::Multimodal),
                    ))
                    .size(11.0)
                    .color(DIM),
                );
            }
        });
    }
    test
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

fn draw_body(ui: &mut Ui, view: &mut AiSettingsView) -> bool {
    let mut test = false;
    ui.vertical(|ui| {
        ui.label(RichText::new("AI 设置").strong().size(22.0).color(TEXT));
        ui.label(
            RichText::new("AI 成片用到的服务。Key 只保存在本机的 Windows 凭据管理器里。")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(16.0);

        service_card(
            ui,
            "模型能力",
            "模块按能力调用，供应商由后台路由",
            |ui| {
                model_field(
                    ui,
                    "chat",
                    "语言模型",
                    &mut view.huniu_language_model,
                    &view.huniu_models,
                    ModelCapability::Language,
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
                model_field(
                    ui,
                    "video",
                    "视频模型",
                    &mut view.huniu_video_model,
                    &view.huniu_models,
                    ModelCapability::Video,
                    "例如 sora-2",
                );
                model_field(
                    ui,
                    "multimodal",
                    "多模态模型",
                    &mut view.huniu_multimodal_model,
                    &view.huniu_models,
                    ModelCapability::Multimodal,
                    "例如 gemini-3.8-flash",
                );
                field(
                    ui,
                    "音频模型",
                    &mut view.volc_audio_model,
                    Some("seed-tts-2.0"),
                );
            },
        );

        ui.add_space(10.0);
        service_card(
            ui,
            "服务连接",
            "地址和密钥只配置一次，能力路由自动复用",
            |ui| {
                field(ui, "虎妞地址", &mut view.huniu_base_url, Some("https://…"));
                field(
                    ui,
                    "虎妞渠道",
                    &mut view.huniu_strategy,
                    Some("例如 XUE"),
                );
                key_card(
                    ui,
                    view.huniu_key.as_deref(),
                    view.huniu_key_source,
                    &mut view.huniu_key_entry,
                    &mut view.huniu_clear_key,
                );
                field(ui, "火山地址", &mut view.volc_base_url, Some("https://…"));
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
                test = connection_row(ui, view);
            },
        );
    });
    test
}

pub fn draw_modal(ctx: &egui::Context, view: &mut AiSettingsView) -> AiSettingsAction {
    if !is_open(ctx) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(AUTO_TEST_ID), false));
        return AiSettingsAction::None;
    }
    let first_open = ctx.data(|data| {
        !data
            .get_temp::<bool>(egui::Id::new(AUTO_TEST_ID))
            .unwrap_or(false)
    });
    if first_open {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(AUTO_TEST_ID), true));
    }
    let mut action = first_open.then_some(AiSettingsAction::ListModels);
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
                .max_height(520.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if draw_body(ui, view) {
                        action = Some(AiSettingsAction::TestConnection);
                    }
                });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, "保存", Btn::Primary, true).clicked() {
                    set_open(ui, false);
                    action = Some(AiSettingsAction::Save);
                }
                if button(ui, "取消", Btn::Normal, true).clicked() {
                    set_open(ui, false);
                    action = None;
                }
            });
        });
    if response.should_close() {
        close(ctx);
        action = None;
    }
    action.unwrap_or(AiSettingsAction::None)
}
