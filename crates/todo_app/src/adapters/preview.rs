use super::{
    desktop::window::{egui_rect_to_screen, viewport_hwnd},
    paths::{preview_host_exe, resolve_media_path},
};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use todo_application::{AudioClockSample, PreviewCommand};
use todo_preview::{
    AlignGuide, AudioClip, ClipRect, HostEvent, PreviewHost, PreviewLayer as HostLayer, ScreenSlot,
    TransformGizmo, VisualTransform,
};
use todo_ui::MonitorHole;

mod proxies;

pub struct PreviewBridge {
    host: Option<PreviewHost>,
    repo: PathBuf,
    hwnd: Option<u64>,
    slot: Option<ScreenSlot>,
    /// Last cut-outs sent for egui popups over the preview window.
    clip: Option<Vec<ClipRect>>,
    gizmo: Option<Option<TransformGizmo>>,
    guides: Option<Vec<AlignGuide>>,
    viewport: Option<todo_preview::PreviewViewport>,
    source_generation: u64,
    source_path: Option<String>,
    /// Parent is (or was) minimized; keep the popup hidden until we can track the hole again.
    iconic: bool,
    /// Remaining frames of forced repaint so the popup tracks the restore animation.
    restore_repaints: u8,
    sources: proxies::PreviewSources,
    started: Instant,
}
impl PreviewBridge {
    pub fn new(repo: &Path) -> Self {
        Self {
            host: None,
            repo: repo.to_owned(),
            hwnd: None,
            slot: None,
            clip: None,
            gizmo: None,
            guides: None,
            viewport: None,
            source_generation: 0,
            source_path: None,
            iconic: false,
            restore_repaints: 0,
            sources: Default::default(),
            started: Instant::now(),
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
        self.clip = None;
        self.gizmo = None;
        self.guides = None;
        self.viewport = None;
        self.source_path = None;
        self.source_generation = 0;
        self.iconic = false;
        self.restore_repaints = 0;
        self.sources.reset_presentation();
        Ok(true)
    }
    /// True while the parent is restoring from minimize — paint every frame with the UI.
    pub fn syncing_restore(&self) -> bool {
        self.iconic || self.restore_repaints > 0
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
                    self.sources.reset_presentation();
                    let _ = self
                        .sources
                        .take_changed(self.started.elapsed().as_secs_f64());
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
                    self.sources
                        .layers(layers, self.started.elapsed().as_secs_f64());
                    // Flush after the loop so transport and picture in one
                    // application update select one consistent representation.
                    Ok(())
                }
                PreviewCommand::Transport { time, playing } => {
                    self.sources
                        .transport(time, playing, self.started.elapsed().as_secs_f64());
                    host.transport(time, playing)
                }
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
                            fade_in: c.fade_in,
                            fade_out: c.fade_out,
                        })
                        .collect::<Vec<_>>(),
                ),
                PreviewCommand::Quit => host.quit(),
            };
            result.map_err(|e| e.to_string())?;
        }
        self.refresh_edit_sources()
    }

    pub fn install_edit_proxy(&mut self, source: String, proxy: PathBuf) {
        self.sources
            .install(source, proxy, self.started.elapsed().as_secs_f64());
    }

    pub fn clear_edit_sources(&mut self) {
        self.sources = Default::default();
    }

    pub fn finish_scrub(&mut self) -> Result<(), String> {
        self.sources.finish_scrub();
        self.refresh_edit_sources()
    }

    /// Settling must also run without new UI input, so the precise original
    /// replaces an editing proxy after the user pauses on a time.
    pub fn refresh_edit_sources(&mut self) -> Result<(), String> {
        let Some(host) = &self.host else {
            return Ok(());
        };
        let Some(layers) = self
            .sources
            .take_changed(self.started.elapsed().as_secs_f64())
        else {
            return Ok(());
        };
        self.gizmo = None;
        self.guides = None;
        self.source_generation = self.source_generation.wrapping_add(1);
        self.source_path = self.sources.top_source().map(str::to_owned);
        let host_layers: Vec<HostLayer> = layers
            .iter()
            .map(|layer| host_layer(&self.repo, layer))
            .collect();
        host.set_layers(&host_layers).map_err(|e| e.to_string())
    }
    /// Place the native preview over the monitor hole. `overlays` are egui
    /// popups drawn over the hole this frame (see `todo_ui::overlays_over`):
    /// the preview window leaves them transparent instead of covering them.
    pub fn sync_popup(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        hole: Option<MonitorHole>,
        overlays: &[egui::Rect],
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
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized {
            // Hide in place — do not shrink to the iconic client rect, or restore
            // will grow the popup from a center stub out of sync with the UI.
            self.iconic = true;
            self.restore_repaints = 0;
            Self::hide_keeping_geometry(host, &mut self.slot);
            return;
        }
        if self.iconic {
            self.iconic = false;
            // Follow the window restore animation frame-for-frame with egui layout.
            self.restore_repaints = 45;
        } else if self.restore_repaints > 0 {
            self.restore_repaints -= 1;
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
        let ppp = ctx.pixels_per_point();
        let cuts: Vec<ClipRect> = overlays
            .iter()
            .filter_map(|overlay| {
                let local = overlay.translate(-rect.min.to_vec2());
                let x0 = (local.min.x * ppp).floor() as i32;
                let y0 = (local.min.y * ppp).floor() as i32;
                let x1 = (local.max.x * ppp).ceil() as i32;
                let y1 = (local.max.y * ppp).ceil() as i32;
                (x1 > x0 && y1 > y0).then_some(ClipRect {
                    x: x0,
                    y: y0,
                    w: x1 - x0,
                    h: y1 - y0,
                })
            })
            .collect();
        // Too many popups at once: hide rather than cover any of them.
        let too_many = cuts.len() > todo_preview::MAX_CLIP_RECTS;
        let slot = ScreenSlot {
            visible: slot.visible && !too_many,
            ..slot
        };
        if slot.visible && self.clip.as_deref() != Some(cuts.as_slice()) && host.clip(&cuts).is_ok()
        {
            self.clip = Some(cuts);
        }
        if !slot.visible {
            // A modal may open without changing the monitor geometry. Always
            // resend the hide command in that state; relying only on the
            // cached slot lets the independent HWND remain above egui.
            if host.size_screen(slot).is_ok() {
                self.slot = Some(slot);
            }
        } else if self.slot != Some(slot) && host.size_screen(slot).is_ok() {
            self.slot = Some(slot);
        }
    }

    fn hide_keeping_geometry(host: &PreviewHost, slot: &mut Option<ScreenSlot>) {
        let Some(previous) = *slot else {
            return;
        };
        if !previous.visible {
            return;
        }
        let hidden = ScreenSlot {
            visible: false,
            ..previous
        };
        if host.size_screen(hidden).is_ok() {
            *slot = Some(hidden);
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

/// An application layer as the native host draws it (preview and export).
pub fn host_layer(repo: &std::path::Path, layer: &todo_application::PreviewLayer) -> HostLayer {
    HostLayer {
        path: resolve_media_path(repo, &layer.path)
            .to_string_lossy()
            .into_owned(),
        media_time: layer.media_time,
        soft: layer.soft.unwrap_or(0.0),
        kind: match layer.kind {
            todo_application::PreviewLayerKind::Video => todo_preview::LayerKind::Video,
            todo_application::PreviewLayerKind::Image => todo_preview::LayerKind::Image,
            todo_application::PreviewLayerKind::Packed => todo_preview::LayerKind::Packed,
            todo_application::PreviewLayerKind::Reveal => todo_preview::LayerKind::Reveal,
            todo_application::PreviewLayerKind::Text => todo_preview::LayerKind::Text,
        },
        transform: VisualTransform {
            x: layer.transform.x,
            y: layer.transform.y,
            scale: layer.transform.scale,
            scale_x: layer.transform.scale_x,
            scale_y: layer.transform.scale_y,
            opacity: layer.transform.opacity,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_layer_budget_matches_the_protocol() {
        // core owns the limit; the protocol (and native host) must accept every stack it builds.
        assert_eq!(
            todo_application::MAX_PREVIEW_LAYERS,
            todo_preview::MAX_PREVIEW_LAYERS
        );
    }

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

    #[test]
    fn restore_tracking_requests_fast_repaints_with_the_ui() {
        let mut bridge = PreviewBridge::new(Path::new("."));
        assert!(!bridge.syncing_restore());
        bridge.iconic = true;
        assert!(bridge.syncing_restore());
        bridge.iconic = false;
        bridge.restore_repaints = 3;
        assert!(bridge.syncing_restore());
        bridge.restore_repaints = 0;
        assert!(!bridge.syncing_restore());
    }
}
