//! Offscreen program frames for review: the same layers, decoders and shaders
//! as the window preview, drawn by `preview_host.exe --render`.
//!
//! Agents use this to look at what they just built: render a second, read the
//! PNG, then judge or fix. Full canvas size; downscale on the caller side.
use super::paths::preview_host_exe;
use super::preview::host_layer;
use std::path::{Path, PathBuf};
use todo_application::{export_layers, picture_layers, Project};
use todo_media::export::timeline::save_bgra_png;
use todo_media::{RasterRequest, TextRasterizer};
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
