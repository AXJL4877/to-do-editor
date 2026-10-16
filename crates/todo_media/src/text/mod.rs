//! Text rasters: rendered by `todo_text`, cached as PNG under the request digest.
//! Owned by one worker thread because the font library keeps loaded fonts.
use crate::{cache::MediaCache, MediaError};
use std::path::{Path, PathBuf};
use todo_core::TransformBounds;
use todo_text::{measure, render, FontLibrary, RasterRequest, TextBlock};

/// Bumped when rasterizer output changes so stale cached PNGs are not reused.
const RENDER_VERSION: u32 = 1;

pub struct TextRasterizer {
    fonts: FontLibrary,
    cache: MediaCache,
}

impl TextRasterizer {
    pub fn new(root: &Path) -> Self {
        Self::with_font_dirs(root, font_dirs(root))
    }

    pub fn with_font_dirs(root: &Path, dirs: Vec<PathBuf>) -> Self {
        Self {
            fonts: FontLibrary::new(dirs),
            cache: MediaCache::new(root.join("cache/media")),
        }
    }

    pub fn font_dirs(&self) -> &[PathBuf] {
        self.fonts.dirs()
    }

    /// Render `request` unless a raster for `digest` is already cached; the text
    /// block is measured either way. `digest` must identify every input of the
    /// request (hex, 8–32 chars).
    pub fn raster(
        &mut self,
        digest: &str,
        request: RasterRequest<'_>,
    ) -> Result<(PathBuf, TransformBounds), MediaError> {
        if !(8..=32).contains(&digest.len()) || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(MediaError::Invalid(format!("text raster key {digest:?}")));
        }
        let key = format!("text-v{RENDER_VERSION}{}", digest.to_ascii_lowercase());
        let path = self.cache.path(&key, "png");
        if path.is_file() {
            return Ok((path, bounds(measure(&mut self.fonts, request)?)));
        }
        let image = render(&mut self.fonts, request)?;
        self.cache.write(&path, &image.encode_png()?);
        if path.is_file() {
            Ok((path, bounds(image.block())))
        } else {
            Err(MediaError::Invalid(format!(
                "cannot write {}",
                path.display()
            )))
        }
    }
}

fn bounds(block: TextBlock) -> TransformBounds {
    TransformBounds {
        left: block.left,
        top: block.top,
        right: block.right,
        bottom: block.bottom,
    }
}

/// `TODO_FONTS_DIR` (`;`-separated), the packaged `dependencies/fonts`, then the
/// Windows font folder for fallbacks such as Microsoft YaHei.
pub fn font_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var("TODO_FONTS_DIR")
        .ok()
        .map(|raw| {
            raw.split(';')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    dirs.push(root.join("dependencies").join("fonts"));
    let windows =
        std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    dirs.push(windows.join("Fonts"));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::{builtin_text_styles, TextContent};

    fn request_parts() -> (TextContent, todo_core::TextStyle) {
        let style = builtin_text_styles().remove(1);
        (TextContent::new("测试", style.id.clone()), style)
    }

    #[test]
    fn caches_by_digest_and_rejects_unsafe_keys() {
        let root = std::env::temp_dir().join(format!("todo_text_raster_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut rasterizer = TextRasterizer::new(&root);
        let (content, style) = request_parts();
        let request = RasterRequest {
            content: &content,
            style: &style,
            canvas: (320, 180),
            reveal: None,
        };
        assert!(matches!(
            rasterizer.raster("../../evil", request),
            Err(MediaError::Invalid(_))
        ));
        if rasterizer.fonts.locate(&style.font.file).is_none() {
            eprintln!("skipped: {} unavailable", style.font.file);
            return;
        }
        let (path, block) = rasterizer.raster("00ff00ff00ff00ff", request).unwrap();
        assert!(block.left > 0.0 && block.right < 1.0 && block.left < block.right);
        assert!(
            (block.left + block.right - 1.0).abs() < 1e-6,
            "block is centered"
        );
        assert!(path.starts_with(root.join("cache/media/text")));
        let written = std::fs::metadata(&path).unwrap().modified().unwrap();
        // Same digest: served from cache, file untouched.
        assert_eq!(
            rasterizer.raster("00ff00ff00ff00ff", request).unwrap(),
            (path.clone(), block)
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            written
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
