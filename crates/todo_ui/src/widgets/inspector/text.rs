//! Text clip fields: content, style, layout and intro. Every change is a full
//! `SetClipText`; typing and dragging are grouped into one undo step each.
use crate::theme::MUTED;
use crate::widgets::controls::edit_response;
use egui::Ui;
use todo_interaction::{Intent, TextAlign, TextContent, TextIntro, TextProperties};

const DEFAULT_FADE_SECS: f64 = 0.6;
const DEFAULT_PER_CHAR_SECS: f64 = 1.2;
const DEFAULT_CHAR_FADE_SECS: f64 = 0.25;

pub(super) fn draw(ui: &mut Ui, clip_id: &str, text: &TextProperties) -> Vec<Intent> {
    let mut intents = Vec::new();
    let content = &text.content;
    let set = |content: TextContent| Intent::SetClipText {
        clip_id: clip_id.to_owned(),
        content,
    };

    ui.label(egui::RichText::new("文字").color(MUTED));
    let mut value = content.text.clone();
    let response = crate::theme::with_input_radius(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut value)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("输入文字，回车换行"),
        )
    });
    let change = (response.changed() && value != content.text).then(|| {
        let mut next = content.clone();
        next.text = value;
        set(next)
    });
    edit_response(ui, &response, change, &mut intents);

    egui::Grid::new("text_fields")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 10.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new("花字").color(MUTED));
            let current = text
                .styles
                .iter()
                .find(|style| style.id == content.style_id)
                .map_or(content.style_id.as_str(), |style| style.name.as_str());
            egui::ComboBox::from_id_salt("text_style")
                .selected_text(current)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for style in &text.styles {
                        let selected = style.id == content.style_id;
                        if ui.selectable_label(selected, &style.name).clicked() && !selected {
                            let mut next = content.clone();
                            next.style_id = style.id.clone();
                            intents.push(set(next));
                        }
                    }
                });
            ui.end_row();

            ui.label(egui::RichText::new("字号").color(MUTED));
            let mut percent = content.layout.font_size * 100.0;
            let response = ui
                .add(
                    egui::DragValue::new(&mut percent)
                        .speed(0.2)
                        .fixed_decimals(1)
                        .range(1.0..=100.0)
                        .suffix(" %"),
                )
                .on_hover_text("相对画布高度");
            let change = response.changed().then(|| {
                let mut next = content.clone();
                next.layout.font_size = percent / 100.0;
                set(next)
            });
            edit_response(ui, &response, change, &mut intents);
            ui.end_row();

            ui.label(egui::RichText::new("字距").color(MUTED));
            let mut spacing = content.layout.letter_spacing;
            let response = ui
                .add(
                    egui::DragValue::new(&mut spacing)
                        .speed(0.005)
                        .fixed_decimals(2)
                        .range(-0.5..=2.0)
                        .suffix(" em"),
                )
                .on_hover_text("字符间距");
            let change = response.changed().then(|| {
                let mut next = content.clone();
                next.layout.letter_spacing = spacing;
                set(next)
            });
            edit_response(ui, &response, change, &mut intents);
            ui.end_row();

            ui.label(egui::RichText::new("对齐").color(MUTED));
            ui.horizontal(|ui| {
                for (align, label) in [
                    (TextAlign::Left, "左"),
                    (TextAlign::Center, "中"),
                    (TextAlign::Right, "右"),
                ] {
                    let selected = content.layout.align == align;
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        let mut next = content.clone();
                        next.layout.align = align;
                        intents.push(set(next));
                    }
                }
            });
            ui.end_row();

            ui.label(egui::RichText::new("入场").color(MUTED));
            let intro = content.animation.intro;
            let label = match intro {
                None => "无",
                Some(TextIntro::Fade { .. }) => "淡入",
                Some(TextIntro::PerChar { .. }) => "逐字出现",
            };
            egui::ComboBox::from_id_salt("text_intro")
                .selected_text(label)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (option, name) in [
                        (None, "无"),
                        (
                            Some(TextIntro::Fade {
                                duration: DEFAULT_FADE_SECS,
                            }),
                            "淡入",
                        ),
                        (
                            Some(TextIntro::PerChar {
                                duration: DEFAULT_PER_CHAR_SECS,
                                char_duration: DEFAULT_CHAR_FADE_SECS,
                            }),
                            "逐字出现",
                        ),
                    ] {
                        let selected = option.map(kind) == intro.map(kind);
                        if ui.selectable_label(selected, name).clicked() && !selected {
                            let mut next = content.clone();
                            next.animation.intro = option;
                            intents.push(set(next));
                        }
                    }
                });
            ui.end_row();

            if let Some(intro) = intro {
                ui.label(egui::RichText::new("入场时长").color(MUTED));
                let mut duration = intro.duration();
                let response = ui.add(
                    egui::DragValue::new(&mut duration)
                        .speed(0.02)
                        .fixed_decimals(2)
                        .range(0.0..=30.0)
                        .suffix(" 秒"),
                );
                let change = response.changed().then(|| {
                    let mut next = content.clone();
                    next.animation.intro = Some(match intro {
                        TextIntro::Fade { .. } => TextIntro::Fade { duration },
                        TextIntro::PerChar { char_duration, .. } => TextIntro::PerChar {
                            duration,
                            char_duration: char_duration.min(duration),
                        },
                    });
                    set(next)
                });
                edit_response(ui, &response, change, &mut intents);
                ui.end_row();
            }
            if let Some(TextIntro::PerChar {
                duration,
                char_duration,
            }) = intro
            {
                ui.label(egui::RichText::new("单字渐显").color(MUTED));
                let mut fade = char_duration;
                let response = ui
                    .add(
                        egui::DragValue::new(&mut fade)
                            .speed(0.01)
                            .fixed_decimals(2)
                            .range(0.0..=duration)
                            .suffix(" 秒"),
                    )
                    .on_hover_text("每个字从透明到完全显示的时长");
                let change = response.changed().then(|| {
                    let mut next = content.clone();
                    next.animation.intro = Some(TextIntro::PerChar {
                        duration,
                        char_duration: fade,
                    });
                    set(next)
                });
                edit_response(ui, &response, change, &mut intents);
                ui.end_row();
            }
        });
    intents
}

#[derive(PartialEq, Eq)]
enum IntroKind {
    Fade,
    PerChar,
}

fn kind(intro: TextIntro) -> IntroKind {
    match intro {
        TextIntro::Fade { .. } => IntroKind::Fade,
        TextIntro::PerChar { .. } => IntroKind::PerChar,
    }
}
