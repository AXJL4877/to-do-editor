//! Offscreen program frames for review: the same layers, decoders and shaders
//! as the window preview, drawn by `preview_host.exe --render`.
//!
//! Agents use this to look at what they just built: render a second, read the
//! PNG, then judge or fix. Full canvas size; downscale on the caller side.
use super::paths::preview_host_exe;
use super::preview::host_layer;
use std::path::{Path, PathBuf};
use todo_application::{export_layers, picture_layers, Project};
use todo_core::{content_duration, workspace_duration};
use todo_media::export::timeline::save_bgra_png;
use todo_media::{stitch_contact_sheet, RasterRequest, TextRasterizer};
use todo_preview::OfflineRenderer;

#[derive(Debug, Clone)]
pub struct FrameRender {
    pub time: f64,
    pub png: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// Render `times` (seconds) from `project` into `output_dir`.
pub fn render_frames(
    root: &Path,
    project: &Project,
    times: &[f64],
    output_dir: &Path,
) -> Result<Vec<FrameRender>, String> {
    if times.is_empty() {
        return Err("没有要渲染的时间点".into());
    }
    let width = project.width;
    let height = project.height;
    if width == 0 || height == 0 {
        return Err("画布尺寸无效".into());
    }
    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;

    let mut renderer = OfflineRenderer::start(&preview_host_exe(root), width, height)
        .map_err(|e| format!("无法启动渲染：{e}"))?;
    let mut rasterizer = TextRasterizer::new(root);
    let mut frame = Vec::new();
    let mut out = Vec::with_capacity(times.len());

    for (index, time) in times.iter().enumerate() {
        let rendered = (|| -> Result<PathBuf, String> {
            let hits = export_layers(project, *time)?;
            let layers = picture_layers(project, hits, &mut |_, raster| {
                rasterizer
                    .raster(
                        &raster.digest,
                        RasterRequest {
                            content: &raster.content,
                            style: &raster.style,
                            canvas: raster.canvas,
                            reveal: raster.reveal,
                        },
                    )
                    .ok()
                    .map(|(path, _)| path.to_string_lossy().into_owned())
            });
            let host: Vec<_> = layers.iter().map(|layer| host_layer(root, layer)).collect();
            renderer
                .frame(&host, &mut frame)
                .map_err(|error| error.to_string())?;
            let path = output_dir.join(format!("frame-{index:02}.png"));
            save_bgra_png(&path, &frame, width, height).map_err(|e| e.to_string())?;
            Ok(path)
        })();
        match rendered {
            Ok(png) => out.push(FrameRender {
                time: *time,
                png,
                width,
                height,
            }),
            Err(error) => {
                let detail = renderer.abort();
                return Err(format!("{time:.2}s 渲染失败：{error} {}", detail.trim()));
            }
        }
    }
    renderer.finish()?;
    Ok(out)
}

/// Absolute timeline sample times: frame 0, every_frames, 2×every_frames, …
/// until `end` (seconds). `fps` comes from the project clock.
pub fn absolute_frame_times(end_secs: f64, fps: f64, every_frames: u32, max_frames: u32) -> Vec<f64> {
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
    let every = every_frames.max(1);
    let max = max_frames.clamp(1, 64) as usize;
    let end = if end_secs.is_finite() && end_secs > 0.0 {
        end_secs
    } else {
        return Vec::new();
    };
    let dt = every as f64 / fps;
    let mut times = Vec::new();
    let mut frame: u64 = 0;
    while times.len() < max {
        let t = frame as f64 / fps;
        if t >= end - 1e-9 {
            break;
        }
        times.push(t);
        frame = frame.saturating_add(u64::from(every));
        // Guard against zero-step / float stalls.
        if dt <= 0.0 {
            break;
        }
    }
    times
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripRange {
    Content,
    Workspace,
}

impl StripRange {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "workspace" => Self::Workspace,
            _ => Self::Content,
        }
    }

    fn end(self, project: &Project) -> f64 {
        match self {
            Self::Content => content_duration(project).max(1.0 / project.fps.max(1.0)),
            Self::Workspace => workspace_duration(project),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimelineStrip {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub every_frames: u32,
    pub frames: Vec<FrameRender>,
}

/// Render absolute-clock frames and stitch them into one contact sheet.
pub fn render_timeline_strip(
    root: &Path,
    project: &Project,
    output_dir: &Path,
    every_frames: u32,
    max_frames: u32,
    thumb_height: u32,
    range: StripRange,
) -> Result<TimelineStrip, String> {
    let end = range.end(project);
    let times = absolute_frame_times(end, project.fps, every_frames, max_frames);
    if times.is_empty() {
        return Err("时间轴上没有可截取的内容（内容时长为 0）".into());
    }
    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let frames_dir = output_dir.join("frames");
    let frames = render_frames(root, project, &times, &frames_dir)?;
    let sheet = output_dir.join("strip.png");
    let columns = ((frames.len() as f64).sqrt().ceil() as u32).clamp(1, 8);
    let (width, height) = stitch_contact_sheet(
        &frames.iter().map(|f| f.png.as_path()).collect::<Vec<_>>(),
        &sheet,
        thumb_height,
        columns,
    )
    .map_err(|e| e.to_string())?;
    Ok(TimelineStrip {
        path: sheet,
        width,
        height,
        fps: project.fps,
        every_frames,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_times_follow_frame_clock() {
        // 30fps, every 30 frames → 0s, 1s, 2s before end=3s
        assert_eq!(
            absolute_frame_times(3.0, 30.0, 30, 24),
            vec![0.0, 1.0, 2.0]
        );
        // 24fps, every 30 frames → 0, 1.25, 2.5 before end=3
        let times = absolute_frame_times(3.0, 24.0, 30, 24);
        assert_eq!(times.len(), 3);
        assert!((times[1] - 1.25).abs() < 1e-9);
    }
}
