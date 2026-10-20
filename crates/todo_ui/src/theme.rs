//! Product colors and egui theme. Chinese glyphs come from a system font when one is present.

use egui::{
    Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, Id, Response,
    RichText, Stroke, Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1b);
pub const PANEL: Color32 = Color32::from_rgb(0x20, 0x22, 0x26);
pub const PANEL_2: Color32 = Color32::from_rgb(0x29, 0x2c, 0x31);
pub const FIELD: Color32 = Color32::from_rgb(0x18, 0x1a, 0x1d);
pub const BORDER: Color32 = Color32::from_rgb(0x34, 0x37, 0x3d);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x49, 0x4e, 0x57);
pub const TEXT: Color32 = Color32::from_rgb(0xdd, 0xe0, 0xe5);
pub const MUTED: Color32 = Color32::from_rgb(0x99, 0xa0, 0xab);
pub const DIM: Color32 = Color32::from_rgb(0x64, 0x6b, 0x76);
pub const BRAND: Color32 = Color32::from_rgb(0x69, 0xb9, 0xe0);
pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0xb4, 0x5b);
pub const VIDEO_LANE: Color32 = Color32::from_rgb(0x20, 0x24, 0x2a);
pub const AUDIO_LANE: Color32 = Color32::from_rgb(0x20, 0x27, 0x27);
pub const TEXT_LANE: Color32 = Color32::from_rgb(0x28, 0x22, 0x2a);
pub const VIDEO_CLIP: Color32 = Color32::from_rgb(0x2a, 0x67, 0x98);
pub const AUDIO_CLIP: Color32 = Color32::from_rgb(0x2b, 0x7c, 0x68);
pub const TEXT_CLIP: Color32 = Color32::from_rgb(0x8a, 0x3f, 0x8f);
pub const TEXT_CLIP_SELECTED: Color32 = Color32::from_rgb(0xb0, 0x5c, 0xb5);
pub const CLIP_TEXT: Color32 = Color32::from_rgb(0xe5, 0xf2, 0xff);
pub const AUDIO_TEXT: Color32 = Color32::from_rgb(0xe4, 0xff, 0xf5);
pub const DANGER: Color32 = Color32::from_rgb(0xf0, 0x76, 0x76);
pub const SUCCESS: Color32 = Color32::from_rgb(0x6e, 0xc4, 0x86);
pub const WARN: Color32 = Color32::from_rgb(0xe0, 0xa8, 0x4c);

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

pub const TOOLBAR_H: f32 = 56.0;
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
    if let Some(bytes) = load_cjk_font() {
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert(
            "cjk".to_owned(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "cjk".to_owned());
        }
        ctx.set_fonts(fonts);
    }
}

fn load_cjk_font() -> Option<Vec<u8>> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
    ];
    CANDIDATES.iter().find_map(|path| std::fs::read(path).ok())
}

fn apply_visuals(ctx: &Context) {
    // The editor is dark only. Without this egui follows a light system
    // theme and draws stock widgets (text fields, combo boxes) light.
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals.dark_mode = true;
    style.visuals.window_fill = BG;
    style.visuals.panel_fill = BG;
    style.visuals.extreme_bg_color = BG;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.window_stroke = Stroke::NONE;
    style.visuals.window_shadow = egui::Shadow::NONE;
    style.visuals.popup_shadow = egui::Shadow::NONE;
    style.visuals.window_corner_radius = CornerRadius::ZERO;
    style.visuals.menu_corner_radius = CornerRadius::ZERO;
    style.visuals.widgets.noninteractive.bg_fill = PANEL;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.inactive.bg_fill = PANEL_2;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.hovered.bg_fill = BORDER;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    style.visuals.widgets.active.bg_fill = BORDER_STRONG;
    style.visuals.selection.bg_fill = Color32::from_rgb(0x2d, 0x60, 0x86);
    style.visuals.widgets.inactive.weak_bg_fill = FIELD;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x15, 0x28, 0x38);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0x1d, 0x3d, 0x55);
    style.visuals.widgets.open.bg_fill = PANEL_2;
    style.visuals.text_cursor.stroke = Stroke::new(1.0_f32, BRAND);
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
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
        Btn::Normal => (Color32::TRANSPARENT, TEXT, Color32::TRANSPARENT, 30.0),
        Btn::Primary => (BRAND, Color32::from_rgb(0x08, 0x17, 0x25), BRAND, 32.0),
        Btn::Ghost => (Color32::TRANSPARENT, MUTED, Color32::TRANSPARENT, 32.0),
        Btn::Danger => (
            Color32::from_rgb(0x2b, 0x18, 0x1e),
            DANGER,
            Color32::from_rgb(0x62, 0x2d, 0x3b),
            32.0,
        ),
        Btn::Tool { active } => {
            let stroke = Color32::TRANSPARENT;
            let text = if active { BRAND } else { MUTED };
            (
                if active {
                    Color32::from_rgb(0x2b, 0x38, 0x41)
                } else {
                    Color32::TRANSPARENT
                },
                text,
                stroke,
                28.0,
            )
        }
    };
    let widget = Button::new(
        RichText::new(label)
            .size(if height < 30.0 { 11.0 } else { 13.0 })
            .color(text),
    )
    .fill(fill)
    .stroke(Stroke::new(1.0_f32, stroke))
    .corner_radius(CornerRadius::ZERO)
    .min_size(Vec2::new(0.0, height));
    let response = ui.add_enabled(enabled, widget);
    if matches!(kind, Btn::Tool { active: true }) || (enabled && response.hovered()) {
        ui.painter().line_segment(
            [response.rect.left_bottom(), response.rect.right_bottom()],
            Stroke::new(
                2.0_f32,
                if matches!(kind, Btn::Tool { active: true }) {
                    BRAND
                } else {
                    BORDER_STRONG
                },
            ),
        );
    }
    response
}
