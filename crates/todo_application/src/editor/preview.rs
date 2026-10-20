use super::*;
use crate::text_rasters::{RasterOutcome, RasterState, TextRaster};
use todo_core::{
    resolve_audio_clips, resolve_layers, resolve_preview, BaseSize, HitSource, PreviewHit,
    TrackKind,
};

impl EditorSession {
    pub fn sync(&mut self) -> Vec<PreviewCommand> {
        self.last_audio = None;
        self.last_path = None;
        self.last_seek_ms = None;
        self.last_aspect = None;
        self.last_transform = None;
        self.last_layers = None;
        self.last_transport = None;
        self.emit(Want {
            audio: true,
            transport: true,
            picture: true,
        })
    }
    pub fn deactivate(&mut self) -> Vec<PreviewCommand> {
        self.playing = false;
        self.last_tick = None;
        self.clock = None;
        self.bump_clock();
        self.last_layers = None;
        vec![
            PreviewCommand::Transport {
                time: self.playhead,
                playing: false,
            },
            PreviewCommand::Audio(Vec::new()),
            PreviewCommand::Layers(Vec::new()),
            PreviewCommand::Path(None),
        ]
    }
    pub fn shutdown(&mut self) -> Vec<PreviewCommand> {
        let mut commands = self.deactivate();
        commands.push(PreviewCommand::Quit);
        commands
    }
    pub(crate) fn emit(&mut self, want: Want) -> Vec<PreviewCommand> {
        let mut commands = Vec::new();
        let output = self.project.output_size();
        if self.last_aspect != Some(output) {
            commands.push(PreviewCommand::Aspect {
                width: output.0,
                height: output.1,
            });
            self.last_aspect = Some(output);
        }
        let mut transport = want.transport;
        let mut force_transport = false;
        if want.audio {
            // Source pane is visual-only; mute the program mix while previewing library media.
            let audio = if self.showing_source() {
                Vec::new()
            } else {
                resolve_audio_clips(&self.project)
            };
            if self.last_audio.as_ref() != Some(&audio) {
                commands.push(PreviewCommand::Audio(audio.clone()));
                self.last_audio = Some(audio);
                transport = true;
                force_transport = true;
            }
        }
        if transport {
            let time = self.preview_time();
            let key = ((time * 1000.0).round() as i64, self.playing);
            if force_transport || self.last_transport != Some(key) {
                commands.push(PreviewCommand::Transport {
                    time: time.max(0.0),
                    playing: self.playing,
                });
                self.last_transport = Some(key);
                self.bump_clock();
            }
        }
        if want.picture {
            let layers = self.preview_layers();
            if self.last_layers.as_ref() != Some(&layers) {
                commands.push(PreviewCommand::Layers(layers.clone()));
                self.last_layers = Some(layers);
                // Legacy single-source caches are superseded by the stack.
                self.last_path = None;
                self.last_seek_ms = None;
                self.last_transform = None;
            }
        }
        commands
    }
    pub(crate) fn bump_clock(&mut self) {
        self.clock_epoch = self.clock_epoch.wrapping_add(1);
        self.clock = None;
        self.clock_missing_at = None;
    }
    pub(crate) fn preview_time(&self) -> f64 {
        if self.showing_source() {
            self.source_monitor.as_ref().map(|s| s.time).unwrap_or(0.0)
        } else {
            self.playhead
        }
    }

    /// Available picture layers at the playhead. Pending text must not hold up
    /// video; a compatible image from the same clip can bridge reveal frames.
    fn preview_layers(&mut self) -> Vec<PreviewLayer> {
        if self.showing_source() {
            self.text_rasters.retain_shown(|_| false);
            return self.source_layers();
        }
        let hits = resolve_layers(&self.project, self.playhead);
        self.text_rasters
            .retain_shown(|clip_id| hits.iter().any(|hit| hit.clip.id == clip_id));
        let rasters = &mut self.text_rasters;
        picture_layers(&self.project, hits, &mut |clip_id, raster| match rasters
            .preview_state(clip_id, raster)
        {
            RasterState::Ready(path) => Some(path),
            RasterState::Pending | RasterState::Failed => None,
        })
    }

    fn source_layers(&self) -> Vec<PreviewLayer> {
        let Some(source) = &self.source_monitor else {
            return Vec::new();
        };
        let Some(media) = self.project.media.iter().find(|m| m.id == source.media_id) else {
            return Vec::new();
        };
        if !todo_core::is_visual_media(media) {
            return Vec::new();
        }
        let Some(path) = media.path.as_ref().filter(|p| !p.trim().is_empty()) else {
            return Vec::new();
        };
        let aspect = match (media.width, media.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        };
        vec![PreviewLayer {
            path: path.clone(),
            media_time: source.time.max(0.0),
            aspect,
            kind: layer_kind(media),
            transform: ClipTransform::default(),
            soft: todo_core::reveal_frame(media, source.time).map(|f| f.soft),
        }]
    }

    /// The stack most recently sent to the preview host.
    pub fn shown_layers(&self) -> &[PreviewLayer] {
        self.last_layers.as_deref().unwrap_or_default()
    }
    /// Text rasters requested since the last call; the workspace turns them into jobs.
    pub(crate) fn take_text_rasters(&mut self) -> Vec<TextRaster> {
        self.text_rasters.take_wanted()
    }
    /// A worker finished a raster; redraw if the current stack was waiting for it.
    pub(crate) fn text_raster_done(
        &mut self,
        digest: String,
        outcome: RasterOutcome,
    ) -> EditorUpdate {
        self.text_rasters.complete(digest, outcome);
        EditorUpdate {
            commands: self.emit(Want {
                audio: false,
                transport: false,
                picture: true,
            }),
            ..EditorUpdate::default()
        }
    }
    /// Identity-transform size of a text layer, known once it has been rendered.
    pub fn text_base(&self, hit: &PreviewHit) -> Option<BaseSize> {
        let HitSource::Text { style, .. } = &hit.source else {
            return None;
        };
        self.text_rasters
            .block(hit.clip.text()?, style, self.project.output_size())
    }
    pub fn picture(&self) -> Picture {
        if self.showing_source() {
            return self.source_picture();
        }
        let hit = resolve_preview(
            &self.project,
            self.playhead,
            None,
            Some(&[TrackKind::Text, TrackKind::Video]),
        );
        Picture {
            active: hit.is_some(),
            clip_id: hit.as_ref().map(|h| h.clip.id.clone()),
            name: hit.as_ref().map(|h| h.display_name()),
            path: hit.as_ref().and_then(|h| h.media()?.path.clone()),
            aspect: hit.as_ref().and_then(|h| h.source_size),
            media_time: hit
                .as_ref()
                .and_then(|h| h.media_time())
                .unwrap_or(self.playhead),
            transform: hit
                .as_ref()
                .map_or_else(ClipTransform::default, |h| h.clip.transform),
        }
    }

    fn source_picture(&self) -> Picture {
        let Some(source) = &self.source_monitor else {
            return Picture {
                active: false,
                clip_id: None,
                name: None,
                path: None,
                aspect: None,
                media_time: 0.0,
                transform: ClipTransform::default(),
            };
        };
        let media = self.project.media.iter().find(|m| m.id == source.media_id);
        let path = media
            .and_then(|m| m.path.clone())
            .filter(|p| !p.trim().is_empty());
        let aspect = media.and_then(|m| match (m.width, m.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        });
        Picture {
            active: path.is_some() && media.is_some_and(todo_core::is_visual_media),
            clip_id: None,
            name: media.map(|m| m.name.clone()),
            path,
            aspect,
            media_time: source.time,
            transform: ClipTransform::default(),
        }
    }

    pub(crate) fn can_play_at(&self, video: bool) -> bool {
        if self.showing_source() {
            return video;
        }
        video
            || resolve_audio_clips(&self.project)
                .iter()
                .any(|c| self.playhead < c.start + c.duration)
    }
}

/// How the native host draws a library item.
fn layer_kind(media: &todo_core::MediaItem) -> PreviewLayerKind {
    match media.alpha {
        Some(todo_core::AlphaPacking::Stacked) => PreviewLayerKind::Packed,
        Some(todo_core::AlphaPacking::Reveal) => PreviewLayerKind::Reveal,
        None if todo_core::is_image_media(media) => PreviewLayerKind::Image,
        None => PreviewLayerKind::Video,
    }
}

/// The picture layers of `hits` (bottom → top), as the native host draws them.
/// Preview passes its budgeted stack, export the exact one; `text` returns a
/// text clip's raster path once rendered. One function for both, so export
/// shows what the preview showed.
pub fn picture_layers(
    project: &todo_core::Project,
    hits: Vec<PreviewHit>,
    text: &mut dyn FnMut(&str, TextRaster) -> Option<String>,
) -> Vec<PreviewLayer> {
    let fps = project.fps;
    let canvas = project.output_size();
    // (layer, is a chalk hand)
    let mut layers: Vec<(PreviewLayer, bool)> = Vec::new();
    for hit in hits {
        let mut transform = todo_core::hit_transform(project, &hit);
        let mut soft = None;
        let mut hand = None;
        let (path, media_time, kind) = match hit.source {
            HitSource::Media { media, media_time } => {
                let Some(path) = media.path.clone().filter(|p| !p.trim().is_empty()) else {
                    continue;
                };
                let mut time = media_time.max(0.0);
                // A reveal plays by progress; its chalk hand is one more layer.
                if let Some(frame) = todo_core::reveal_frame(&media, time) {
                    hand = todo_core::reveal_hand(&media, &transform, canvas, time);
                    soft = Some(frame.soft);
                    time = frame.progress;
                }
                (path, time, layer_kind(&media))
            }
            HitSource::Text { style, frame } => {
                let Some(content) = hit.clip.text().cloned() else {
                    continue;
                };
                transform.opacity *= frame.opacity;
                // Per-character reveals change once per output frame at most.
                let reveal = frame.reveal.map(|mut reveal| {
                    if fps.is_finite() && fps > 0.0 {
                        reveal.time = (reveal.time * fps).floor() / fps;
                    }
                    reveal
                });
                match text(
                    &hit.clip.id,
                    TextRaster::new(content, style, canvas, reveal),
                ) {
                    // Text rasters are stills: no source time.
                    Some(path) => (path, 0.0, PreviewLayerKind::Text),
                    None => continue,
                }
            }
        };
        layers.push((
            PreviewLayer {
                path,
                media_time,
                aspect: hit.source_size,
                kind,
                transform,
                soft,
            },
            false,
        ));
        if let Some(hand) = hand {
            layers.push((
                PreviewLayer {
                    path: hand.path,
                    media_time: 0.0,
                    aspect: Some(hand.size),
                    kind: PreviewLayerKind::Image,
                    transform: hand.transform,
                    soft: None,
                },
                true,
            ));
        }
    }
    // Over the layer budget, hands go first, then the deepest layers.
    while layers.len() > todo_core::MAX_PREVIEW_LAYERS {
        match layers.iter().rposition(|(_, hand)| *hand) {
            Some(i) => {
                layers.remove(i);
            }
            None => {
                layers.remove(0);
            }
        }
    }
    layers.into_iter().map(|(layer, _)| layer).collect()
}
