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
            let audio = resolve_audio_clips(&self.project);
            if self.last_audio.as_ref() != Some(&audio) {
                commands.push(PreviewCommand::Audio(audio.clone()));
                self.last_audio = Some(audio);
                transport = true;
                force_transport = true;
            }
        }
        if transport {
            let key = ((self.playhead * 1000.0).round() as i64, self.playing);
            if force_transport || self.last_transport != Some(key) {
                commands.push(PreviewCommand::Transport {
                    time: self.playhead.max(0.0),
                    playing: self.playing,
                });
                self.last_transport = Some(key);
                self.bump_clock();
            }
        }
        if want.picture {
            // A stack waiting for text rasters is not sent; the host keeps showing
            // the previous complete stack until the rasters arrive.
            if let Some(layers) = self.preview_layers() {
                if self.last_layers.as_ref() != Some(&layers) {
                    commands.push(PreviewCommand::Layers(layers.clone()));
                    self.last_layers = Some(layers);
                    // Legacy single-source caches are superseded by the stack.
                    self.last_path = None;
                    self.last_seek_ms = None;
                    self.last_transform = None;
                }
            }
        }
        commands
    }
    pub(crate) fn bump_clock(&mut self) {
        self.clock_epoch = self.clock_epoch.wrapping_add(1);
        self.clock = None;
        self.clock_missing_at = None;
    }
    /// The full picture stack at the playhead, or `None` while a text raster is
    /// still being rendered.
    fn preview_layers(&mut self) -> Option<Vec<PreviewLayer>> {
        let fps = self.project.fps;
        let canvas = self.project.output_size();
        let mut layers = Vec::new();
        let mut pending = false;
        for hit in resolve_layers(&self.project, self.playhead) {
            let mut transform = hit.clip.transform;
            let (path, media_time) = match hit.source {
                HitSource::Media { media, media_time } => {
                    let Some(path) = media.path.filter(|p| !p.trim().is_empty()) else {
                        continue;
                    };
                    (path, media_time.max(0.0))
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
                    match self
                        .text_rasters
                        .state(TextRaster::new(content, style, canvas, reveal))
                    {
                        // Text rasters are stills: no source time.
                        RasterState::Ready(path) => (path, 0.0),
                        RasterState::Pending => {
                            pending = true;
                            continue;
                        }
                        RasterState::Failed => continue,
                    }
                }
            };
            layers.push(PreviewLayer {
                path,
                media_time,
                aspect: hit.source_size,
                transform,
            });
        }
        (!pending).then_some(layers)
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
    pub(crate) fn can_play_at(&self, video: bool) -> bool {
        video
            || resolve_audio_clips(&self.project)
                .iter()
                .any(|c| self.playhead < c.start + c.duration)
    }
}
