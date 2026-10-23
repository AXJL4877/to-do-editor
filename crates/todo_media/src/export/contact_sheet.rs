//! Stitch stills into one contact sheet (timeline strip for AI review).
use crate::MediaError;
use image::{imageops, Rgba, RgbaImage};
use std::path::Path;

/// Lay out `frames` left-to-right in rows of `columns`, each scaled to
/// `thumb_height` (aspect preserved). Thin 1px gutter between cells.
pub fn stitch_contact_sheet(
    frames: &[impl AsRef<Path>],
    output: &Path,
    thumb_height: u32,
    columns: u32,
) -> Result<(u32, u32), MediaError> {
    if frames.is_empty() {
        return Err(MediaError::Invalid("没有可拼接的帧".into()));
    }
    let thumb_height = thumb_height.clamp(32, 720);
    let columns = columns.clamp(1, 32);
    let gutter = 2u32;

    let mut thumbs = Vec::with_capacity(frames.len());
    let mut cell_w = 0u32;
    for path in frames {
        let path = path.as_ref();
        let img = image::open(path)
            .map_err(|e| MediaError::Invalid(format!("{}: {e}", path.display())))?
            .to_rgba8();
        let (w, h) = (img.width().max(1), img.height().max(1));
        let scaled_w = ((w as f64) * (thumb_height as f64) / (h as f64))
            .round()
            .clamp(1.0, 4096.0) as u32;
        let thumb = imageops::resize(&img, scaled_w, thumb_height, imageops::FilterType::Triangle);
        cell_w = cell_w.max(thumb.width());
        thumbs.push(thumb);
    }

    let rows = ((thumbs.len() as u32) + columns - 1) / columns;
    let sheet_w = columns * cell_w + (columns.saturating_sub(1)) * gutter;
    let sheet_h = rows * thumb_height + (rows.saturating_sub(1)) * gutter;
    let mut sheet = RgbaImage::from_pixel(sheet_w, sheet_h, Rgba([18, 18, 22, 255]));

    for (index, thumb) in thumbs.iter().enumerate() {
        let col = (index as u32) % columns;
        let row = (index as u32) / columns;
        let x = col * (cell_w + gutter) + (cell_w.saturating_sub(thumb.width())) / 2;
        let y = row * (thumb_height + gutter);
        imageops::overlay(&mut sheet, thumb, i64::from(x), i64::from(y));
    }

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| MediaError::io(parent, e))?;
    }
    sheet
        .save(output)
        .map_err(|e| MediaError::Invalid(format!("写入拼接图失败：{e}")))?;
    Ok((sheet_w, sheet_h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn stitches_two_frames_into_one_row() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/contact-sheet-tests")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.png");
        let b = dir.join("b.png");
        RgbaImage::from_pixel(40, 20, Rgba([255, 0, 0, 255]))
            .save(&a)
            .unwrap();
        RgbaImage::from_pixel(20, 20, Rgba([0, 255, 0, 255]))
            .save(&b)
            .unwrap();
        let out = dir.join("sheet.png");
        let (w, h) = stitch_contact_sheet(&[a, b], &out, 40, 8).unwrap();
        assert!(out.is_file());
        assert_eq!(h, 40);
        assert!(w > 40, "two thumbs side by side: {w}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
