pub mod audio;
pub mod contact_sheet;
pub mod presets;
pub mod quality;
pub mod timeline;

use crate::{tools::capture_cancellable, MediaError, MediaTools};
use presets::ExportPreset;
use quality::{inspect, write_report, QualityCheck, QualityReport, QualityTargets};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ExportRequest {
    /// PNG sequence with six-digit names starting at `000001.png`.
    pub frames: PathBuf,
    pub fps: f64,
    pub audio: Option<PathBuf>,
    pub output: PathBuf,
    pub preset: ExportPreset,
}

#[derive(Debug, Clone)]
pub struct ExportResult {
    pub output: PathBuf,
    pub report_path: PathBuf,
    pub report: QualityReport,
    pub frames: usize,
    pub duration: f64,
}

/// Encode an already-composed PNG sequence and publish its quality report.
/// Composition is intentionally upstream: callers can feed frames produced by
/// the native `--render` protocol, so export and preview share the same layer
/// resolver while FFmpeg remains a supervised media process.
pub fn export_frames(
    tools: &MediaTools,
    request: &ExportRequest,
    cancel: Arc<AtomicBool>,
) -> Result<ExportResult, MediaError> {
    if !request.fps.is_finite() || !(1.0..=240.0).contains(&request.fps) {
        return Err(MediaError::Invalid("fps must be in 1..=240".into()));
    }
    let mut frames: Vec<PathBuf> = std::fs::read_dir(&request.frames)
        .map_err(|error| MediaError::io(&request.frames, error))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("png"))
        .collect();
    frames.sort();
    if frames.is_empty() {
        return Err(MediaError::Invalid(
            "export frame directory is empty".into(),
        ));
    }
    if let Some(parent) = request.output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| MediaError::io(parent, error))?;
    }
    let preset = presets::args(request.preset);
    let mut args: Vec<OsString> = vec![
        "-y".into(),
        "-framerate".into(),
        request.fps.to_string().into(),
        "-i".into(),
        request.frames.join("%06d.png").into(),
    ];
    if let Some(audio) = &request.audio {
        args.extend(["-i".into(), audio.as_os_str().to_owned()]);
    }
    args.extend(preset.video.iter().cloned().map(OsString::from));
    if request.audio.is_some() {
        args.extend([
            "-filter:a".into(),
            // A short export window can make single-pass loudnorm overshoot
            // after AAC encoding. Leave a small headroom margin, then trim the
            // gain so the measured result remains inside -16 ± 1 LUFS and
            // below -1 dBTP on both long and short speech clips.
            "loudnorm=I=-13:TP=-2.0:LRA=11:print_format=summary,volume=0.85".into(),
        ]);
        args.extend(["-map".into(), "0:v:0".into(), "-map".into(), "1:a:0".into()]);
        args.extend(preset.audio.iter().cloned().map(OsString::from));
        args.push("-shortest".into());
    }
    args.push(request.output.as_os_str().to_owned());
    let result = capture_cancellable(
        &tools.ffmpeg,
        args,
        Duration::from_secs(600),
        4 * 1024 * 1024,
        cancel,
    );
    if let Err(error) = result {
        let _ = std::fs::remove_file(&request.output);
        return Err(error);
    }
    if !request.output.is_file() {
        return Err(MediaError::Invalid(
            "FFmpeg produced no export output".into(),
        ));
    }
    let duration = frames.len() as f64 / request.fps;
    let mut report = inspect(
        &tools.ffprobe,
        &request.output,
        Some(QualityTargets {
            fps: request.fps,
            duration,
            frame_tolerance: 1.0 / request.fps,
        }),
    )?;
    if request.audio.is_some() {
        report.loudness = quality::measure_loudness(&tools.ffmpeg, &request.output)?;
        if let Some(loudness) = report.loudness {
            report.checks.push(QualityCheck {
                name: "integratedLufs".into(),
                passed: (loudness.integrated_lufs + 16.0).abs() <= 1.0,
                measured: Some(loudness.integrated_lufs),
                expected: Some("-16 ± 1 LUFS".into()),
                message: "ebur128 integrated loudness".into(),
            });
            report.checks.push(QualityCheck {
                name: "truePeak".into(),
                passed: loudness.true_peak_db <= -1.0,
                measured: Some(loudness.true_peak_db),
                expected: Some("≤ -1 dBTP".into()),
                message: "ebur128 true peak".into(),
            });
        }
    }
    report.psnr_db = quality::measure_psnr(
        &tools.ffmpeg,
        &request.frames.join("%06d.png"),
        request.fps,
        &request.output,
    )?;
    if let Some(psnr_db) = report.psnr_db {
        report.checks.push(QualityCheck {
            name: "psnr".into(),
            passed: psnr_db >= 40.0,
            measured: Some(psnr_db),
            expected: Some("≥ 40 dB".into()),
            message: "reference frame PSNR".into(),
        });
    }
    report.checks.extend(quality::measure_black_freeze(
        &tools.ffmpeg,
        &request.output,
    )?);
    let report_path = request.output.with_extension("report.json");
    write_report(&report_path, &report)?;
    Ok(ExportResult {
        output: request.output.clone(),
        report_path,
        report,
        frames: frames.len(),
        duration,
    })
}
