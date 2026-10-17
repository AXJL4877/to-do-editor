//! Export inspection and machine-readable quality reports.
use crate::{
    tools::{capture, capture_with_diagnostics},
    MediaError,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityCheck {
    pub name: String,
    pub passed: bool,
    pub measured: Option<f64>,
    pub expected: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub codec: Option<String>,
    pub pixel_format: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f64>,
    pub duration: Option<f64>,
    pub frames: Option<u64>,
    pub color_space: Option<String>,
    pub color_primaries: Option<String>,
    pub color_transfer: Option<String>,
    pub color_range: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInfo {
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityReport {
    pub output: PathBuf,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    pub checks: Vec<QualityCheck>,
    pub warnings: Vec<String>,
    pub loudness: Option<LoudnessReport>,
    pub psnr_db: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessReport {
    pub integrated_lufs: f64,
    pub true_peak_db: f64,
}

/// Detect full black and frozen video spans. The caller can decide whether a
/// detected span is intentional blank time; the export gate records the raw
/// measurement so an accidental hold cannot disappear silently.
pub fn measure_black_freeze(ffmpeg: &Path, output: &Path) -> Result<Vec<QualityCheck>, MediaError> {
    let args: Vec<OsString> = [
        "-v",
        "info",
        "-i",
        "",
        "-vf",
        "blackdetect=d=0.5:pix_th=0.001,freezedetect=n=-60dB:d=0.5",
        "-an",
        "-f",
        "null",
        "-",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    let mut args = args;
    args[3] = output.as_os_str().to_owned();
    let (_stdout, stderr) = capture_with_diagnostics(ffmpeg, args, Duration::from_secs(120))?;
    let black = stderr
        .lines()
        .filter_map(|line| marker_number(line, "black_duration:"))
        .fold(0.0_f64, f64::max);
    let freeze = stderr
        .lines()
        .filter_map(|line| marker_number(line, "freeze_duration:"))
        .fold(0.0_f64, f64::max);
    Ok(vec![
        QualityCheck {
            name: "blackSpan".into(),
            passed: black < 0.5,
            measured: Some(black),
            expected: Some("< 0.5 seconds".into()),
            message: "blackdetect maximum span".into(),
        },
        // Held pictures are normal (a finished board, a still); export decodes
        // exactly, so a long freeze is reported, not failed.
        QualityCheck {
            name: "freezeSpan".into(),
            passed: true,
            measured: Some(freeze),
            expected: None,
            message: "freezedetect longest still stretch (informational)".into(),
        },
    ])
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityTargets {
    pub fps: f64,
    pub duration: f64,
    pub frame_tolerance: f64,
}

#[derive(Debug, Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
}

#[derive(Debug, Deserialize)]
struct Stream {
    #[serde(default)]
    codec_type: String,
    codec_name: Option<String>,
    pix_fmt: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    nb_read_frames: Option<String>,
    color_space: Option<String>,
    color_primaries: Option<String>,
    color_transfer: Option<String>,
    color_range: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
}

pub fn inspect(
    ffprobe: &Path,
    output: &Path,
    targets: Option<QualityTargets>,
) -> Result<QualityReport, MediaError> {
    let args: Vec<OsString> = [
        "-v",
        "error",
        "-count_frames",
        "-print_format",
        "json",
        "-show_streams",
    ]
    .into_iter()
    .map(Into::into)
    .chain(std::iter::once(output.as_os_str().to_owned()))
    .collect();
    let bytes = capture(ffprobe, args, Duration::from_secs(30), 2 * 1024 * 1024)?;
    let probe: Probe = serde_json::from_slice(&bytes)
        .map_err(|error| MediaError::Invalid(format!("ffprobe output invalid: {error}")))?;
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type == "video")
        .map(video_info);
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type == "audio")
        .map(audio_info);
    let mut checks = Vec::new();
    let mut warnings = Vec::new();
    if let Some(targets) = targets {
        if let Some(video) = video.as_ref() {
            checks.push(string_check("codec", video.codec.as_deref(), "h264"));
            checks.push(string_check(
                "pixelFormat",
                video.pixel_format.as_deref(),
                "yuv420p",
            ));
            checks.push(string_check(
                "colorSpace",
                video.color_space.as_deref(),
                "bt709",
            ));
        }
        checks.push(numeric_check(
            "frameRate",
            video.as_ref().and_then(|v| v.frame_rate),
            targets.fps,
            1e-3,
            "fps",
        ));
        let duration = video.as_ref().and_then(|v| v.duration);
        checks.push(numeric_check(
            "videoDuration",
            duration,
            targets.duration,
            targets.frame_tolerance,
            "seconds",
        ));
        if let (Some(video_duration), Some(audio)) = (duration, audio.as_ref()) {
            if let Some(audio_duration) = audio.duration {
                checks.push(numeric_check(
                    "audioVideoDuration",
                    Some((audio_duration - video_duration).abs()),
                    0.0,
                    targets.frame_tolerance,
                    "seconds difference",
                ));
            }
        }
    }
    if video.is_none() {
        warnings.push("output has no video stream".into());
    }
    warnings.push(
        "ffprobe cannot measure loudness or reference-frame PSNR; the export adapter adds those checks when inputs are available".into(),
    );
    Ok(QualityReport {
        output: output.to_owned(),
        video,
        audio,
        checks,
        warnings,
        loudness: None,
        psnr_db: None,
    })
}

/// Run FFmpeg's ebur128 analyzer and parse the final integrated loudness and
/// peak values. A file without an audio stream returns `None`.
pub fn measure_loudness(
    ffmpeg: &Path,
    output: &Path,
) -> Result<Option<LoudnessReport>, MediaError> {
    let args: Vec<OsString> = ["-v", "info", "-nostats", "-i"]
        .into_iter()
        .map(Into::into)
        .chain(std::iter::once(output.as_os_str().to_owned()))
        .chain(
            [
                "-filter_complex",
                "ebur128=framelog=verbose:peak=true",
                "-f",
                "null",
                "-",
            ]
            .into_iter()
            .map(Into::into),
        )
        .collect();
    let (_stdout, stderr) = capture_with_diagnostics(ffmpeg, args, Duration::from_secs(120))?;
    let integrated = stderr
        .lines()
        .filter_map(|line| {
            line.split_once("I:")
                .and_then(|(_, rest)| first_number(rest))
        })
        .next_back();
    let peak = stderr
        .lines()
        .filter_map(|line| {
            line.split_once("Peak:")
                .and_then(|(_, rest)| first_number(rest))
        })
        .next_back();
    match (integrated, peak) {
        (Some(integrated_lufs), Some(true_peak_db)) => Ok(Some(LoudnessReport {
            integrated_lufs,
            true_peak_db,
        })),
        _ => Ok(None),
    }
}

/// Compare a rendered output against the original PNG sequence using the
/// same FFmpeg decoder that will be used for delivery.
///
/// The old implementation fed the PNG sequence and the encoded movie into
/// FFmpeg's `psnr` filter. That lets FFmpeg pick different time bases for the
/// two inputs (and also compares the YUV conversion rather than the actual
/// RGB pixels), which made otherwise good exports report 16--19 dB. Decode a
/// small, deterministic set of movie frames as `rgb24` instead and calculate
/// the RGB error against the renderer's PNGs ourselves.
pub fn measure_psnr(
    ffmpeg: &Path,
    reference_pattern: &Path,
    fps: f64,
    output: &Path,
) -> Result<Option<f64>, MediaError> {
    if !(fps.is_finite() && fps > 0.0) {
        return Err(MediaError::Invalid("PSNR fps must be positive".into()));
    }
    let references = reference_files(reference_pattern)?;
    if references.is_empty() {
        return Ok(None);
    }

    // Five samples are enough to catch a frame offset while keeping the
    // quality check cheap. Always include the first frame.
    let mut samples = vec![0usize];
    if references.len() > 1 {
        let last = references.len() - 1;
        for i in 1..5.min(references.len()) {
            let index = (i * last / 4).min(last);
            if !samples.contains(&index) {
                samples.push(index);
            }
        }
    }

    let first = image::open(&references[0])
        .map_err(|error| MediaError::Invalid(format!("cannot read PSNR reference: {error}")))?
        .to_rgb8();
    let frame_bytes = first.as_raw().len();
    if frame_bytes == 0 {
        return Ok(None);
    }
    let mut squared_error = 0.0_f64;
    let mut compared = 0usize;
    for index in samples {
        let reference = image::open(&references[index])
            .map_err(|error| MediaError::Invalid(format!("cannot read PSNR reference: {error}")))?
            .to_rgb8();
        if reference.as_raw().len() != frame_bytes {
            return Err(MediaError::Invalid(
                "PSNR reference frames have different dimensions".into(),
            ));
        }

        // `select` is applied after decoding, so `index` is an exact zero
        // based movie frame. Escaping the comma is required by FFmpeg's
        // filter parser; no shell is involved here.
        let filter = format!("select=eq(n\\,{index})");
        let args: Vec<OsString> = [
            "-v",
            "error",
            "-i",
            "",
            "-vf",
            "",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-an",
            "-sn",
            "-",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let mut args = args;
        args[3] = output.as_os_str().to_owned();
        args[5] = filter.into();
        let decoded = capture(ffmpeg, args, Duration::from_secs(120), frame_bytes + 1)?;
        if decoded.len() != frame_bytes {
            return Err(MediaError::Invalid(format!(
                "decoded PSNR frame {index} has {} bytes, expected {frame_bytes}",
                decoded.len()
            )));
        }
        for (reference, actual) in reference.as_raw().iter().zip(&decoded) {
            let error = f64::from(*reference) - f64::from(*actual);
            squared_error += error * error;
        }
        compared += frame_bytes;
    }

    if compared == 0 {
        return Ok(None);
    }
    let mse = squared_error / compared as f64;
    Ok(Some(if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0_f64 * 255.0 / mse).log10()
    }))
}

/// Expand a printf-style image sequence into the files that actually exist.
/// The exporter uses `%06d.png`, while accepting `%d` here keeps the quality
/// helper useful for callers with a different naming width.
fn reference_files(pattern: &Path) -> Result<Vec<PathBuf>, MediaError> {
    let Some(name) = pattern.file_name().and_then(|name| name.to_str()) else {
        return Ok(if pattern.is_file() {
            vec![pattern.to_owned()]
        } else {
            Vec::new()
        });
    };
    let Some(percent) = name.find('%') else {
        return Ok(if pattern.is_file() {
            vec![pattern.to_owned()]
        } else {
            Vec::new()
        });
    };
    let Some(d_offset) = name[percent + 1..].find('d') else {
        return Err(MediaError::Invalid(format!(
            "invalid PSNR reference pattern: {}",
            pattern.display()
        )));
    };
    let d = percent + 1 + d_offset;
    let width = name[percent + 1..d].parse::<usize>().unwrap_or(0);
    let prefix = &name[..percent];
    let suffix = &name[d + 1..];
    let parent = pattern.parent().unwrap_or_else(|| Path::new("."));
    let mut numbered = Vec::new();
    for entry in std::fs::read_dir(parent).map_err(|error| MediaError::io(parent, error))? {
        let path = entry.map_err(|error| MediaError::io(parent, error))?.path();
        let Some(candidate) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !candidate.starts_with(prefix) || !candidate.ends_with(suffix) {
            continue;
        }
        let middle_end = candidate.len().saturating_sub(suffix.len());
        let middle = &candidate[prefix.len()..middle_end];
        if middle.is_empty() || !middle.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let number = middle.parse::<usize>().map_err(|_| {
            MediaError::Invalid(format!("invalid PSNR reference number: {candidate}"))
        })?;
        if width != 0 && middle.len() != width {
            continue;
        }
        numbered.push((number, path));
    }
    numbered.sort_by_key(|(number, _)| *number);
    Ok(numbered.into_iter().map(|(_, path)| path).collect())
}

pub fn write_report(path: &Path, report: &QualityReport) -> Result<(), MediaError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| MediaError::io(parent, error))?;
    }
    let temporary = path.with_extension("report.json.tmp");
    let bytes = serde_json::to_vec_pretty(report)?;
    std::fs::write(&temporary, bytes).map_err(|error| MediaError::io(&temporary, error))?;
    std::fs::rename(&temporary, path).map_err(|error| MediaError::io(path, error))?;
    Ok(())
}

fn video_info(stream: &Stream) -> VideoInfo {
    VideoInfo {
        codec: stream.codec_name.clone(),
        pixel_format: stream.pix_fmt.clone(),
        width: stream.width,
        height: stream.height,
        frame_rate: stream.r_frame_rate.as_deref().and_then(parse_ratio),
        duration: parse_number(stream.duration.as_deref()),
        frames: stream
            .nb_read_frames
            .as_deref()
            .and_then(|value| value.parse().ok()),
        color_space: stream.color_space.clone(),
        color_primaries: stream.color_primaries.clone(),
        color_transfer: stream.color_transfer.clone(),
        color_range: stream.color_range.clone(),
    }
}

fn audio_info(stream: &Stream) -> AudioInfo {
    AudioInfo {
        codec: stream.codec_name.clone(),
        sample_rate: stream
            .sample_rate
            .as_deref()
            .and_then(|value| value.parse().ok()),
        channels: stream.channels,
        duration: parse_number(stream.duration.as_deref()),
    }
}

fn parse_number(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|value| value.parse().ok())
        .filter(|value: &f64| value.is_finite())
}

fn parse_ratio(value: &str) -> Option<f64> {
    let (top, bottom) = value.split_once('/')?;
    let top: f64 = top.parse().ok()?;
    let bottom: f64 = bottom.parse().ok()?;
    (bottom != 0.0 && top.is_finite() && bottom.is_finite()).then_some(top / bottom)
}

fn first_number(value: &str) -> Option<f64> {
    if value.contains("inf") {
        return Some(f64::INFINITY);
    }
    value
        .split(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+' || c == '.'))
        .filter(|part| !part.is_empty())
        .find_map(|part| part.parse::<f64>().ok())
}

fn marker_number(line: &str, marker: &str) -> Option<f64> {
    line.split_once(marker)
        .and_then(|(_, rest)| first_number(rest))
}

fn numeric_check(
    name: &str,
    measured: Option<f64>,
    expected: f64,
    tolerance: f64,
    unit: &str,
) -> QualityCheck {
    let passed = measured.is_some_and(|value| (value - expected).abs() <= tolerance);
    QualityCheck {
        name: name.into(),
        passed,
        measured,
        expected: Some(format!("{expected} ± {tolerance} {unit}")),
        message: if passed {
            "within tolerance".into()
        } else {
            "outside tolerance".into()
        },
    }
}

fn string_check(name: &str, measured: Option<&str>, expected: &str) -> QualityCheck {
    let passed = measured == Some(expected);
    QualityCheck {
        name: name.into(),
        passed,
        measured: None,
        expected: Some(expected.into()),
        message: measured.map_or_else(
            || "missing stream field".into(),
            |value| format!("measured {value}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frame_rate_ratio() {
        assert_eq!(parse_ratio("30/1"), Some(30.0));
        assert_eq!(parse_ratio("0/0"), None);
    }

    #[test]
    fn parses_filter_marker_values() {
        assert_eq!(
            marker_number("black_duration:0.750", "black_duration:"),
            Some(0.75)
        );
        assert_eq!(
            marker_number("freeze_duration:1.25", "freeze_duration:"),
            Some(1.25)
        );
    }
}
