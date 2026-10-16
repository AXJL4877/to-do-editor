#[cfg(test)]
mod tests;

use crate::{files::temporary_file, tools::capture_cancellable, MediaError, MediaService};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

// Bump the profile whenever encoding, geometry or timestamp behavior changes.
const PROFILE: &str = "editproxies-v2-h264-intra-crf20-1280x720-vfr-normalized-sdr";
const FILE_LIMIT: u64 = 1024 * 1024 * 1024;
const CACHE_LIMIT: u64 = 4 * FILE_LIMIT;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditProxy {
    pub path: PathBuf,
    pub source_width: u32,
    pub source_height: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    proxy: EditProxy,
    output_identity: String,
}

struct VideoInfo {
    index: u64,
    width: u32,
    height: u32,
    sar: f64,
    start: f64,
    duration: f64,
    origin: f64,
    time_base: String,
    frames: Option<u64>,
    codec: String,
    has_audio: bool,
    hdr: bool,
}

impl MediaService {
    /// Build or reuse a disposable, video-only all-I H.264 editing proxy.
    /// Clip time is unchanged: input timestamps are normalized by the container
    /// start time, preserving video offsets relative to the original audio.
    pub fn get_edit_proxy(&self, stored: &str) -> Result<EditProxy, MediaError> {
        self.get_edit_proxy_cancellable(stored, Arc::new(AtomicBool::new(false)))
    }

    pub fn get_edit_proxy_cancellable(
        &self,
        stored: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<EditProxy, MediaError> {
        check_cancelled(&cancelled)?;
        let source = self.resolve(stored)?;
        let key = self.cache.key(&source, PROFILE, 1)?;
        let destination = self.cache.path(&key, "mp4");
        self.cache.with_key(&key, || {
            check_cancelled(&cancelled)?;
            if let Some(proxy) = self.cached_proxy(&destination) {
                return Ok(proxy);
            }
            // Also serialize budget reservations for different sources using
            // this cache. Never evict a file a native decoder may still own.
            self.cache.with_key("editproxies-budget", || {
                check_cancelled(&cancelled)?;
                let input = self.proxy_video_info(&source, &cancelled)?;
                if input.hdr {
                    return Err(MediaError::Invalid(
                        "HDR editing proxies require tone mapping; use the original media".into(),
                    ));
                }
                reject_still(&source)?;
                let (width, height) = proxy_dimensions(input.width, input.height)?;
                let directory = destination.parent().unwrap();
                fs::create_dir_all(directory).map_err(|error| MediaError::io(directory, error))?;
                // Invalid entries are owned cache products, and can never be
                // returned by this API. Clean them before reserving capacity.
                remove_if_present(&destination)?;
                remove_if_present(&destination.with_extension("json"))?;
                let available = available_capacity(directory, CACHE_LIMIT, FILE_LIMIT)?;
                let (pending, file) = temporary_file(directory, "mp4")?;
                drop(file);
                let mut args: Vec<OsString> = [
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-y",
                    "-copyts",
                    "-start_at_zero",
                    "-noautorotate",
                    "-threads",
                    "2",
                    "-i",
                ]
                .map(Into::into)
                .to_vec();
                args.push(source.as_os_str().into());
                args.extend(
                    [
                        "-map",
                        &format!("0:{}", input.index),
                        "-an",
                        "-sn",
                        "-dn",
                        "-map_metadata",
                        "-1",
                        "-filter_threads",
                        "1",
                        "-vf",
                        &format!("scale={width}:{height}:flags=fast_bilinear"),
                        "-c:v",
                        "libx264",
                        "-preset",
                        "veryfast",
                        "-crf",
                        "20",
                        "-g",
                        "1",
                        "-keyint_min",
                        "1",
                        "-bf",
                        "0",
                        "-threads",
                        "2",
                        "-pix_fmt",
                        "yuv420p",
                        "-fps_mode",
                        "passthrough",
                        "-enc_time_base",
                        &input.time_base,
                        "-avoid_negative_ts",
                        "disabled",
                        "-fs",
                        &available.to_string(),
                        "-f",
                        "mp4",
                    ]
                    .map(Into::into),
                );
                args.push(pending.path().as_os_str().into());
                capture_cancellable(
                    &self.tools.ffmpeg,
                    args,
                    self.timeout,
                    1024,
                    cancelled.clone(),
                )?;
                check_cancelled(&cancelled)?;
                let output = self.proxy_video_info(pending.path(), &cancelled)?;
                validate_output(&input, &output, width, height)?;
                let size = fs::metadata(pending.path())
                    .map_err(|error| MediaError::io(pending.path(), error))?
                    .len();
                if size > available {
                    return Err(MediaError::Invalid(
                        "editing proxy exceeded its disk budget".into(),
                    ));
                }
                if self.cache.key(&source, PROFILE, 1)? != key {
                    return Err(MediaError::Invalid(
                        "editing proxy source changed during generation".into(),
                    ));
                }
                check_cancelled(&cancelled)?;
                pending.publish(&destination)?;
                let proxy = EditProxy {
                    path: destination.clone(),
                    source_width: input.width,
                    source_height: input.height,
                    width,
                    height,
                };
                let publish_manifest = || -> Result<(), MediaError> {
                    let manifest = Manifest {
                        proxy: proxy.clone(),
                        output_identity: self.cache.key(&destination, "editproxyoutput", 1)?,
                    };
                    let bytes = serde_json::to_vec(&manifest)?;
                    let (pending, mut file) = temporary_file(directory, "json")?;
                    file.write_all(&bytes)
                        .map_err(|error| MediaError::io(pending.path(), error))?;
                    drop(file);
                    check_cancelled(&cancelled)?;
                    pending.publish(&destination.with_extension("json"))
                };
                if let Err(error) = publish_manifest() {
                    let _ = fs::remove_file(&destination);
                    return Err(error);
                }
                Ok(proxy)
            })
        })
    }

    fn cached_proxy(&self, destination: &Path) -> Option<EditProxy> {
        let manifest_path = destination.with_extension("json");
        if fs::metadata(&manifest_path).ok()?.len() > 16 * 1024 {
            return None;
        }
        let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path).ok()?).ok()?;
        let proxy = &manifest.proxy;
        if proxy.path != destination
            || !(2..=1280).contains(&proxy.width)
            || !(2..=720).contains(&proxy.height)
            || proxy.source_width == 0
            || proxy.source_height == 0
            || fs::metadata(destination).ok()?.len() > FILE_LIMIT
            || self.cache.key(destination, "editproxyoutput", 1).ok()? != manifest.output_identity
        {
            return None;
        }
        Some(manifest.proxy)
    }

    fn proxy_video_info(
        &self,
        path: &Path,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<VideoInfo, MediaError> {
        let mut args: Vec<OsString> = [
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ]
        .map(Into::into)
        .to_vec();
        args.push(path.as_os_str().into());
        let bytes = capture_cancellable(
            &self.tools.ffprobe,
            args,
            self.timeout,
            4 * 1024 * 1024,
            cancelled.clone(),
        )?;
        parse_video_info(serde_json::from_slice(&bytes)?)
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), MediaError> {
    if cancelled.load(Ordering::Acquire) {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

fn remove_if_present(path: &Path) -> Result<(), MediaError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(MediaError::io(path, error)),
    }
}

fn available_capacity(
    directory: &Path,
    total_limit: u64,
    file_limit: u64,
) -> Result<u64, MediaError> {
    let mut used = 0u64;
    for entry in fs::read_dir(directory).map_err(|error| MediaError::io(directory, error))? {
        let entry = entry.map_err(|error| MediaError::io(directory, error))?;
        let metadata = entry
            .metadata()
            .map_err(|error| MediaError::io(&entry.path(), error))?;
        if metadata.is_file() {
            used = used.saturating_add(metadata.len());
        }
    }
    let available = total_limit.saturating_sub(used).min(file_limit);
    if available < 1024 * 1024 {
        return Err(MediaError::Invalid(
            "editing proxy cache disk budget is full (4 GiB total, 1 GiB per file)".into(),
        ));
    }
    Ok(available)
}

fn proxy_dimensions(width: u32, height: u32) -> Result<(u32, u32), MediaError> {
    if width < 2 || height < 2 {
        return Err(MediaError::Invalid(
            "editing proxy needs video dimensions of at least 2 pixels".into(),
        ));
    }
    let ratio = (1280.0 / width as f64).min(720.0 / height as f64).min(1.0);
    Ok((
        ((width as f64 * ratio) as u32 / 2 * 2).max(2),
        ((height as f64 * ratio) as u32 / 2 * 2).max(2),
    ))
}

fn decimal(value: &Value) -> Option<f64> {
    value
        .as_str()?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}
fn rational(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once([':', '/'])?;
    let value = numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?;
    (value.is_finite() && value > 0.0).then_some(value)
}
fn parse_video_info(json: Value) -> Result<VideoInfo, MediaError> {
    let invalid = || {
        MediaError::Invalid(
            "editing proxy requires a video with finite timing and dimensions".into(),
        )
    };
    let streams = json["streams"].as_array().ok_or_else(invalid)?;
    let stream = streams
        .iter()
        .find(|stream| {
            stream["codec_type"] == "video"
                && stream["disposition"]["attached_pic"].as_u64().unwrap_or(0) == 0
        })
        .ok_or_else(invalid)?;
    let width = stream["width"]
        .as_u64()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(invalid)?;
    let height = stream["height"]
        .as_u64()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(invalid)?;
    let start = decimal(&stream["start_time"]).unwrap_or(0.0);
    let origin = decimal(&json["format"]["start_time"]).unwrap_or(start);
    let duration = decimal(&stream["duration"])
        .or_else(|| decimal(&json["format"]["duration"]).map(|end| end - (start - origin)))
        .filter(|v| *v > 0.0)
        .ok_or_else(invalid)?;
    let time_base = stream["time_base"]
        .as_str()
        .filter(|v| rational(v).is_some())
        .ok_or_else(invalid)?
        .replace('/', ":");
    Ok(VideoInfo {
        index: stream["index"].as_u64().ok_or_else(invalid)?,
        width,
        height,
        sar: stream["sample_aspect_ratio"]
            .as_str()
            .and_then(rational)
            .unwrap_or(1.0),
        start,
        duration,
        origin,
        time_base,
        frames: stream["nb_frames"].as_str().and_then(|v| v.parse().ok()),
        codec: stream["codec_name"].as_str().unwrap_or_default().into(),
        has_audio: streams.iter().any(|stream| stream["codec_type"] == "audio"),
        hdr: matches!(
            stream["color_transfer"].as_str(),
            Some("smpte2084" | "arib-std-b67")
        ),
    })
}

fn reject_still(source: &Path) -> Result<(), MediaError> {
    if matches!(
        source
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif" | "tif" | "tiff"
    ) {
        return Err(MediaError::Invalid(
            "editing proxies are only generated for video".into(),
        ));
    }
    Ok(())
}

fn validate_output(
    input: &VideoInfo,
    output: &VideoInfo,
    width: u32,
    height: u32,
) -> Result<(), MediaError> {
    let input_dar = input.width as f64 / input.height as f64 * input.sar;
    let output_dar = output.width as f64 / output.height as f64 * output.sar;
    let tick = rational(&input.time_base).unwrap_or(0.001).max(0.001) * 2.0;
    if output.codec != "h264"
        || output.has_audio
        || output.width != width
        || output.height != height
        || (input_dar - output_dar).abs() > input_dar * 0.0001
        || (output.start - (input.start - input.origin)).abs() > tick
        || (output.duration - input.duration).abs() > tick
        || input
            .frames
            .is_some_and(|frames| output.frames != Some(frames))
    {
        return Err(MediaError::Invalid("editing proxy output is incomplete or changed source geometry/timing (possibly disk budget exceeded)".into()));
    }
    Ok(())
}
