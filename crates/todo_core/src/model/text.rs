//! Styled text ("花字") data: clip content, reusable styles and intro animation.
//!
//! Units: `font_size` is a fraction of the canvas height, like [`crate::Transform`]
//! offsets. Everything inside a style (offsets, stroke width, blur, erosion cell)
//! is in em, so one style scales with any font size.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub const DEFAULT_TEXT_DURATION_SECS: f64 = 3.0;
pub const DEFAULT_FONT_SIZE: f64 = 0.12;
pub const MAX_TEXT_CHARS: usize = 2000;
/// Style that ships with every project that adds text.
pub const DEFAULT_TEXT_STYLE_ID: &str = "grunge-red-echo";

/// Text clip content. The style is referenced from [`crate::Project::text_styles`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
    pub text: String,
    pub style_id: String,
    #[serde(default)]
    pub layout: TextLayout,
    #[serde(default, skip_serializing_if = "TextAnimation::is_none")]
    pub animation: TextAnimation,
}

impl TextContent {
    pub fn new(text: impl Into<String>, style_id: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style_id: style_id.into(),
            layout: TextLayout::default(),
            animation: TextAnimation::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TextLayout {
    /// Glyph em size as a fraction of the canvas height.
    pub font_size: f64,
    /// Extra advance between characters, in em.
    pub letter_spacing: f64,
    /// Baseline-to-baseline distance, in em.
    pub line_height: f64,
    pub align: TextAlign,
}

impl Default for TextLayout {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            letter_spacing: 0.0,
            line_height: 1.2,
            align: TextAlign::Center,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TextAnimation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro: Option<TextIntro>,
}

impl TextAnimation {
    pub fn is_none(&self) -> bool {
        self.intro.is_none()
    }
}

/// Intro played from the clip start. Times are seconds of clip-local time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TextIntro {
    /// The whole text fades from transparent to opaque over `duration`.
    #[serde(rename_all = "camelCase")]
    Fade { duration: f64 },
    /// Characters appear one after another; each fades in over `char_duration`
    /// and the last one is fully visible at `duration`.
    #[serde(rename_all = "camelCase")]
    PerChar { duration: f64, char_duration: f64 },
}

impl TextIntro {
    pub fn duration(&self) -> f64 {
        match *self {
            Self::Fade { duration } | Self::PerChar { duration, .. } => duration,
        }
    }
}

/// Reusable look: font plus layers drawn bottom → top.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub id: String,
    pub name: String,
    pub font: FontRef,
    pub layers: Vec<TextLayer>,
}

/// Font files are names resolved against the renderer's font directories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FontRef {
    pub family: String,
    pub file: String,
    /// Tried in order for characters missing from `file`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<String>,
}

/// One pass over the glyph outlines. A shadow is a blurred, offset fill; a glow
/// is a blurred stroke; an echo is a scaled fill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextLayer {
    #[serde(flatten)]
    pub kind: TextLayerKind,
    pub paint: TextPaint,
    /// Translation in em.
    #[serde(default, skip_serializing_if = "is_zero_offset")]
    pub offset: [f64; 2],
    /// Scale around the text block center.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f64,
    /// Gaussian-like blur radius in em.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub blur: f64,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub erode: Option<TextErode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TextLayerKind {
    Fill,
    /// Outline centered on the glyph edge; `width` in em.
    Stroke {
        width: f64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TextPaint {
    Solid(TextColor),
    #[serde(rename_all = "camelCase")]
    Linear {
        from: TextColor,
        to: TextColor,
        /// Degrees; 0 runs top → bottom, 90 runs left → right.
        #[serde(default)]
        angle: f64,
    },
}

/// Deterministic worn-ink erosion: a seeded value-noise mask removes `amount`
/// of the layer's coverage in cells of `scale` em.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextErode {
    pub amount: f64,
    pub scale: f64,
    #[serde(default)]
    pub seed: u32,
}

/// Straight (non-premultiplied) RGBA, written as `#RRGGBB` or `#RRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextColor(pub [u8; 4]);

impl TextColor {
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        if !hex.is_ascii() || !(hex.len() == 6 || hex.len() == 8) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let alpha = if hex.len() == 8 { byte(6)? } else { 255 };
        Some(Self([byte(0)?, byte(2)?, byte(4)?, alpha]))
    }
}

impl fmt::Display for TextColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.0;
        write!(f, "#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

impl Serialize for TextColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TextColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid color {text:?}")))
    }
}

fn one() -> f64 {
    1.0
}
fn is_one(value: &f64) -> bool {
    *value == 1.0
}
fn is_zero(value: &f64) -> bool {
    *value == 0.0
}
fn is_zero_offset(value: &[f64; 2]) -> bool {
    *value == [0.0, 0.0]
}

/// Animation state at a clip-local time, independent of rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextFrame {
    /// Whole-text opacity multiplier.
    pub opacity: f64,
    /// Per-character reveal: `Some(progress)` where character `i` of `n` has
    /// alpha [`TextFrame::char_alpha`]. `None` means every character is opaque.
    pub reveal: Option<PerCharReveal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerCharReveal {
    pub time: f64,
    pub duration: f64,
    pub char_duration: f64,
}

impl PerCharReveal {
    /// Alpha of visible character `index` among `count` (whitespace excluded).
    pub fn char_alpha(&self, index: usize, count: usize) -> f64 {
        if count == 0 {
            return 1.0;
        }
        let fade = self.char_duration.clamp(0.0, self.duration);
        let step = if count > 1 {
            (self.duration - fade).max(0.0) / (count - 1) as f64
        } else {
            0.0
        };
        let begin = step * index as f64;
        if fade <= 0.0 {
            return if self.time >= begin { 1.0 } else { 0.0 };
        }
        ((self.time - begin) / fade).clamp(0.0, 1.0)
    }
}

impl TextAnimation {
    /// Sample the intro at clip-local `time`. After the intro the frame is static.
    pub fn frame_at(&self, time: f64) -> TextFrame {
        let opaque = TextFrame {
            opacity: 1.0,
            reveal: None,
        };
        let Some(intro) = self.intro else {
            return opaque;
        };
        let duration = intro.duration();
        if !time.is_finite() || !duration.is_finite() || duration <= 0.0 || time >= duration {
            return opaque;
        }
        let time = time.max(0.0);
        match intro {
            TextIntro::Fade { .. } => TextFrame {
                opacity: (time / duration).clamp(0.0, 1.0),
                reveal: None,
            },
            TextIntro::PerChar { char_duration, .. } => TextFrame {
                opacity: 1.0,
                reveal: Some(PerCharReveal {
                    time,
                    duration,
                    char_duration,
                }),
            },
        }
    }
}

/// Stable identity of one rendered text image. Animation settings are excluded:
/// only the sampled per-character `reveal` changes pixels (whole-text opacity is
/// applied when compositing).
pub fn text_raster_digest(
    content: &TextContent,
    style: &TextStyle,
    canvas: (u32, u32),
    reveal: Option<&PerCharReveal>,
) -> String {
    let identity = serde_json::to_vec(&(&content.text, &content.layout, style, canvas, reveal))
        .expect("text data serializes");
    // FNV-1a: stable across runs and Rust versions, unlike `DefaultHasher`.
    let hash = identity
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{hash:016x}")
}

/// Styles available to every project; adding text copies the used style into
/// the project so documents render the same after presets change.
pub fn builtin_text_styles() -> Vec<TextStyle> {
    let color = |hex: &str| TextColor::parse(hex).expect("builtin color");
    let layer = |kind, paint, offset, scale, blur, opacity, erode| TextLayer {
        kind,
        paint,
        offset,
        scale,
        blur,
        opacity,
        erode,
    };
    vec![
        // Reference look (s00_intro1_hold.png): worn grey echo, soft shadow, red brush ink.
        TextStyle {
            id: DEFAULT_TEXT_STYLE_ID.into(),
            name: "做旧红字·灰重影".into(),
            font: FontRef {
                family: "字魂剑气手书".into(),
                file: "zihun-jianqi-shoushu.ttf".into(),
                fallbacks: vec!["msyhbd.ttc".into(), "msyh.ttc".into()],
            },
            layers: vec![
                layer(
                    TextLayerKind::Fill,
                    TextPaint::Solid(color("#474747")),
                    [0.0, 0.0],
                    1.33,
                    0.0,
                    0.92,
                    Some(TextErode {
                        amount: 0.18,
                        scale: 0.035,
                        seed: 7,
                    }),
                ),
                layer(
                    TextLayerKind::Fill,
                    TextPaint::Solid(color("#000000")),
                    [0.035, 0.05],
                    1.0,
                    0.025,
                    0.8,
                    None,
                ),
                layer(
                    TextLayerKind::Fill,
                    TextPaint::Solid(color("#E00A0A")),
                    [0.0, 0.0],
                    1.0,
                    0.0,
                    1.0,
                    Some(TextErode {
                        amount: 0.08,
                        scale: 0.02,
                        seed: 3,
                    }),
                ),
            ],
        },
        TextStyle {
            id: "clean-white-outline".into(),
            name: "白字黑描边".into(),
            font: FontRef {
                family: "微软雅黑".into(),
                file: "msyhbd.ttc".into(),
                fallbacks: vec!["msyh.ttc".into()],
            },
            layers: vec![
                layer(
                    TextLayerKind::Fill,
                    TextPaint::Solid(color("#00000099")),
                    [0.03, 0.04],
                    1.0,
                    0.04,
                    1.0,
                    None,
                ),
                layer(
                    TextLayerKind::Stroke { width: 0.12 },
                    TextPaint::Solid(color("#000000")),
                    [0.0, 0.0],
                    1.0,
                    0.0,
                    1.0,
                    None,
                ),
                layer(
                    TextLayerKind::Fill,
                    TextPaint::Linear {
                        from: color("#FFFFFF"),
                        to: color("#FFE08A"),
                        angle: 0.0,
                    },
                    [0.0, 0.0],
                    1.0,
                    0.0,
                    1.0,
                    None,
                ),
            ],
        },
    ]
}

/// Validation shared by persistence and edits; renderer-independent.
pub(crate) fn text_content_valid(content: &TextContent) -> bool {
    let layout = &content.layout;
    let finite = |value: f64| value.is_finite();
    content.text.chars().count() <= MAX_TEXT_CHARS
        && !content.style_id.is_empty()
        && finite(layout.font_size)
        && layout.font_size > 0.0
        && layout.font_size <= 4.0
        && finite(layout.letter_spacing)
        && (-1.0..=10.0).contains(&layout.letter_spacing)
        && finite(layout.line_height)
        && (0.1..=10.0).contains(&layout.line_height)
        && content.animation.intro.is_none_or(|intro| match intro {
            TextIntro::Fade { duration } => finite(duration) && duration >= 0.0,
            TextIntro::PerChar {
                duration,
                char_duration,
            } => {
                finite(duration) && duration >= 0.0 && finite(char_duration) && char_duration >= 0.0
            }
        })
}

pub(crate) fn text_style_valid(style: &TextStyle) -> bool {
    let finite = |value: f64| value.is_finite();
    !style.id.is_empty()
        && !style.font.file.trim().is_empty()
        && style.layers.len() <= 16
        && style.layers.iter().all(|layer| {
            finite(layer.offset[0])
                && finite(layer.offset[1])
                && layer.offset.iter().all(|v| v.abs() <= 10.0)
                && finite(layer.scale)
                && layer.scale > 0.0
                && layer.scale <= 10.0
                && finite(layer.blur)
                && (0.0..=2.0).contains(&layer.blur)
                && finite(layer.opacity)
                && (0.0..=1.0).contains(&layer.opacity)
                && match layer.kind {
                    TextLayerKind::Fill => true,
                    TextLayerKind::Stroke { width } => finite(width) && width > 0.0 && width <= 2.0,
                }
                && match layer.paint {
                    TextPaint::Solid(_) => true,
                    TextPaint::Linear { angle, .. } => finite(angle),
                }
                && layer.erode.is_none_or(|erode| {
                    finite(erode.amount)
                        && (0.0..=1.0).contains(&erode.amount)
                        && finite(erode.scale)
                        && erode.scale > 0.0
                        && erode.scale <= 10.0
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_roundtrip_and_rejects_garbage() {
        assert_eq!(
            TextColor::parse("#E00A0A"),
            Some(TextColor([224, 10, 10, 255]))
        );
        assert_eq!(
            TextColor::parse("#00000080").unwrap().to_string(),
            "#00000080"
        );
        assert!(TextColor::parse("E00A0A").is_none());
        assert!(TextColor::parse("#E00A0").is_none());
        assert!(TextColor::parse("#GG0000").is_none());
    }

    #[test]
    fn style_json_is_compact_and_roundtrips() {
        let styles = builtin_text_styles();
        let value = serde_json::to_value(&styles[0]).unwrap();
        assert_eq!(value["layers"][0]["kind"], "fill");
        assert_eq!(value["layers"][0]["paint"], "#474747FF");
        assert_eq!(value["layers"][0]["scale"], 1.33);
        assert!(value["layers"][2].get("blur").is_none());
        let back: TextStyle = serde_json::from_value(value).unwrap();
        assert_eq!(back, styles[0]);
        let stroke = serde_json::to_value(&styles[1].layers[1]).unwrap();
        assert_eq!(stroke["kind"], "stroke");
        assert_eq!(stroke["width"], 0.12);
        let gradient = serde_json::to_value(&styles[1].layers[2]).unwrap();
        assert_eq!(gradient["paint"]["from"], "#FFFFFFFF");
        assert!(styles.iter().all(text_style_valid));
    }

    #[test]
    fn intro_json_uses_kind_tags() {
        let content: TextContent = serde_json::from_str(
            r#"{"text":"你好","styleId":"s","animation":{"intro":{"kind":"perChar","duration":1,"charDuration":0.2}}}"#,
        )
        .unwrap();
        assert_eq!(
            content.animation.intro,
            Some(TextIntro::PerChar {
                duration: 1.0,
                char_duration: 0.2
            })
        );
        assert_eq!(content.layout, TextLayout::default());
        let fade = serde_json::to_value(TextIntro::Fade { duration: 0.5 }).unwrap();
        assert_eq!(fade, serde_json::json!({"kind": "fade", "duration": 0.5}));
    }

    #[test]
    fn fade_ramps_then_holds() {
        let animation = TextAnimation {
            intro: Some(TextIntro::Fade { duration: 2.0 }),
        };
        assert_eq!(animation.frame_at(0.0).opacity, 0.0);
        assert!((animation.frame_at(0.5).opacity - 0.25).abs() < 1e-9);
        assert_eq!(animation.frame_at(2.0).opacity, 1.0);
        assert_eq!(animation.frame_at(9.0).reveal, None);
        assert_eq!(TextAnimation::default().frame_at(0.0).opacity, 1.0);
    }

    #[test]
    fn per_char_reveals_in_order_and_finishes_at_duration() {
        let animation = TextAnimation {
            intro: Some(TextIntro::PerChar {
                duration: 1.0,
                char_duration: 0.2,
            }),
        };
        let reveal = animation.frame_at(0.1).reveal.unwrap();
        // Five characters start at 0, .2, .4, .6, .8.
        assert!((reveal.char_alpha(0, 5) - 0.5).abs() < 1e-9);
        assert_eq!(reveal.char_alpha(1, 5), 0.0);
        let late = animation.frame_at(0.9).reveal.unwrap();
        assert_eq!(late.char_alpha(3, 5), 1.0);
        assert!((late.char_alpha(4, 5) - 0.5).abs() < 1e-9);
        assert!(animation.frame_at(1.0).reveal.is_none());
        let instant = PerCharReveal {
            time: 0.5,
            duration: 1.0,
            char_duration: 0.0,
        };
        assert_eq!(instant.char_alpha(0, 3), 1.0);
        assert_eq!(instant.char_alpha(2, 3), 0.0);
    }
}
