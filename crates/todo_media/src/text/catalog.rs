//! The font library as the user sees it: every font file in the packaged
//! font folders, what it is, where it comes from and whether its licence is
//! on record (`fonts.json` next to the files), plus preview pictures.
use super::packaged_font_dirs;
use crate::MediaError;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use todo_core::{builtin_text_styles, TextContent, TextStyle};
use todo_text::{family_name, render, FontLibrary, RasterRequest};

/// Font records next to the font files.
pub const FONT_MANIFEST: &str = "fonts.json";

/// Font preview size in pixels.
const PREVIEW: (u32, u32) = (720, 150);

/// One record of `fonts.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontRecord {
    pub file: String,
    pub name: String,
    #[serde(default)]
    pub source: String,
    /// Licence name; empty means the licence is not on record.
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFile {
    pub file: String,
    pub path: PathBuf,
    /// The family the file declares.
    pub family: Option<String>,
    pub record: Option<FontRecord>,
}

/// Every `.ttf` / `.otf` / `.ttc` in the packaged font folders (the first
/// folder wins for a file name, as in rendering), sorted by file name.
pub fn font_catalog(root: &Path) -> (Vec<FontFile>, Vec<String>) {
    let mut fonts: Vec<FontFile> = Vec::new();
    let mut warnings = Vec::new();
    for dir in packaged_font_dirs(root) {
        let records = read_records(&dir, &mut warnings);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let is_font = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"));
            let Some(file) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            if !is_font || fonts.iter().any(|f| f.file == file) {
                continue;
            }
            let family = std::fs::read(&path).ok().and_then(|bytes| family_name(&bytes));
            if family.is_none() {
                warnings.push(format!("字体 {file} 无法读取"));
            }
            fonts.push(FontFile {
                record: records.iter().find(|r| r.file == file).cloned(),
                file,
                path,
                family,
            });
        }
    }
    fonts.sort_by(|a, b| a.file.cmp(&b.file));
    (fonts, warnings)
}

fn read_records(dir: &Path, warnings: &mut Vec<String>) -> Vec<FontRecord> {
    let path = dir.join(FONT_MANIFEST);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        warnings.push(format!("{} 无法解析：{error}", path.display()));
        Vec::new()
    })
}

/// `text` set plainly in `file` (white, thin outline), as PNG bytes.
pub fn font_preview(fonts: &mut FontLibrary, file: &str, text: &str) -> Result<Vec<u8>, MediaError> {
    let mut style = plain_style();
    style.font.file = file.to_owned();
    style.font.fallbacks = vec!["noto-sans-sc-bold.otf".into()];
    preview(fonts, &style, text, 0.4)
}

/// `text` in a text style, exactly as it renders on the canvas, as PNG bytes.
pub fn style_preview(
    fonts: &mut FontLibrary,
    style: &TextStyle,
    text: &str,
) -> Result<Vec<u8>, MediaError> {
    preview(fonts, style, text, 0.5)
}

fn plain_style() -> TextStyle {
    let styles = builtin_text_styles();
    styles
        .iter()
        .find(|s| s.id == "clean-white-outline")
        .or(styles.first())
        .cloned()
        .expect("built-in text styles")
}

fn preview(
    fonts: &mut FontLibrary,
    style: &TextStyle,
    text: &str,
    size: f64,
) -> Result<Vec<u8>, MediaError> {
    let mut content = TextContent::new(text, style.id.clone());
    content.layout.font_size = size;
    let image = render(
        fonts,
        RasterRequest {
            content: &content,
            style,
            canvas: PREVIEW,
            reveal: None,
        },
    )?;
    Ok(image.encode_png()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_name_the_packaged_fonts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (fonts, warnings) = font_catalog(&root);
        assert!(warnings.is_empty(), "{warnings:?}");
        let Some(kuaile) = fonts.iter().find(|f| f.file == "zcool-kuaile.ttf") else {
            eprintln!("skipped: fonts are not installed");
            return;
        };
        assert!(kuaile.family.is_some());
        assert!(kuaile.record.as_ref().is_some_and(|r| r.license.contains("OFL")));
        let mut library = FontLibrary::new(packaged_font_dirs(&root));
        let png = font_preview(&mut library, &kuaile.file, "海绵宝宝 Aa123").unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
