//! Which text images the preview needs and which are ready. Rendering and files
//! belong to workers; this only tracks identities, outcomes and measured blocks.
use std::collections::{HashMap, HashSet};
use todo_core::{
    text_raster_digest, BaseSize, PerCharReveal, TextContent, TextStyle, TransformBounds,
};

/// One text image to render: canvas-sized, transparent, text block centered.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRaster {
    pub digest: String,
    pub content: TextContent,
    pub style: TextStyle,
    pub canvas: (u32, u32),
    pub reveal: Option<PerCharReveal>,
}

impl TextRaster {
    pub fn new(
        content: TextContent,
        style: TextStyle,
        canvas: (u32, u32),
        reveal: Option<PerCharReveal>,
    ) -> Self {
        Self {
            digest: text_raster_digest(&content, &style, canvas, reveal.as_ref()),
            content,
            style,
            canvas,
            reveal,
        }
    }

    /// Identity of the layout, shared by every reveal frame of the same text.
    fn layout_digest(&self) -> String {
        text_raster_digest(&self.content, &self.style, self.canvas, None)
    }
}

/// Worker answer for one requested raster.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RasterOutcome {
    Rendered {
        path: String,
        block: TransformBounds,
    },
    Failed,
    /// Dropped in favor of a newer frame; requested again if still needed.
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RasterState {
    Ready(String),
    Pending,
    /// Rendering failed; the layer is left out instead of stalling the preview.
    Failed,
}

#[derive(Debug, Default)]
pub(crate) struct TextRasters {
    ready: HashMap<String, String>,
    failed: HashSet<String>,
    /// Requested digest → layout digest, until the worker answers.
    requested: HashMap<String, String>,
    wanted: Vec<TextRaster>,
    blocks: HashMap<String, BaseSize>,
    /// Last exact raster actually selected for each visible clip. Worker
    /// completion order must not move a pending reveal back to an older frame.
    shown: HashMap<String, ShownRaster>,
}

#[derive(Debug)]
struct ShownRaster {
    layout: String,
    path: String,
}

impl TextRasters {
    /// Only the current program stack may supply a temporary text fallback.
    pub fn retain_shown(&mut self, keep: impl Fn(&str) -> bool) {
        self.shown.retain(|clip_id, _| keep(clip_id));
    }

    /// A pending reveal may retain this clip's last shown compatible image.
    /// Exact images always win, including when scrubbing backwards.
    pub fn preview_state(&mut self, clip_id: &str, raster: TextRaster) -> RasterState {
        let layout = raster.layout_digest();
        match self.state(raster) {
            RasterState::Ready(path) => {
                self.shown.insert(
                    clip_id.to_owned(),
                    ShownRaster {
                        layout,
                        path: path.clone(),
                    },
                );
                RasterState::Ready(path)
            }
            RasterState::Pending => {
                if let Some(shown) = self
                    .shown
                    .get(clip_id)
                    .filter(|shown| shown.layout == layout)
                {
                    return RasterState::Ready(shown.path.clone());
                }
                self.shown.remove(clip_id);
                RasterState::Pending
            }
            RasterState::Failed => {
                self.shown.remove(clip_id);
                RasterState::Failed
            }
        }
    }

    /// Current state; an unknown raster is queued for [`TextRasters::take_wanted`].
    pub fn state(&mut self, raster: TextRaster) -> RasterState {
        if let Some(path) = self.ready.get(&raster.digest) {
            return RasterState::Ready(path.clone());
        }
        if self.failed.contains(&raster.digest) {
            return RasterState::Failed;
        }
        if !self.requested.contains_key(&raster.digest) {
            self.requested
                .insert(raster.digest.clone(), raster.layout_digest());
            self.wanted.push(raster);
        }
        RasterState::Pending
    }

    pub fn take_wanted(&mut self) -> Vec<TextRaster> {
        std::mem::take(&mut self.wanted)
    }

    pub fn complete(&mut self, digest: String, outcome: RasterOutcome) {
        let layout = self.requested.remove(&digest);
        match outcome {
            RasterOutcome::Rendered { path, block } => {
                if let Some(layout) = layout {
                    self.blocks.insert(
                        layout,
                        BaseSize {
                            width: block.width(),
                            height: block.height(),
                        },
                    );
                }
                self.failed.remove(&digest);
                self.ready.insert(digest, path);
            }
            RasterOutcome::Failed => {
                self.failed.insert(digest);
            }
            RasterOutcome::Skipped => {}
        }
    }

    /// Measured block of a text, once any of its rasters has been rendered.
    pub fn block(
        &self,
        content: &TextContent,
        style: &TextStyle,
        canvas: (u32, u32),
    ) -> Option<BaseSize> {
        self.blocks
            .get(&text_raster_digest(content, style, canvas, None))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::{builtin_text_styles, DEFAULT_TEXT_STYLE_ID};

    fn raster(text: &str) -> TextRaster {
        TextRaster::new(
            TextContent::new(text, DEFAULT_TEXT_STYLE_ID),
            builtin_text_styles().remove(0),
            (1920, 1080),
            None,
        )
    }

    fn block() -> TransformBounds {
        TransformBounds {
            left: 0.2,
            top: 0.4,
            right: 0.8,
            bottom: 0.6,
        }
    }

    #[test]
    fn requests_each_raster_once_and_tracks_outcomes() {
        let mut rasters = TextRasters::default();
        assert_eq!(rasters.state(raster("a")), RasterState::Pending);
        assert_eq!(rasters.state(raster("a")), RasterState::Pending);
        let wanted = rasters.take_wanted();
        assert_eq!(wanted.len(), 1);
        assert!(rasters.take_wanted().is_empty());
        rasters.complete(
            wanted[0].digest.clone(),
            RasterOutcome::Rendered {
                path: "a.png".into(),
                block: block(),
            },
        );
        assert_eq!(
            rasters.state(raster("a")),
            RasterState::Ready("a.png".into())
        );
        let b = raster("b");
        rasters.state(b.clone());
        rasters.complete(b.digest.clone(), RasterOutcome::Failed);
        assert_eq!(rasters.state(b), RasterState::Failed);
        // A skipped frame is simply asked for again.
        let c = raster("c");
        rasters.state(c.clone());
        rasters.take_wanted();
        rasters.complete(c.digest.clone(), RasterOutcome::Skipped);
        assert_eq!(rasters.state(c), RasterState::Pending);
        assert_eq!(rasters.take_wanted().len(), 1);
    }

    #[test]
    fn a_reveal_frame_measures_the_block_for_the_whole_text() {
        let mut rasters = TextRasters::default();
        let base = raster("a");
        let frame = TextRaster::new(
            base.content.clone(),
            base.style.clone(),
            base.canvas,
            Some(PerCharReveal {
                time: 0.1,
                duration: 1.0,
                char_duration: 0.2,
            }),
        );
        assert_eq!(rasters.block(&base.content, &base.style, base.canvas), None);
        rasters.state(frame.clone());
        rasters.complete(
            frame.digest,
            RasterOutcome::Rendered {
                path: "f.png".into(),
                block: block(),
            },
        );
        let measured = rasters
            .block(&base.content, &base.style, base.canvas)
            .unwrap();
        assert!((measured.width - 0.6).abs() < 1e-9 && (measured.height - 0.2).abs() < 1e-9);
    }

    #[test]
    fn digest_ignores_animation_but_not_reveal_or_canvas() {
        let mut animated = raster("a");
        animated.content.animation.intro = Some(todo_core::TextIntro::Fade { duration: 1.0 });
        let animated = TextRaster::new(animated.content, animated.style, (1920, 1080), None);
        assert_eq!(animated.digest, raster("a").digest);
        let smaller = TextRaster::new(
            animated.content.clone(),
            animated.style.clone(),
            (1280, 720),
            None,
        );
        assert_ne!(smaller.digest, animated.digest);
        let revealing = TextRaster::new(
            animated.content,
            animated.style,
            (1920, 1080),
            Some(PerCharReveal {
                time: 0.1,
                duration: 1.0,
                char_duration: 0.2,
            }),
        );
        assert_ne!(revealing.digest, raster("a").digest);
    }
}
