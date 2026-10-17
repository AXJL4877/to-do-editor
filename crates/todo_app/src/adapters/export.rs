//! Timeline export: every frame is drawn by the native offline renderer with
//! the preview's own layers, decoders and shaders, then encoded by FFmpeg;
//! the audio is mixed from the same clips the preview plays and normalized
//! to -16 LUFS. A quality report is written next to the video.
use super::paths::{preview_host_exe, resolve_media_path};
use super::preview::host_layer;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use todo_application::{
    content_duration, export_layers, picture_layers, resolve_audio_clips, Project,
};
use todo_media::export::timeline::{mix_audio, mux, save_bgra_png, timeline_report, VideoSink};
use todo_media::{ExportPreset, MediaTools, RasterRequest, TextRasterizer};
use todo_preview::OfflineRenderer;

/// Frames kept as references for the encoder-fidelity (PSNR) check.
const REFERENCE_FRAMES: usize = 30;

#[derive(Debug, Clone)]
pub struct ExportOutcome {
    pub output: PathBuf,
    pub report: PathBuf,
    pub frames: usize,
    pub failed_checks: Vec<String>,
}

pub fn export_timeline(
    root: &Path,
    project: &Project,
    output: &Path,
    preset: ExportPreset,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<ExportOutcome, String> {
    let tools = MediaTools::discover(root);
    if !tools.ffmpeg.is_file() {
        return Err("找不到 FFmpeg，无法导出".into());
    }
    let (width, height) = project.output_size();
    let fps = project.fps;
    let duration = content_duration(project);
    if !(duration.is_finite() && duration > 0.0) {
        return Err("时间轴是空的".into());
    }
    let frames = (duration * fps).ceil() as usize;
    let parts = output.with_extension("parts");
    let _ = std::fs::remove_dir_all(&parts);
    std::fs::create_dir_all(parts.join("ref")).map_err(|e| e.to_string())?;
    let result = (|| -> Result<ExportOutcome, String> {
        let video = parts.join("video.mp4");
        let mut sink = VideoSink::start(&tools, width, height, fps, preset, &video)
            .map_err(|e| e.to_string())?;
        let mut renderer = match OfflineRenderer::start(&preview_host_exe(root), width, height) {
            Ok(renderer) => renderer,
            Err(e) => {
                sink.abort();
                return Err(format!("无法启动渲染：{e}"));
            }
        };
        let mut rasterizer = TextRasterizer::new(root);
        let mut frame = Vec::new();
        for index in 0..frames {
            if cancel.load(Ordering::Relaxed) {
                renderer.abort();
                sink.abort();
                return Err("已取消导出".into());
            }
            let time = index as f64 / fps;
            let rendered = (|| -> Result<(), String> {
                let hits = export_layers(project, time)?;
                let layers = picture_layers(project, hits, &mut |_, raster| {
                    let request = RasterRequest {
                        content: &raster.content,
                        style: &raster.style,
                        canvas: raster.canvas,
                        reveal: raster.reveal,
                    };
                    rasterizer
                        .raster(&raster.digest, request)
                        .ok()
                        .map(|(path, _)| path.to_string_lossy().into_owned())
                });
                let host: Vec<_> = layers.iter().map(|l| host_layer(root, l)).collect();
                renderer
                    .frame(&host, &mut frame)
                    .map_err(|e| e.to_string())?;
                sink.push(&frame).map_err(|e| e.to_string())?;
                if index < REFERENCE_FRAMES {
                    save_bgra_png(
                        &parts.join(format!("ref/{:06}.png", index + 1)),
                        &frame,
                        width,
                        height,
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok(())
            })();
            if let Err(error) = rendered {
                let detail = renderer.abort();
                sink.abort();
                return Err(format!(
                    "第 {index} 帧（{time:.2}s）导出失败：{error} {}",
                    detail.trim()
                ));
            }
            progress(index + 1, frames);
        }
        renderer.finish()?;
        sink.finish().map_err(|e| e.to_string())?;
        let mut clips = resolve_audio_clips(project);
        for clip in &mut clips {
            clip.path = resolve_media_path(root, &clip.path)
                .to_string_lossy()
                .into_owned();
        }
        let audio = mix_audio(
            &tools,
            &clips,
            frames as f64 / fps,
            &parts.join("audio.wav"),
        )
        .map_err(|e| e.to_string())?;
        mux(&tools, &video, audio.as_deref(), preset, output).map_err(|e| e.to_string())?;
        let (report, report_path) = timeline_report(
            &tools,
            output,
            fps,
            frames as f64 / fps,
            audio.is_some(),
            Some(&parts.join("ref")),
        )
        .map_err(|e| e.to_string())?;
        Ok(ExportOutcome {
            output: output.to_path_buf(),
            report: report_path,
            frames,
            failed_checks: report
                .checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| format!("{}：{}", c.name, c.message))
                .collect(),
        })
    })();
    // Diagnostics: keep the intermediate files when asked.
    if std::env::var_os("TODO_EXPORT_KEEP_PARTS").is_none() {
        let _ = std::fs::remove_dir_all(&parts);
    }
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}
