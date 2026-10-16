use super::{
    desktop::window::{egui_rect_to_screen, viewport_hwnd},
    paths::{preview_host_exe, resolve_media_path},
};
use eframe::egui;
use std::path::{Path, PathBuf};
use todo_application::{AudioClockSample, PreviewCommand};
use todo_preview::{
    AlignGuide, AudioClip, HostEvent, PreviewHost, PreviewLayer as HostLayer, ScreenSlot,
    TransformGizmo, VisualTransform,
};
use todo_ui::MonitorHole;

pub struct PreviewBridge {
    host: Option<PreviewHost>,
    repo: PathBuf,
    hwnd: Option<u64>,
    slot: Option<ScreenSlot>,
    gizmo: Option<Option<TransformGizmo>>,
    guides: Option<Vec<AlignGuide>>,
    viewport: Option<todo_preview::PreviewViewport>,
    source_generation: u64,
    source_path: Option<String>,
}
impl PreviewBridge {
    pub fn new(repo: &Path) -> Self {
        Self {
            host: None,
            repo: repo.to_owned(),
            hwnd: None,
            slot: None,
            gizmo: None,
            guides: None,
            viewport: None,
            source_generation: 0,
            source_path: None,
        }
    }
    pub fn ensure(&mut self) -> Result<bool, String> {
        if self.host.as_ref().is_some_and(PreviewHost::is_running) {
            return Ok(false);
        }
        self.host = None;
        self.host =
            Some(PreviewHost::spawn_at(preview_host_exe(&self.repo)).map_err(|e| e.to_string())?);
        self.hwnd = None;
        self.slot = None;
        self.gizmo = None;
        self.guides = None;
        self.viewport = None;
        self.source_path = None;
        Ok(true)
    }
    pub fn poll(&self) -> Option<HostEvent> {
        self.host.as_ref()?.try_recv()
    }
    /// Resolve a decoder reply only against the currently requested source.
    /// The interaction layer receives the original project path, never a desktop path.
    pub fn source_path(&self, generation: u64) -> Option<&str> {
        (generation == self.source_generation)
            .then_some(self.source_path.as_deref())
            .flatten()
    }
    pub fn clock(&self) -> Option<AudioClockSample> {
        self.host.as_ref()?.read_clock().map(|c| AudioClockSample {
            time: c.time,
            playing: c.playing,
            available: c.available,
        })
    }
    pub fn apply(&mut self, commands: Vec<PreviewCommand>) -> Result<(), String> {
        let Some(host) = &self.host else {
            return Ok(());
        };
        for command in commands {
            let result = match command {
                PreviewCommand::Path(path) => {
                    self.gizmo = None;
                    self.guides = None;
                    let generation = self.source_generation.wrapping_add(1);
                    let absolute = path.as_ref().map(|p| {
                        resolve_media_path(&self.repo, p)
                            .to_string_lossy()
                            .into_owned()
                    });
                    host.set_path_with_generation(absolute.as_deref(), generation)
                        .map_err(|e| e.to_string())?;
                    self.source_generation = generation;
                    self.source_path = path;
                    Ok(())
                }
                PreviewCommand::Seek(t) => host.seek(t),
                PreviewCommand::Aspect { width, height } => {
                    host.aspect(width as f64, height as f64)
                }
                PreviewCommand::Transform(t) => host.set_transform(VisualTransform {
                    x: t.x,
                    y: t.y,
                    scale: t.scale,
                    scale_x: t.scale_x,
                    scale_y: t.scale_y,
                    opacity: t.opacity,
                }),
                PreviewCommand::Layers(layers) => {
                    self.gizmo = None;
                    self.guides = None;
                    let generation = self.source_generation.wrapping_add(1);
                    self.source_generation = generation;
                    self.source_path = layers.last().map(|layer| layer.path.clone());
                    let host_layers: Vec<HostLayer> = layers
                        .into_iter()
                        .map(|layer| HostLayer {
                            path: resolve_media_path(&self.repo, &layer.path)
                                .to_string_lossy()
                                .into_owned(),
                            media_time: layer.media_time,
                            transform: VisualTransform {
                                x: layer.transform.x,
                                y: layer.transform.y,
                                scale: layer.transform.scale,
                                scale_x: layer.transform.scale_x,
                                scale_y: layer.transform.scale_y,
                                opacity: layer.transform.opacity,
                            },
                        })
                        .collect();
                    host.set_layers(&host_layers)
                }
                PreviewCommand::Transport { time, playing } => host.transport(time, playing),
                PreviewCommand::Audio(clips) => host.set_audio(
                    &clips
                        .into_iter()
                        .map(|c| AudioClip {
                            path: resolve_media_path(&self.repo, &c.path)
                                .to_string_lossy()
                                .into_owned(),
                            start: c.start,
                            duration: c.duration,
                            offset: c.offset,
                            rate: c.rate,
                            gain: c.gain,
                        })
                        .collect::<Vec<_>>(),
                ),
                PreviewCommand::Quit => host.quit(),
            };
            result.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn sync_popup(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        hole: Option<MonitorHole>,
        active: bool,
    ) {
        let (Some(host), Some(hwnd)) = (&self.host, viewport_hwnd(frame)) else {
            return;
        };
        let id = hwnd as u64;
        if self.hwnd != Some(id) && host.hwnd(id).is_ok() {
            self.hwnd = Some(id);
            self.slot = None;
        }
        let (rect, visible) = match hole {
            Some(h) => (
                egui::Rect::from_min_size(egui::pos2(h.x, h.y), egui::vec2(h.w, h.h)),
                active && h.w > 1.0 && h.h > 1.0,
            ),
            None => (
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO),
                false,
            ),
        };
        let screen = egui_rect_to_screen(hwnd, rect, ctx.pixels_per_point());
        let slot = ScreenSlot {
            x: screen.x,
            y: screen.y,
            w: screen.w,
            h: screen.h,
            visible: screen.visible && visible,
        };
        if self.slot != Some(slot) && host.size_screen(slot).is_ok() {
            self.slot = Some(slot);
        }
    }
    /// Transient selection decoration is projected after viewport inputs have been
    /// applied; it is separate from persisted transforms and editing history.
    pub fn sync_view(&mut self, view: Option<todo_interaction::PreviewView>) -> Result<(), String> {
        let Some(host) = &self.host else {
            return Ok(());
        };
        if let Some(view) = view.as_ref() {
            if let Some(projection) = view.projection {
                let viewport = todo_preview::PreviewViewport {
                    left: projection.left,
                    top: projection.top,
                    width: projection.width,
                    height: projection.height,
                    show_outside: !view.viewport.output_only(),
                };
                if self.viewport != Some(viewport) {
                    host.set_viewport(viewport).map_err(|e| e.to_string())?;
                    self.viewport = Some(viewport);
                }
            }
        }
        let gizmo = view.as_ref().and_then(|view| {
            view.selection.map(|b| TransformGizmo {
                left: b.left,
                top: b.top,
                right: b.right,
                bottom: b.bottom,
                editable: view.editable,
            })
        });
        if self.gizmo != Some(gizmo) {
            host.set_gizmo(gizmo).map_err(|e| e.to_string())?;
            self.gizmo = Some(gizmo);
        }
        let guides: Vec<AlignGuide> = view
            .as_ref()
            .map(|view| {
                view.align_guides
                    .iter()
                    .map(|g| AlignGuide {
                        vertical: g.vertical,
                        position: g.position,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self.guides.as_ref() != Some(&guides) {
            host.set_guides(&guides).map_err(|e| e.to_string())?;
            self.guides = Some(guides);
        }
        Ok(())
    }
    pub fn shutdown(&mut self) {
        if let Some(host) = self.host.take() {
            let _ = host.quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_geometry_maps_only_the_current_request_to_its_project_path() {
        let mut bridge = PreviewBridge::new(Path::new("."));
        assert_eq!(bridge.source_path(0), None);
        bridge.source_generation = 7;
        bridge.source_path = Some("data/media/source.mp4".into());
        assert_eq!(bridge.source_path(7), Some("data/media/source.mp4"));
        assert_eq!(bridge.source_path(6), None);
        // Reopening the same path is a distinct request. Its older reply is stale.
        bridge.source_generation = 8;
        assert_eq!(bridge.source_path(7), None);
        assert_eq!(bridge.source_path(8), Some("data/media/source.mp4"));
        bridge.source_path = None;
        assert_eq!(bridge.source_path(8), None);
    }
}
