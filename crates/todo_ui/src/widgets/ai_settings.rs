use egui::{Color32, RichText, Ui};
use todo_interaction::{AiSettingsView, AvailableModelView, KeySource, ModelCapability};

use crate::theme::{
    button, heading, label_muted, modal_frame, section_card, with_input_radius, Btn, BORDER, BRAND,
    DANGER, FIELD, MUTED, PANEL_2, RADIUS, SUCCESS,
};

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
const LABEL_W: f32 = 96.0;
const FIELD_W: f32 = 360.0;

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
        ui.add_sized([LABEL_W, 30.0], egui::Label::new(label_muted(label)));
        with_input_radius(ui, |ui| {
            let mut edit = egui::TextEdit::singleline(value)
                .desired_width(FIELD_W)
                .margin(egui::Margin::symmetric(10, 6));
            if let Some(hint) = hint {
                edit = edit.hint_text(hint);
            }
            ui.add(edit);
        });
    });
    ui.add_space(4.0);
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
        ui.add_sized([LABEL_W, 30.0], egui::Label::new(label_muted(label)));
        let available: Vec<&AvailableModelView> = models
            .iter()
            .filter(|model| model.capabilities.contains(&capability))
            .collect();
        if available.is_empty() {
            with_input_radius(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(value)
                        .desired_width(FIELD_W)
                        .hint_text(hint)
                        .margin(egui::Margin::symmetric(10, 6)),
                );
            });
            return;
        }
        with_input_radius(ui, |ui| {
            egui::ComboBox::from_id_salt(("model", id))
                .selected_text(value.as_str())
                .width(FIELD_W)
                .show_ui(ui, |ui| {
                    for model in available {
                        ui.selectable_value(value, model.id.clone(), &model.id);
                    }
                });
        });
    });
    ui.add_space(4.0);
}

fn connection_row(ui: &mut Ui, view: &AiSettingsView) -> bool {
    let mut test = false;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_sized([LABEL_W, 30.0], egui::Label::new(label_muted("连接")));
        let label = if view.huniu_testing {
            "测试中…"
        } else {
            "测试连接"
        };
        test = button(ui, label, Btn::Normal, !view.huniu_testing).clicked();
        if let Some((ok, message)) = &view.huniu_connection {
            ui.label(
                RichText::new(message)
                    .size(12.0)
                    .color(if *ok { SUCCESS } else { DANGER }),
            );
        }
    });
    test
}

fn key_color(source: KeySource) -> Color32 {
    match source {
        KeySource::Environment => BRAND,
        KeySource::Keychain => SUCCESS,
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
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_sized([LABEL_W, 24.0], egui::Label::new(label_muted("密钥")));
        ui.label(
            RichText::new(key.unwrap_or("未设置"))
                .size(12.5)
                .color(key_color(source)),
        );
        if source == KeySource::Missing {
            ui.label(RichText::new("未配置").size(12.0).color(DANGER));
        }
    });
    ui.horizontal(|ui| {
        ui.add_sized([LABEL_W, 30.0], egui::Label::new(""));
        ui.add_enabled_ui(source != KeySource::Environment, |ui| {
            with_input_radius(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(entry)
                        .password(true)
                        .desired_width(268.0)
                        .hint_text("输入新密钥")
                        .margin(egui::Margin::symmetric(10, 6)),
                );
            });
            if button(ui, "清除", Btn::Danger, true).clicked() {
                *clear = true;
            }
        });
    });
}

fn draw_body(ui: &mut Ui, view: &mut AiSettingsView) -> bool {
    let mut test = false;
    ui.vertical(|ui| {
        section_card(ui, "模型", "", |ui| {
            model_field(
                ui,
                "chat",
                "语言",
                &mut view.huniu_language_model,
                &view.huniu_models,
                ModelCapability::Language,
                "选择模型",
            );
            model_field(
                ui,
                "image",
                "生图",
                &mut view.huniu_image_model,
                &view.huniu_models,
                ModelCapability::Image,
                "选择模型",
            );
            model_field(
                ui,
                "video",
                "视频",
                &mut view.huniu_video_model,
                &view.huniu_models,
                ModelCapability::Video,
                "选择模型",
            );
            model_field(
                ui,
                "multimodal",
                "多模态",
                &mut view.huniu_multimodal_model,
                &view.huniu_models,
                ModelCapability::Multimodal,
                "选择模型",
            );
            field(ui, "音频", &mut view.volc_audio_model, Some("选择模型"));
        });

        ui.add_space(12.0);
        section_card(ui, "连接", "", |ui| {
            field(ui, "虎妞地址", &mut view.huniu_base_url, Some("https://"));
            field(ui, "虎妞渠道", &mut view.huniu_strategy, None);
            key_card(
                ui,
                view.huniu_key.as_deref(),
                view.huniu_key_source,
                &mut view.huniu_key_entry,
                &mut view.huniu_clear_key,
            );
            ui.add_space(8.0);
            field(ui, "火山地址", &mut view.volc_base_url, Some("https://"));
            field(ui, "默认音色", &mut view.volc_speaker, None);
            key_card(
                ui,
                view.volc_key.as_deref(),
                view.volc_key_source,
                &mut view.volc_key_entry,
                &mut view.volc_clear_key,
            );
            test = connection_row(ui, view);
        });
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
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(560.0);
            // Header
            egui::Frame::NONE
                .fill(PANEL_2)
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(heading("AI 设置"));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("×").size(16.0).color(MUTED))
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::NONE)
                                        .corner_radius(egui::CornerRadius::same(RADIUS)),
                                )
                                .clicked()
                            {
                                set_open(ui, false);
                                action = None;
                            }
                        });
                    });
                });
            ui.painter().hline(
                ui.max_rect().x_range(),
                ui.cursor().top(),
                egui::Stroke::new(1.0_f32, BORDER),
            );

            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(480.0)
                        .auto_shrink([false, true])
                        .id_salt("ai_settings_scroll")
                        .show(ui, |ui| {
                            ui.set_min_width(520.0);
                            if draw_body(ui, view) {
                                action = Some(AiSettingsAction::TestConnection);
                            }
                        });
                });

            // Footer
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
                        if button(ui, "保存", Btn::Primary, true).clicked() {
                            set_open(ui, false);
                            action = Some(AiSettingsAction::Save);
                        }
                        ui.add_space(6.0);
                        if button(ui, "取消", Btn::Ghost, true).clicked() {
                            set_open(ui, false);
                            action = None;
                        }
                    });
                });
        });
    if response.should_close() {
        close(ctx);
        action = None;
    }
    action.unwrap_or(AiSettingsAction::None)
}
