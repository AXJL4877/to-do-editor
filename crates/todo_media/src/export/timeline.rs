//! Timeline export pieces around FFmpeg: a raw-frame video sink fed by the
//! native offline renderer, the audio mixdown (two-pass loudness), muxing and
//! the quality report. Frame composition itself is the renderer's.
use super::audio::{filter_graph, loudnorm_apply, loudnorm_measure, LoudnessMeasurement};
use super::presets::{args as preset_args, ExportPreset};
use super::quality::{
    inspect, measure_black_freeze, measure_loudness, measure_psnr, write_report, QualityCheck,
    QualityReport, QualityTargets,
};
use crate::tools::capture_with_diagnostics;
use crate::{MediaError, MediaTools};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::Duration;
use todo_core::ResolvedAudioClip;

fn invalid(message: impl Into<String>) -> MediaError {
    MediaError::Invalid(message.into())
}

/// FFmpeg encoding raw BGRA frames from stdin into a video-only file.
pub struct VideoSink {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: std::thread::JoinHandle<String>,
    frame_bytes: usize,
    pub frames: usize,
}

impl VideoSink {
    pub fn start(
        tools: &MediaTools,
        width: u32,
        height: u32,
        fps: f64,
        preset: ExportPreset,
        output: &Path,
    ) -> Result<Self, MediaError> {
        if !(fps.is_finite() && (1.0..=240.0).contains(&fps)) || width < 2 || height < 2 {
            return Err(invalid("export size or fps is invalid"));
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MediaError::io(parent, e))?;
        }
        let mut video: Vec<String> = preset_args(preset).video;
        // The project decides the frame rate; the preset only the quality.
        if let Some(i) = video.iter().position(|a| a == "-r") {
            video.drain(i..i + 2);
        }
        let mut command = Command::new(&tools.ffmpeg);
        command
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "bgra",
            ])
            .args([
                "-s",
                &format!("{width}x{height}"),
                "-r",
                &fps.to_string(),
                "-i",
                "-",
            ])
            .args(&video)
            .arg("-an")
            .arg(output)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        todo_process::hide_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| invalid(format!("cannot start FFmpeg: {e}")))?;
        let mut pipe = child.stderr.take().expect("stderr is piped");
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        });
        Ok(Self {
            stdin: child.stdin.take(),
            child,
            stderr,
            frame_bytes: width as usize * height as usize * 4,
            frames: 0,
        })
    }

    pub fn push(&mut self, bgra: &[u8]) -> Result<(), MediaError> {
        if bgra.len() != self.frame_bytes {
            return Err(invalid("frame size does not match the export size"));
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| invalid("encoder is closed"))?;
        stdin
            .write_all(bgra)
            .map_err(|e| invalid(format!("encoder stopped: {e}")))?;
        self.frames += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<usize, MediaError> {
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|e| invalid(e.to_string()))?;
        let stderr = self.stderr.join().unwrap_or_default();
        if !status.success() {
            return Err(invalid(format!("FFmpeg encode failed: {}", tail(&stderr))));
        }
        Ok(self.frames)
    }

    pub fn abort(mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tail(text: &str) -> String {
    let start = text.len().saturating_sub(600);
    text[text
        .char_indices()
        .map(|(i, _)| i)
        .find(|i| *i >= start)
        .unwrap_or(0)..]
        .trim()
        .to_owned()
}

fn audio_inputs(clips: &[ResolvedAudioClip]) -> Vec<OsString> {
    clips
        .iter()
        .flat_map(|clip| ["-i".into(), OsString::from(&clip.path)])
        .collect()
}

fn parse_measurement(stderr: &str) -> Option<LoudnessMeasurement> {
    let start = stderr.rfind('{')?;
    let end = stderr[start..].find('}')? + start;
    let value: serde_json::Value = serde_json::from_str(&stderr[start..=end]).ok()?;
    let number = |key: &str| value.get(key)?.as_str()?.trim().parse::<f64>().ok();
    Some(LoudnessMeasurement {
        input_i: number("input_i")?,
        input_lra: number("input_lra")?,
        input_tp: number("input_tp")?,
        input_thresh: number("input_thresh")?,
        target_offset: number("target_offset")?,
    })
}

/// Mix the timeline's audio (gains, fades, rates, positions) to a 48 kHz
/// stereo WAV of `duration`, normalized to -16 LUFS in two passes. `None`
/// when the timeline has no audible clip.
pub fn mix_audio(
    tools: &MediaTools,
    clips: &[ResolvedAudioClip],
    duration: f64,
    output: &Path,
) -> Result<Option<PathBuf>, MediaError> {
    if clips.is_empty() {
        return Ok(None);
    }
    let graph = filter_graph(clips);
    // Pad to the video length with `apad=whole_dur` (the mix never runs
    // longer than the timeline). An `apad,atrim` pair misbehaves after
    // `atempo` in FFmpeg 9: `atrim` ended the audio, and through
    // `-shortest` the video, at the first clip's length.
    let base = format!(
        "{graph};[outa]apad=whole_dur={duration:.6},aresample=48000,aformat=channel_layouts=stereo"
    );
    let mut measure: Vec<OsString> = vec!["-hide_banner".into(), "-nostats".into()];
    measure.extend(audio_inputs(clips));
    measure.extend([
        "-filter_complex".into(),
        format!("{base},{}[m]", loudnorm_measure()).into(),
        "-map".into(),
        "[m]".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]);
    let (_, stderr) = capture_with_diagnostics(&tools.ffmpeg, measure, Duration::from_secs(1800))?;
    let normalize = match parse_measurement(&stderr) {
        // Silence measures -inf: keep it as it is.
        Some(m) if m.input_i.is_finite() && m.input_i > -70.0 => format!(",{}", loudnorm_apply(m)),
        _ => String::new(),
    };
    let mut mix: Vec<OsString> = vec!["-y".into(), "-hide_banner".into(), "-nostats".into()];
    mix.extend(audio_inputs(clips));
    mix.extend([
        "-filter_complex".into(),
        format!("{base}{normalize},aresample=48000[mix]").into(),
        "-map".into(),
        "[mix]".into(),
        "-c:a".into(),
        "pcm_s16le".into(),
        output.as_os_str().to_owned(),
    ]);
    capture_with_diagnostics(&tools.ffmpeg, mix, Duration::from_secs(1800))?;
    Ok(Some(output.to_path_buf()))
}

/// Video stream copied, audio encoded to AAC by the preset.
pub fn mux(
    tools: &MediaTools,
    video: &Path,
    audio: Option<&Path>,
    preset: ExportPreset,
    output: &Path,
) -> Result<(), MediaError> {
    let mut args: Vec<OsString> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-i".into(),
        video.into(),
    ];
    if let Some(audio) = audio {
        args.extend([
            "-i".into(),
            audio.into(),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "1:a:0".into(),
        ]);
        args.extend(preset_args(preset).audio.into_iter().map(OsString::from));
        args.push("-shortest".into());
    }
    args.extend([
        "-c:v".into(),
        "copy".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    args.push(output.into());
    capture_with_diagnostics(&tools.ffmpeg, args, Duration::from_secs(1800))?;
    Ok(())
}

/// BGRA frame → PNG (reference frames for the PSNR check).
pub fn save_bgra_png(path: &Path, bgra: &[u8], width: u32, height: u32) -> Result<(), MediaError> {
    let mut rgba = bgra.to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        px[3] = 255;
    }
    let image =
        image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| invalid("frame size"))?;
    image.save(path).map_err(|e| invalid(e.to_string()))
}

/// Inspect the export: format, timing, loudness, encoder fidelity against the
/// renderer's own first frames, black and frozen stretches.
pub fn timeline_report(
    tools: &MediaTools,
    output: &Path,
    fps: f64,
    duration: f64,
    has_audio: bool,
    references: Option<&Path>,
) -> Result<(QualityReport, PathBuf), MediaError> {
    let mut report = inspect(
        &tools.ffprobe,
        output,
        Some(QualityTargets {
            fps,
            duration,
            frame_tolerance: 1.0 / fps,
        }),
    )?;
    if has_audio {
        report.loudness = measure_loudness(&tools.ffmpeg, output)?;
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
    if let Some(references) = references {
        report.psnr_db = measure_psnr(&tools.ffmpeg, &references.join("%06d.png"), fps, output)?;
        if let Some(psnr) = report.psnr_db {
            report.checks.push(QualityCheck {
                name: "psnr".into(),
                passed: psnr >= 40.0,
                measured: Some(psnr),
                expected: Some("≥ 40 dB".into()),
                message: "encoded frames vs the renderer's frames".into(),
            });
        }
    }
    report
        .checks
        .extend(measure_black_freeze(&tools.ffmpeg, output)?);
    let report_path = output.with_extension("report.json");
    write_report(&report_path, &report)?;
    Ok((report, report_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real FFmpeg: a retimed, delayed clip must not cut the mix short.
    #[test]
    fn mixed_audio_lasts_the_whole_timeline_with_retimed_and_delayed_clips() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tools = MediaTools::discover(&root);
        if !tools.ffmpeg.is_file() || !tools.ffprobe.is_file() {
            eprintln!("skipped: no FFmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("todo-mix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tone = |name: &str, seconds: u32, rate: u32| {
            let path = dir.join(name);
            let status = std::process::Command::new(&tools.ffmpeg)
                .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
                .arg(format!("sine=f=440:d={seconds}:sample_rate={rate}"))
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
            path.to_string_lossy().into_owned()
        };
        let clip = |id: &str, path: String, start: f64, duration: f64, rate: f64| {
            ResolvedAudioClip {
                id: id.into(),
                path,
                start,
                duration,
                offset: 0.0,
                rate,
                gain: 0.3,
                fade_in: 0.5,
                fade_out: 0.5,
            }
        };
        // The first clip is the shortest and starts late; one clip is retimed.
        let clips = vec![
            clip("late", tone("a.wav", 10, 44100), 6.0, 3.0, 1.0),
            clip("bed", tone("b.wav", 10, 44100), 0.0, 5.0, 1.5),
            clip("voice", tone("c.wav", 12, 24000), 0.5, 9.0, 1.0),
        ];
        let out = dir.join("mix.wav");
        mix_audio(&tools, &clips, 10.0, &out).unwrap().unwrap();
        let probe = std::process::Command::new(&tools.ffprobe)
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(&out)
            .output()
            .unwrap();
        let seconds: f64 = String::from_utf8_lossy(&probe.stdout).trim().parse().unwrap();
        assert!((seconds - 10.0).abs() < 0.05, "mix lasted {seconds}s");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn loudnorm_measurements_are_parsed_from_ffmpeg_output() {
        let stderr = "[Parsed_loudnorm_0 @ 0]\n{\n\t\"input_i\" : \"-23.10\",\n\t\"input_tp\" : \"-5.20\",\n\t\"input_lra\" : \"3.40\",\n\t\"input_thresh\" : \"-33.30\",\n\t\"output_i\" : \"-16.0\",\n\t\"target_offset\" : \"0.10\"\n}\n";
        let m = parse_measurement(stderr).unwrap();
        assert_eq!((m.input_i, m.input_tp, m.target_offset), (-23.1, -5.2, 0.1));
    }
}
