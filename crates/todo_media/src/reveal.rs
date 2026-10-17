//! Read/write the `todo-reveal/1` stacked PNG metadata format.
use crate::MediaError;
use image::{ImageBuffer, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub const REVEAL_SPEC: &str = "todo-reveal/1";

/// Compose one reveal frame without decoding another image.  The timing image
/// stores a 16-bit first-visible time in R/G, a soft edge in B, and a
/// participation bit in A.  This CPU implementation is also the reference
/// used by the native shader and by export tests.
pub fn compose_reveal(
    color: &RgbaImage,
    timing: &RgbaImage,
    meta: &RevealMeta,
    time: f64,
) -> Result<RgbaImage, MediaError> {
    let duration = meta.duration;
    if color.dimensions() != timing.dimensions()
        || !duration.is_finite()
        || duration <= 0.0
        || !time.is_finite()
    {
        return Err(MediaError::Invalid(
            "reveal frame inputs are invalid".into(),
        ));
    }
    let mut output = color.clone();
    for (dst, timing) in output.pixels_mut().zip(timing.pixels()) {
        let reveal = (u16::from(timing[0]) << 8 | u16::from(timing[1])) as f64 / 65535.0 * duration;
        let soft = f64::from(timing[2]) / 255.0 * meta.soft_max;
        let visibility = if timing[3] == 0 {
            0.0
        } else if time >= reveal {
            1.0
        } else if soft <= f64::EPSILON || time <= reveal - soft {
            0.0
        } else {
            let u = ((time - (reveal - soft)) / soft).clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        };
        dst[3] = (f64::from(dst[3]) * visibility).round().clamp(0.0, 255.0) as u8;
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevealMeta {
    pub spec: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub soft_max: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand: Option<HandMeta>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandMeta {
    pub file: String,
    pub anchor: [f64; 2],
    pub height: f64,
    pub path: Vec<[f64; 3]>,
}

pub fn write_reveal(
    path: &Path,
    color: &RgbaImage,
    timing: &RgbaImage,
    mut meta: RevealMeta,
) -> Result<(), MediaError> {
    let (width, height) = color.dimensions();
    if timing.dimensions() != (width, height) || width == 0 || height == 0 {
        return Err(MediaError::Invalid(
            "reveal image dimensions do not match".into(),
        ));
    }
    if !meta.duration.is_finite()
        || meta.duration <= 0.0
        || !meta.soft_max.is_finite()
        || meta.soft_max < 0.0
    {
        return Err(MediaError::Invalid(
            "reveal duration/softMax is invalid".into(),
        ));
    }
    meta.spec = REVEAL_SPEC.into();
    meta.width = width;
    meta.height = height;
    let mut pixels = Vec::with_capacity((width * 2 * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&color.get_pixel(x, y).0);
        }
        for x in 0..width {
            pixels.extend_from_slice(&timing.get_pixel(x, y).0);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| MediaError::io(parent, e))?;
    }
    let file = File::create(path).map_err(|e| MediaError::io(path, e))?;
    let mut encoder = png::Encoder::new(file, width * 2, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| MediaError::Invalid(e.to_string()))?;
    writer
        .write_text_chunk(&png::text_metadata::ITXtChunk::new(
            "todo-reveal",
            serde_json::to_string(&meta)?,
        ))
        .map_err(|e| MediaError::Invalid(e.to_string()))?;
    writer
        .write_image_data(&pixels)
        .map_err(|e| MediaError::Invalid(e.to_string()))?;
    Ok(())
}

pub fn read_reveal(path: &Path) -> Result<(RevealMeta, RgbaImage, RgbaImage), MediaError> {
    let file = File::open(path).map_err(|e| MediaError::io(path, e))?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .map_err(|e| MediaError::Invalid(e.to_string()))?;
    let info = reader.info().clone();
    let text = info
        .uncompressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == "todo-reveal")
        .map(|chunk| chunk.text.clone())
        .or_else(|| {
            info.utf8_text
                .iter()
                .find(|chunk| chunk.keyword == "todo-reveal")
                .and_then(|chunk| chunk.get_text().ok())
        })
        .ok_or_else(|| MediaError::Invalid("missing todo-reveal metadata".into()))?;
    let meta: RevealMeta = serde_json::from_str(&text)?;
    if meta.spec != REVEAL_SPEC || info.width != meta.width * 2 || info.height != meta.height {
        return Err(MediaError::Invalid(
            "reveal metadata does not match PNG dimensions".into(),
        ));
    }
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| MediaError::Invalid("PNG has no image buffer".into()))?
    ];
    let frame = reader
        .next_frame(&mut bytes)
        .map_err(|e| MediaError::Invalid(e.to_string()))?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err(MediaError::Invalid("reveal PNG must be RGBA8".into()));
    }
    let mut color = ImageBuffer::new(meta.width, meta.height);
    let mut timing = ImageBuffer::new(meta.width, meta.height);
    let stride = (meta.width * 2 * 4) as usize;
    for y in 0..meta.height {
        for x in 0..meta.width {
            let a = y as usize * stride + x as usize * 4;
            let b = y as usize * stride + (meta.width + x) as usize * 4;
            color.put_pixel(x, y, Rgba(bytes[a..a + 4].try_into().unwrap()));
            timing.put_pixel(x, y, Rgba(bytes[b..b + 4].try_into().unwrap()));
        }
    }
    Ok((meta, color, timing))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_and_pixels_round_trip() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reveal-tests");
        let _ = std::fs::create_dir_all(&root);
        let path = root.join("roundtrip.reveal.png");
        let color = RgbaImage::from_pixel(2, 1, Rgba([10, 20, 30, 255]));
        let timing = RgbaImage::from_pixel(2, 1, Rgba([0, 1, 2, 255]));
        let meta = RevealMeta {
            spec: String::new(),
            width: 0,
            height: 0,
            duration: 2.0,
            soft_max: 0.4,
            hand: None,
        };
        write_reveal(&path, &color, &timing, meta).unwrap();
        let (meta, read_color, read_timing) = read_reveal(&path).unwrap();
        assert_eq!(meta.spec, REVEAL_SPEC);
        assert_eq!(read_color, color);
        assert_eq!(read_timing, timing);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn compose_reveal_is_transparent_before_time_and_final_after_duration() {
        let color = RgbaImage::from_pixel(2, 1, Rgba([10, 20, 30, 255]));
        let mut timing = RgbaImage::from_pixel(2, 1, Rgba([0, 0, 0, 0]));
        *timing.get_pixel_mut(0, 0) = Rgba([0, 0, 0, 255]);
        *timing.get_pixel_mut(1, 0) = Rgba([255, 255, 0, 255]);
        let meta = RevealMeta {
            spec: REVEAL_SPEC.into(),
            width: 2,
            height: 1,
            duration: 2.0,
            soft_max: 0.4,
            hand: None,
        };
        let first = compose_reveal(&color, &timing, &meta, 0.0).unwrap();
        assert_eq!(first.get_pixel(0, 0)[3], 255);
        assert_eq!(first.get_pixel(1, 0)[3], 0);
        let final_frame = compose_reveal(&color, &timing, &meta, 2.0).unwrap();
        assert_eq!(final_frame, color);
        // The soft edge is a fraction of softMax: full soft (B=255) starts
        // fading in softMax seconds before the pixel's time.
        *timing.get_pixel_mut(1, 0) = Rgba([255, 255, 255, 255]);
        let early = compose_reveal(&color, &timing, &meta, 1.5).unwrap();
        assert_eq!(early.get_pixel(1, 0)[3], 0);
        let fading = compose_reveal(&color, &timing, &meta, 1.8).unwrap();
        assert!((100..160).contains(&fading.get_pixel(1, 0)[3]));
    }
}
