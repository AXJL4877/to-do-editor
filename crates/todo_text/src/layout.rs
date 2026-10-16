//! Line breaking on `\n`, per-character font fallback, alignment and centering.
use crate::fonts::lookup;
use todo_core::{TextAlign, TextContent};
use ttf_parser::{Face, GlyphId};

pub(crate) struct PlacedGlyph {
    pub face: usize,
    pub id: GlyphId,
    /// Pen position of the glyph origin in canvas pixels.
    pub x: f32,
    pub baseline: f32,
    /// Order among visible (non-whitespace) characters, for the per-char reveal.
    pub reveal_index: usize,
}

pub(crate) struct Layout {
    pub glyphs: Vec<PlacedGlyph>,
    pub font_px: f32,
    /// Text block center; style scaling happens around this point.
    pub center: (f32, f32),
    /// Block extent: left, top, right, bottom in canvas pixels.
    pub bounds: [f32; 4],
    pub visible_count: usize,
}

struct Advance {
    face: usize,
    id: GlyphId,
    width: f32,
    visible: bool,
}

pub(crate) fn layout(faces: &[Face<'_>], content: &TextContent, canvas: (u32, u32)) -> Layout {
    let font_px = (content.layout.font_size * f64::from(canvas.1)) as f32;
    let spacing = content.layout.letter_spacing as f32 * font_px;
    let line_px = content.layout.line_height as f32 * font_px;
    let scale_of = |face: &Face<'_>| font_px / f32::from(face.units_per_em().max(1));
    let primary = &faces[0];
    let ascent = f32::from(primary.ascender()) * scale_of(primary);
    let descent = -f32::from(primary.descender()) * scale_of(primary);

    let lines: Vec<Vec<Advance>> = content
        .text
        .split('\n')
        .map(|line| {
            line.trim_end_matches('\r')
                .chars()
                .map(|ch| {
                    let (face, id) = lookup(faces, ch);
                    let measured = faces[face]
                        .glyph_hor_advance(id)
                        .map(|units| f32::from(units) * scale_of(&faces[face]));
                    let width = match ch {
                        '\u{3000}' => measured.unwrap_or(font_px),
                        '\t' => font_px * 2.0,
                        _ => measured.unwrap_or(font_px * 0.5),
                    };
                    Advance {
                        face,
                        id,
                        width,
                        visible: !ch.is_whitespace(),
                    }
                })
                .collect()
        })
        .collect();

    let line_width = |line: &[Advance]| {
        line.iter().map(|a| a.width).sum::<f32>() + spacing * line.len().saturating_sub(1) as f32
    };
    let block_width = lines.iter().map(|l| line_width(l)).fold(0.0_f32, f32::max);
    let block_height = line_px * lines.len().saturating_sub(1) as f32 + ascent + descent;
    let center = (canvas.0 as f32 / 2.0, canvas.1 as f32 / 2.0);
    let left = center.0 - block_width / 2.0;
    let top = center.1 - block_height / 2.0;

    let mut glyphs = Vec::new();
    let mut visible_count = 0;
    for (row, line) in lines.iter().enumerate() {
        let width = line_width(line);
        let mut x = match content.layout.align {
            TextAlign::Left => left,
            TextAlign::Center => center.0 - width / 2.0,
            TextAlign::Right => left + block_width - width,
        };
        let baseline = top + ascent + line_px * row as f32;
        for advance in line {
            if advance.visible {
                glyphs.push(PlacedGlyph {
                    face: advance.face,
                    id: advance.id,
                    x,
                    baseline,
                    reveal_index: visible_count,
                });
                visible_count += 1;
            }
            x += advance.width + spacing;
        }
    }
    Layout {
        glyphs,
        font_px,
        center,
        bounds: [left, top, left + block_width, top + block_height],
        visible_count,
    }
}
