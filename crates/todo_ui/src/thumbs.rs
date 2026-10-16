//! Library strip thumbnails. Fetched by the app, painted by the media library.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions, Ui};

#[derive(Clone)]
struct ThumbImage {
    color: Arc<ColorImage>,
}

#[derive(Default)]
pub struct ThumbCache {
    images: HashMap<String, ThumbImage>,
    /// Negative cache so we do not hammer a failing path every frame.
    failed: HashMap<String, ()>,
    textures: HashMap<String, TextureHandle>,
}

impl ThumbCache {
    pub fn contains(&self, path: &str) -> bool {
        self.images.contains_key(path) || self.failed.contains_key(path)
    }

    pub fn ready(&self, path: &str) -> bool {
        self.images.contains_key(path)
    }

    pub fn mark_failed(&mut self, path: String) {
        self.failed.insert(path, ());
    }

    /// Decode a JPEG/PNG thumb body into RGBA and store it.
    pub fn insert_bytes(&mut self, path: String, bytes: &[u8]) -> bool {
        let Ok(dyn_img) = image::load_from_memory(bytes) else {
            self.failed.insert(path, ());
            return false;
        };
        let rgba = dyn_img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        if size[0] == 0 || size[1] == 0 {
            self.failed.insert(path, ());
            return false;
        }
        let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        self.failed.remove(&path);
        self.textures.remove(&path);
        self.images.insert(
            path,
            ThumbImage {
                color: Arc::new(color),
            },
        );
        true
    }

    /// Paint a cover-fit thumbnail into `rect`, or return false to draw a fallback.
    pub fn paint(&mut self, ui: &Ui, path: &str, rect: egui::Rect) -> bool {
        let Some(image) = self.images.get(path).cloned() else {
            return false;
        };
        let texture = self
            .textures
            .entry(path.to_string())
            .or_insert_with(|| {
                ui.ctx().load_texture(
                    format!("todo_thumb:{path}"),
                    (*image.color).clone(),
                    TextureOptions::LINEAR,
                )
            })
            .clone();

        let size = texture.size_vec2();
        if size.x <= 0.0 || size.y <= 0.0 {
            return false;
        }
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        let draw = egui::vec2(size.x * scale, size.y * scale);
        let paint_rect = egui::Rect::from_center_size(rect.center(), draw);
        ui.painter().image(
            texture.id(),
            paint_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        true
    }
}
