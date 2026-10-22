//! Library strip thumbnails. Fetched by the app, painted by the media library.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions, Ui};

const MAX_THUMBS: usize = 256;
const MAX_THUMB_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
struct ThumbImage {
    color: Arc<ColorImage>,
    used: u64,
}

#[derive(Default)]
pub struct ThumbCache {
    images: HashMap<String, ThumbImage>,
    /// Negative cache so we do not hammer a failing path every frame.
    failed: HashMap<String, ()>,
    textures: HashMap<String, TextureHandle>,
    clock: u64,
    image_bytes: usize,
}

impl ThumbCache {
    pub fn contains(&self, path: &str) -> bool {
        self.images.contains_key(path) || self.failed.contains_key(path)
    }

    pub fn ready(&self, path: &str) -> bool {
        self.images.contains_key(path)
    }

    pub fn mark_failed(&mut self, path: String) {
        if self.failed.len() >= MAX_THUMBS {
            self.failed.clear();
        }
        self.failed.insert(path, ());
    }

    /// Decode a JPEG/PNG thumb body into RGBA and store it.
    pub fn insert_bytes(&mut self, path: String, bytes: &[u8]) -> bool {
        let Ok(dyn_img) = image::load_from_memory(bytes) else {
            self.mark_failed(path);
            return false;
        };
        let dyn_img = if dyn_img.width() > 512 || dyn_img.height() > 512 {
            dyn_img.thumbnail(512, 512)
        } else {
            dyn_img
        };
        let rgba = dyn_img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        if size[0] == 0 || size[1] == 0 {
            self.mark_failed(path);
            return false;
        }
        let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        self.failed.remove(&path);
        self.textures.remove(&path);
        if let Some(old) = self.images.remove(&path) {
            self.image_bytes -= old.color.pixels.len() * 4;
        }
        let bytes = color.pixels.len() * 4;
        while self.images.len() >= MAX_THUMBS || self.image_bytes + bytes > MAX_THUMB_BYTES {
            let Some(oldest) = self.images.iter()
                .min_by_key(|(_, image)| image.used)
                .map(|(path, _)| path.clone()) else { break; };
            if let Some(old) = self.images.remove(&oldest) {
                self.image_bytes -= old.color.pixels.len() * 4;
            }
            // Dropping the handle also frees its GPU texture.
            self.textures.remove(&oldest);
        }
        self.clock += 1;
        self.image_bytes += bytes;
        self.images.insert(
            path,
            ThumbImage {
                color: Arc::new(color),
                used: self.clock,
            },
        );
        true
    }

    /// Paint a cover-fit thumbnail into `rect`, or return false to draw a fallback.
    pub fn paint(&mut self, ui: &Ui, path: &str, rect: egui::Rect) -> bool {
        let Some(image) = self.images.get_mut(path) else {
            return false;
        };
        // Off-screen inspector rows must not upload or retain GPU textures.
        if !ui.is_rect_visible(rect) {
            return true;
        }
        self.clock += 1;
        image.used = self.clock;
        let image = image.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_many_relay_images_keeps_cache_bounded() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(8, 8)
            .write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let mut cache = ThumbCache::default();
        for i in 0..300 {
            assert!(cache.insert_bytes(format!("image-{i}"), bytes.get_ref()));
        }
        assert!(cache.ready("image-299"));
        assert!(!cache.ready("image-0"), "old images are retained forever");
        assert!(cache.images.len() <= 256);
    }
}
