//! Product colors and egui theme. Chinese glyphs come from a system font when one is present.
//!
//! Visual direction: commercial NLE / tooling — graphite surfaces, cool cyan accent,
//! sharp rects for chrome; only text fields keep a light radius.

use egui::{
    Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Id,
    Response, RichText, Stroke, Ui, Vec2,
};

// —— Surfaces (dark graphite, slight lift per layer) ——
pub const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x17);
pub const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1d, 0x22);
pub const PANEL_2: Color32 = Color32::from_rgb(0x22, 0x26, 0x2c);
pub const PANEL_3: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x36);
pub const FIELD: Color32 = Color32::from_rgb(0x14, 0x16, 0x1a);
pub const BORDER: Color32 = Color32::from_rgb(0x2e, 0x33, 0x3a);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x45, 0x4c, 0x56);
pub const HAIRLINE: Color32 = Color32::from_rgb(0x25, 0x29, 0x2f);

// —— Type ——
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe9, 0xee);
pub const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa3, 0xb0);
/// Secondary caption (≥4.5:1 on PANEL for WCAG AA body-size text).
pub const DIM: Color32 = Color32::from_rgb(0x7d, 0x85, 0x92);

// —— Accent (cool steel cyan — product, not purple SaaS) ——
pub const BRAND: Color32 = Color32::from_rgb(0x5c, 0xc4, 0xe8);
pub const BRAND_DIM: Color32 = Color32::from_rgb(0x2a, 0x5a, 0x6e);
pub const BRAND_SOFT: Color32 = Color32::from_rgb(0x1a, 0x32, 0x3c);
pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0xb4, 0x5b);

// —— Timeline ——
pub const VIDEO_LANE: Color32 = Color32::from_rgb(0x1c, 0x20, 0x26);
pub const AUDIO_LANE: Color32 = Color32::from_rgb(0x1c, 0x23, 0x23);
pub const TEXT_LANE: Color32 = Color32::from_rgb(0x24, 0x1e, 0x26);
pub const VIDEO_CLIP: Color32 = Color32::from_rgb(0x2a, 0x67, 0x98);
pub const AUDIO_CLIP: Color32 = Color32::from_rgb(0x2b, 0x7c, 0x68);
pub const TEXT_CLIP: Color32 = Color32::from_rgb(0x8a, 0x3f, 0x8f);
pub const TEXT_CLIP_SELECTED: Color32 = Color32::from_rgb(0xb0, 0x5c, 0xb5);
pub const CLIP_TEXT: Color32 = Color32::from_rgb(0xe5, 0xf2, 0xff);
pub const AUDIO_TEXT: Color32 = Color32::from_rgb(0xe4, 0xff, 0xf5);

pub const DANGER: Color32 = Color32::from_rgb(0xf0, 0x76, 0x76);
pub const SUCCESS: Color32 = Color32::from_rgb(0x6e, 0xc4, 0x86);
pub const WARN: Color32 = Color32::from_rgb(0xe0, 0xa8, 0x4c);

/// Structural UI (buttons, cards, modals, menus) stays square.
pub const RADIUS: u8 = 0;
pub const RADIUS_LG: u8 = 0;
/// Text fields / drag values only.
pub const INPUT_RADIUS: u8 = 6;

pub struct Colors {
    pub bg: Color32,
    pub panel: Color32,
    pub accent: Color32,
    pub text: Color32,
    pub clip: Color32,
    pub clip_sel: Color32,
    pub playhead: Color32,
    pub brand: Color32,
}

pub const COLORS: Colors = Colors {
    bg: BG,
    panel: PANEL,
    accent: VIDEO_CLIP,
    text: TEXT,
    clip: VIDEO_CLIP,
    clip_sel: Color32::from_rgb(0x3d, 0x8f, 0xd0),
    playhead: PLAYHEAD,
    brand: BRAND,
};

pub const TOOLBAR_H: f32 = 44.0;
pub const RULER_H: f32 = 28.0;
pub const ROW_H: f32 = 48.0;

const FONT_FLAG: &str = "todo_ui_cjk_font";

pub fn install(ctx: &Context) {
    apply_visuals(ctx);
    let id = Id::new(FONT_FLAG);
    if ctx.data(|data| data.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(id, true));
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = load_ui_font() {
        fonts.font_data.insert(
            "ui".to_owned(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "ui".to_owned());
        }
    }
    if let Some(bytes) = load_cjk_font() {
        fonts.font_data.insert(
            "cjk".to_owned(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            // After Latin UI font so Latin keeps Segoe metrics; CJK fills gaps.
            fonts
                .families
                .entry(family)
                .or_default()
                .push("cjk".to_owned());
        }
    }
    if let Some(bytes) = load_mono_font() {
        fonts.font_data.insert(
            "mono".to_owned(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "mono".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn load_ui_font() -> Option<Vec<u8>> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\SegoeUI.ttf",
        r"C:\Windows\Fonts\calibri.ttf",
    ];
    CANDIDATES.iter().find_map(|path| std::fs::read(path).ok())
}

fn load_cjk_font() -> Option<Vec<u8>> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
    ];
    CANDIDATES.iter().find_map(|path| std::fs::read(path).ok())
}

fn load_mono_font() -> Option<Vec<u8>> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\consola.ttf",
        r"C:\Windows\Fonts\cascadiamono.ttf",
        r"C:\Windows\Fonts\CascadiaMono.ttf",
    ];
    CANDIDATES.iter().find_map(|path| std::fs::read(path).ok())
}

fn apply_visuals(ctx: &Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals.dark_mode = true;
    style.visuals.window_fill = PANEL;
    style.visuals.panel_fill = BG;
    style.visuals.extreme_bg_color = FIELD;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.window_shadow = egui::Shadow {
        offset: [0, 12],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    style.visuals.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    style.visuals.window_corner_radius = CornerRadius::ZERO;
    style.visuals.menu_corner_radius = CornerRadius::ZERO;
    style.visuals.widgets.noninteractive.bg_fill = PANEL;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.inactive.bg_fill = PANEL_2;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.hovered.bg_fill = PANEL_3;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    style.visuals.widgets.active.bg_fill = BORDER_STRONG;
    style.visuals.selection.bg_fill = BRAND_DIM;
    style.visuals.widgets.inactive.weak_bg_fill = FIELD;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x18, 0x28, 0x32);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0x1e, 0x3a, 0x48);
    style.visuals.widgets.open.bg_fill = PANEL_2;
    style.visuals.text_cursor.stroke = Stroke::new(1.5_f32, BRAND);
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(0);
    style.spacing.indent = 16.0;
    // Square chrome by default; text fields temporarily opt into INPUT_RADIUS.
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::ZERO;
    }
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// Run `add` with input-field corner radius (TextEdit / framed fields).
pub fn with_input_radius<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = CornerRadius::same(INPUT_RADIUS);
    let prev = {
        let w = &ui.style().visuals.widgets;
        (
            w.inactive.corner_radius,
            w.hovered.corner_radius,
            w.active.corner_radius,
        )
    };
    {
        let w = &mut ui.style_mut().visuals.widgets;
        w.inactive.corner_radius = r;
        w.hovered.corner_radius = r;
        w.active.corner_radius = r;
    }
    let out = add(ui);
    {
        let w = &mut ui.style_mut().visuals.widgets;
        w.inactive.corner_radius = prev.0;
        w.hovered.corner_radius = prev.1;
        w.active.corner_radius = prev.2;
    }
    out
}

#[derive(Clone, Copy)]
pub enum Btn {
    Normal,
    Primary,
    Ghost,
    Danger,
    Tool { active: bool },
}

pub fn button(ui: &mut Ui, label: &str, kind: Btn, enabled: bool) -> Response {
    let (fill, text, stroke, height) = match kind {
        Btn::Normal => (PANEL_2, TEXT, BORDER, 30.0),
        Btn::Primary => (BRAND, Color32::from_rgb(0x08, 0x14, 0x1c), BRAND, 32.0),
        Btn::Ghost => (Color32::TRANSPARENT, MUTED, Color32::TRANSPARENT, 30.0),
        Btn::Danger => (
            Color32::from_rgb(0x2b, 0x18, 0x1e),
            DANGER,
            Color32::from_rgb(0x62, 0x2d, 0x3b),
            30.0,
        ),
        Btn::Tool { active } => {
            let text = if active { BRAND } else { MUTED };
            (
                if active { BRAND_SOFT } else { Color32::TRANSPARENT },
                text,
                if active { BRAND_DIM } else { Color32::TRANSPARENT },
                28.0,
            )
        }
    };
    let widget = Button::new(
        RichText::new(label)
            .size(if height < 30.0 { 12.0 } else { 13.0 })
            .color(text),
    )
    .fill(fill)
    .stroke(Stroke::new(1.0_f32, stroke))
    .corner_radius(CornerRadius::ZERO)
    .min_size(Vec2::new(0.0, height));
    let response = ui.add_enabled(enabled, widget);
    if matches!(kind, Btn::Tool { active: true }) {
        ui.painter().line_segment(
            [response.rect.left_bottom(), response.rect.right_bottom()],
            Stroke::new(2.0_f32, BRAND),
        );
    }
    response
}

/// Modal / dialog shell: header + optional footer; body via `add`.
pub fn modal_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER_STRONG))
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(0.0)
}

pub fn section_card(ui: &mut Ui, title: &str, subtitle: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .fill(PANEL_2)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .inner_margin(egui::Margin::symmetric(16, 14))
        .corner_radius(CornerRadius::ZERO)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong().size(14.0).color(TEXT));
                if !subtitle.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(subtitle).size(11.5).color(MUTED));
                }
            });
            ui.add_space(12.0);
            add(ui);
        });
}

pub fn hairline(ui: &mut Ui) {
    let rect = ui.max_rect();
    let y = ui.cursor().top();
    ui.painter()
        .hline(rect.x_range(), y, Stroke::new(1.0_f32, HAIRLINE));
    ui.add_space(1.0);
}

pub fn label_muted(text: &str) -> RichText {
    RichText::new(text).size(12.0).color(MUTED)
}

pub fn heading(text: &str) -> RichText {
    RichText::new(text).strong().size(18.0).color(TEXT)
}

pub fn caption(text: &str) -> RichText {
    RichText::new(text).size(11.0).color(DIM)
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}
