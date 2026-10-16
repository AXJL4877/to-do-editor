//! Glyph outlines → style layers → canvas.
use crate::effects::{blur, erosion_keep, erosion_noise};
use crate::layout::{layout, Layout};
use crate::{FontLibrary, RasterRequest, TextError, MAX_CANVAS_EDGE};
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, Point, Rect, Shader, SpreadMode, Stroke, Transform,
};
use todo_core::{TextColor, TextLayer, TextLayerKind, TextPaint};
use ttf_parser::{Face, OutlineBuilder};

/// Laid-out text block as canvas fractions, before style effects and the clip
/// transform. Always centered on the canvas; editors use it as the element box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBlock {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl TextBlock {
    fn from_layout(placed: &Layout, canvas: (u32, u32)) -> Self {
        let [left, top, right, bottom] = placed.bounds;
        let (w, h) = (f64::from(canvas.0), f64::from(canvas.1));
        Self {
            left: f64::from(left) / w,
            top: f64::from(top) / h,
            right: f64::from(right) / w,
            bottom: f64::from(bottom) / h,
        }
    }
}

/// Transparent canvas-sized raster, premultiplied RGBA8.
pub struct TextImage {
    pixmap: Pixmap,
    block: TextBlock,
}

impl TextImage {
    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub fn block(&self) -> TextBlock {
        self.block
    }

    pub fn premultiplied_rgba(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// Straight-alpha RGBA at `(x, y)`.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let c = self.pixmap.pixel(x, y)?.demultiply();
        Some([c.red(), c.green(), c.blue(), c.alpha()])
    }

    /// Straight-alpha RGBA PNG. Rasters are a local cache, so encoding speed
    /// matters more than file size.
    pub fn encode_png(&self) -> Result<Vec<u8>, TextError> {
        let error = |e: png::EncodingError| TextError::Encode(e.to_string());
        let straight: Vec<u8> = self
            .pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let c = pixel.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width(), self.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fastest);
        let mut writer = encoder.write_header().map_err(error)?;
        writer.write_image_data(&straight).map_err(error)?;
        writer.finish().map_err(error)?;
        Ok(out)
    }
}

struct GlyphPath {
    path: Path,
    reveal_index: usize,
}

/// Render one request. Only the per-character reveal is animated here; whole-text
/// opacity belongs to the compositor.
pub fn render(fonts: &mut FontLibrary, request: RasterRequest<'_>) -> Result<TextImage, TextError> {
    let (width, height) = check_canvas(request.canvas)?;
    let mut canvas = Pixmap::new(width, height).ok_or(TextError::InvalidCanvas(width, height))?;
    let chain = fonts.chain(&request.style.font.file, &request.style.font.fallbacks)?;
    let faces = chain.faces();
    let placed = layout(&faces, request.content, request.canvas);
    let block = TextBlock::from_layout(&placed, request.canvas);
    let paths = glyph_paths(&faces, &placed);
    if paths.is_empty() {
        return Ok(TextImage {
            pixmap: canvas,
            block,
        });
    }
    let alphas: Vec<f32> = paths
        .iter()
        .map(|glyph| {
            request.reveal.map_or(1.0, |reveal| {
                reveal.char_alpha(glyph.reveal_index, placed.visible_count) as f32
            })
        })
        .collect();
    for layer in &request.style.layers {
        draw_layer(&mut canvas, layer, &placed, &paths, &alphas);
    }
    Ok(TextImage {
        pixmap: canvas,
        block,
    })
}

/// The text block without rasterizing; cheap enough for cache hits.
pub fn measure(
    fonts: &mut FontLibrary,
    request: RasterRequest<'_>,
) -> Result<TextBlock, TextError> {
    check_canvas(request.canvas)?;
    let chain = fonts.chain(&request.style.font.file, &request.style.font.fallbacks)?;
    let faces = chain.faces();
    let placed = layout(&faces, request.content, request.canvas);
    Ok(TextBlock::from_layout(&placed, request.canvas))
}

fn check_canvas((width, height): (u32, u32)) -> Result<(u32, u32), TextError> {
    if width == 0 || height == 0 || width > MAX_CANVAS_EDGE || height > MAX_CANVAS_EDGE {
        return Err(TextError::InvalidCanvas(width, height));
    }
    Ok((width, height))
}

fn glyph_paths(faces: &[Face<'_>], placed: &Layout) -> Vec<GlyphPath> {
    placed
        .glyphs
        .iter()
        .filter_map(|glyph| {
            let face = &faces[glyph.face];
            let scale = placed.font_px / f32::from(face.units_per_em().max(1));
            let mut builder = Outline {
                path: PathBuilder::new(),
                scale,
                x: glyph.x,
                y: glyph.baseline,
            };
            face.outline_glyph(glyph.id, &mut builder)?;
            Some(GlyphPath {
                path: builder.path.finish()?,
                reveal_index: glyph.reveal_index,
            })
        })
        .collect()
}

struct Outline {
    path: PathBuilder,
    scale: f32,
    x: f32,
    y: f32,
}

impl Outline {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.y - y * self.scale)
    }
}

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.path.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.path.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x, y) = self.map(x, y);
        self.path.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x2, y2) = self.map(x2, y2);
        let (x, y) = self.map(x, y);
        self.path.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.path.close();
    }
}

/// Draw one style layer into its own tight buffer, apply erosion and blur, then
/// composite it with the layer opacity.
fn draw_layer(
    canvas: &mut Pixmap,
    layer: &TextLayer,
    placed: &Layout,
    paths: &[GlyphPath],
    alphas: &[f32],
) {
    let em = placed.font_px;
    let scale = layer.scale as f32;
    let (cx, cy) = placed.center;
    let dx = layer.offset[0] as f32 * em;
    let dy = layer.offset[1] as f32 * em;
    // Scale around the block center, then offset.
    let placement = Transform::from_translate(-cx, -cy)
        .post_scale(scale, scale)
        .post_translate(cx + dx, cy + dy);
    let stroke_px = match layer.kind {
        TextLayerKind::Fill => 0.0,
        TextLayerKind::Stroke { width } => width as f32 * em,
    };
    let blur_px = layer.blur as f32 * em;
    let margin = stroke_px * scale / 2.0 + blur_px * 3.0 + 2.0;

    let Some(bounds) = paths
        .iter()
        .filter_map(|glyph| glyph.path.clone().transform(placement))
        .map(|path| path.bounds())
        .reduce(|a, b| {
            Rect::from_ltrb(
                a.left().min(b.left()),
                a.top().min(b.top()),
                a.right().max(b.right()),
                a.bottom().max(b.bottom()),
            )
            .unwrap_or(a)
        })
    else {
        return;
    };
    let left = ((bounds.left() - margin).floor() as i32).max(0);
    let top = ((bounds.top() - margin).floor() as i32).max(0);
    let right = ((bounds.right() + margin).ceil() as i32).min(canvas.width() as i32);
    let bottom = ((bounds.bottom() + margin).ceil() as i32).min(canvas.height() as i32);
    if right <= left || bottom <= top {
        return;
    }
    let Some(mut buffer) = Pixmap::new((right - left) as u32, (bottom - top) as u32) else {
        return;
    };
    let local = placement.post_translate(-left as f32, -top as f32);
    let stroke = Stroke {
        width: stroke_px.max(0.01),
        line_join: LineJoin::Round,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    for (glyph, &alpha) in paths.iter().zip(alphas) {
        if alpha <= 0.0 {
            continue;
        }
        let Some(shader) = shader(layer.paint, placed, alpha) else {
            continue;
        };
        let paint = Paint {
            shader,
            anti_alias: true,
            ..Paint::default()
        };
        match layer.kind {
            TextLayerKind::Fill => {
                buffer.fill_path(&glyph.path, &paint, FillRule::Winding, local, None)
            }
            TextLayerKind::Stroke { .. } => {
                buffer.stroke_path(&glyph.path, &paint, &stroke, local, None)
            }
        }
    }
    if let Some(erode) = layer.erode {
        // Noise lives in unscaled text space so the pattern follows the layer.
        let cell = (erode.scale as f32 * em).max(0.5);
        let width = buffer.width() as usize;
        let data = buffer.data_mut();
        for (index, pixel) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if pixel[3] == 0 {
                continue;
            }
            let x = (index % width) as f32 + left as f32 + 0.5;
            let y = (index / width) as f32 + top as f32 + 0.5;
            let ux = (x - cx - dx) / scale + cx;
            let uy = (y - cy - dy) / scale + cy;
            let keep = erosion_keep(
                erosion_noise(ux / cell, uy / cell, erode.seed),
                erode.amount as f32,
            );
            if keep < 1.0 {
                for channel in pixel.iter_mut() {
                    *channel = (f32::from(*channel) * keep).round() as u8;
                }
            }
        }
    }
    let (w, h) = (buffer.width() as usize, buffer.height() as usize);
    blur(buffer.data_mut(), w, h, blur_px);
    canvas.draw_pixmap(
        left,
        top,
        buffer.as_ref(),
        &PixmapPaint {
            opacity: layer.opacity as f32,
            ..PixmapPaint::default()
        },
        Transform::identity(),
        None,
    );
}

fn color(value: TextColor, alpha: f32) -> Color {
    let [r, g, b, a] = value.0;
    Color::from_rgba8(r, g, b, (f32::from(a) * alpha).round() as u8)
}

/// Paint in untransformed layout space; the layer transform moves it with the text.
fn shader(paint: TextPaint, placed: &Layout, alpha: f32) -> Option<Shader<'static>> {
    match paint {
        TextPaint::Solid(value) => Some(Shader::SolidColor(color(value, alpha))),
        TextPaint::Linear { from, to, angle } => {
            let [left, top, right, bottom] = placed.bounds;
            let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
            let radians = (angle as f32).to_radians();
            let (dx, dy) = (radians.sin(), radians.cos());
            let extent =
                (((right - left) / 2.0) * dx.abs() + ((bottom - top) / 2.0) * dy.abs()).max(0.5);
            LinearGradient::new(
                Point::from_xy(cx - dx * extent, cy - dy * extent),
                Point::from_xy(cx + dx * extent, cy + dy * extent),
                vec![
                    GradientStop::new(0.0, color(from, alpha)),
                    GradientStop::new(1.0, color(to, alpha)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
    }
}
