//! PNG sequence encoding, including the packed-alpha interchange format.
use crate::{MediaError, MediaService};
use image::{ImageBuffer, Rgba, RgbaImage};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    format: ProbeFormat,
    #[serde(default)]
    streams: Vec<ProbeStream>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ProbeFormat {
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeStream {
    #[serde(default)]
    codec_type: String,
    width: Option<u32>,
    height: Option<u32>,
    #[serde(default)]
    nb_read_frames: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeQuality {
    Standard,
    High,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncodeInfo {
    pub file: PathBuf,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub alpha: bool,
}

impl MediaService {
    #[allow(clippy::too_many_arguments)]
    pub fn encode_frames(
        &self,
        frames_dir: &Path,
        fps: f64,
        alpha: bool,
        quality: EncodeQuality,
        audio: Option<&Path>,
        out: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<EncodeInfo, MediaError> {
        encode_with_tool(
            &self.tools.ffmpeg,
            frames_dir,
            fps,
            alpha,
            quality,
            audio,
            out,
            cancel,
        )
    }
}

pub fn encode_frames(
    frames_dir: &Path,
    fps: f64,
    alpha: bool,
    quality: EncodeQuality,
    audio: Option<&Path>,
    out: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<EncodeInfo, MediaError> {
    let ffmpeg = std::env::var_os("TODO_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    encode_with_tool(&ffmpeg, frames_dir, fps, alpha, quality, audio, out, cancel)
}

#[allow(clippy::too_many_arguments)]
fn encode_with_tool(
    ffmpeg: &Path,
    frames_dir: &Path,
    fps: f64,
    alpha: bool,
    quality: EncodeQuality,
    audio: Option<&Path>,
    out: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<EncodeInfo, MediaError> {
    if !fps.is_finite() || !(1.0..=240.0).contains(&fps) {
        return Err(MediaError::Invalid("fps must be in 1..=240".into()));
    }
    let mut frames: Vec<PathBuf> = fs::read_dir(frames_dir)
        .map_err(|e| MediaError::io(frames_dir, e))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        })
        .collect();
    frames.sort();
    if frames.is_empty() {
        return Err(MediaError::Invalid("frames directory is empty".into()));
    }
    let first = image::open(&frames[0])
        .map_err(|e| MediaError::Invalid(e.to_string()))?
        .to_rgba8();
    let (source_width, source_height) = first.dimensions();
    if source_width == 0 || source_height == 0 {
        return Err(MediaError::Invalid("frame has no pixels".into()));
    }
    // libx264 with yuv420p requires even dimensions. Reveal crops are tight
    // and can have an odd width or height, so pad with transparent pixels
    // before packing without changing the visible content.
    let width = source_width.next_multiple_of(2);
    let height = source_height.next_multiple_of(2);
    let temporary = out
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".todo-packed-{}", Uuid::new_v4()));
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| MediaError::io(parent, e))?;
    }
    fs::create_dir_all(&temporary).map_err(|e| MediaError::io(&temporary, e))?;
    let result = (|| {
        for (index, frame) in frames.iter().enumerate() {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(MediaError::Cancelled);
            }
            let image = image::open(frame)
                .map_err(|e| MediaError::Invalid(e.to_string()))?
                .to_rgba8();
            if image.dimensions() != (source_width, source_height) {
                return Err(MediaError::Invalid("frame dimensions differ".into()));
            }
            let image = if (source_width, source_height) == (width, height) {
                image
            } else {
                let mut padded = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
                image::imageops::overlay(&mut padded, &image, 0, 0);
                padded
            };
            let packed = if alpha { pack_alpha(&image) } else { image };
            packed
                .save(temporary.join(format!("{:06}.png", index + 1)))
                .map_err(|e| MediaError::Invalid(e.to_string()))?;
        }
        let mut args: Vec<OsString> = vec![
            "-y".into(),
            "-framerate".into(),
            fps.to_string().into(),
            "-i".into(),
            temporary.join("%06d.png").into(),
        ];
        if let Some(audio) = audio {
            args.extend(["-i".into(), audio.into()]);
        }
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-crf".into(),
            (match quality {
                EncodeQuality::Standard => "18",
                EncodeQuality::High => "16",
            })
            .into(),
            "-preset".into(),
            "slow".into(),
            "-g".into(),
            fps.round().max(1.0).to_string().into(),
        ]);
        if alpha {
            // MP4 only preserves arbitrary keys as format metadata when the
            // `use_metadata_tags` muxer flag is enabled.  Keeping this as a
            // container tag makes the packed-alpha contract discoverable by
            // ffprobe and by the native importer.
            args.extend([
                "-metadata".into(),
                "todo_alpha=stacked".into(),
                "-movflags".into(),
                "+use_metadata_tags".into(),
            ]);
        }
        if audio.is_some() {
            args.extend([
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-shortest".into(),
            ]);
        }
        args.push(out.as_os_str().to_owned());
        crate::tools::capture_cancellable(
            ffmpeg,
            args,
            Duration::from_secs(600),
            4 * 1024 * 1024,
            cancel,
        )?;
        if !out.is_file() {
            return Err(MediaError::Invalid("ffmpeg produced no output".into()));
        }
        validate_output(ffmpeg, out, width, height, frames.len(), alpha)?;
        Ok(EncodeInfo {
            file: out.to_owned(),
            duration: frames.len() as f64 / fps,
            width,
            height,
            frames: frames.len(),
            alpha,
        })
    })();
    let _ = fs::remove_dir_all(&temporary);
    if result.is_err() {
        let _ = fs::remove_file(out);
    }
    result
}

/// Verify the container contract before publishing the output.  This catches
/// silent muxer changes (wrong packed dimensions, dropped metadata, or a
/// truncated sequence) while the temporary directory is still available for
/// cleanup by the caller.
fn validate_output(
    ffmpeg: &Path,
    out: &Path,
    width: u32,
    height: u32,
    frames: usize,
    alpha: bool,
) -> Result<(), MediaError> {
    let ffprobe = std::env::var_os("TODO_FFPROBE")
        .map(PathBuf::from)
        .or_else(|| {
            ffmpeg
                .parent()
                .map(|parent| parent.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX)))
        });
    let Some(ffprobe) = ffprobe.filter(|path| path.is_file()) else {
        return Ok(());
    };
    let args: Vec<OsString> = [
        "-v",
        "error",
        "-count_frames",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]
    .into_iter()
    .map(Into::into)
    .chain(std::iter::once(out.as_os_str().to_owned()))
    .collect();
    let bytes = crate::tools::capture(&ffprobe, args, Duration::from_secs(30), 2 * 1024 * 1024)?;
    let probe: ProbeOutput = serde_json::from_slice(&bytes)
        .map_err(|error| MediaError::Invalid(format!("ffprobe output invalid: {error}")))?;
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type == "video")
        .ok_or_else(|| MediaError::Invalid("encoded output has no video stream".into()))?;
    let expected_height = if alpha { height * 2 } else { height };
    if video.width != Some(width) || video.height != Some(expected_height) {
        return Err(MediaError::Invalid(format!(
            "encoded dimensions are {:?}x{:?}, expected {width}x{expected_height}",
            video.width, video.height
        )));
    }
    if video
        .nb_read_frames
        .as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|count| count != frames)
    {
        return Err(MediaError::Invalid(
            "encoded frame count differs from input".into(),
        ));
    }
    if alpha && probe.format.tags.get("todo_alpha").map(String::as_str) != Some("stacked") {
        return Err(MediaError::Invalid("packed-alpha metadata missing".into()));
    }
    Ok(())
}

fn pack_alpha(source: &RgbaImage) -> RgbaImage {
    let (width, height) = source.dimensions();
    let mut color = source.clone();
    for _ in 0..8 {
        let previous = color.clone();
        for y in 0..height {
            for x in 0..width {
                if previous.get_pixel(x, y)[3] != 0 {
                    continue;
                }
                let mut replacement = None;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        if nx >= 0 && ny >= 0 && (nx as u32) < width && (ny as u32) < height {
                            let pixel = previous.get_pixel(nx as u32, ny as u32);
                            if pixel[3] != 0 {
                                replacement = Some([pixel[0], pixel[1], pixel[2], 0]);
                                break;
                            }
                        }
                    }
                    if let Some(value) = replacement {
                        *color.get_pixel_mut(x, y) = Rgba(value);
                    }
                }
            }
        }
    }
    let mut packed: RgbaImage = ImageBuffer::new(width, height * 2);
    for y in 0..height {
        for x in 0..width {
            let pixel = color.get_pixel(x, y);
            packed.put_pixel(x, y, *pixel);
            packed.put_pixel(x, y + height, Rgba([pixel[3], pixel[3], pixel[3], 255]));
        }
    }
    packed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_rgb_is_dilated_into_packed_color() {
        let mut image = RgbaImage::from_pixel(3, 1, Rgba([0, 0, 0, 0]));
        *image.get_pixel_mut(1, 0) = Rgba([200, 100, 20, 255]);
        let packed = pack_alpha(&image);
        assert_eq!(packed.dimensions(), (3, 2));
        let p = packed.get_pixel(0, 0);
        assert_eq!((p[0], p[1], p[2]), (200, 100, 20));
        assert_eq!(packed.get_pixel(0, 1)[0], 0);
        assert_eq!(packed.get_pixel(1, 1)[0], 255);
    }
}
