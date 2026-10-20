use serde::{Deserialize, Serialize};

use super::clip::normalize_existing;
use super::folder::MediaFolder;
use super::media::MediaItem;
use super::text::TextStyle;
use super::track::{create_default_tracks, Track};

/// Default composition / workspace length for new projects (PR/AE-style).
pub const DEFAULT_WORKSPACE_SECS: f64 = 60.0;

/// Default program-canvas pixel size (1080p landscape).
pub const DEFAULT_OUTPUT_WIDTH: u32 = 1920;
pub const DEFAULT_OUTPUT_HEIGHT: u32 = 1080;

fn default_output_width() -> u32 {
    DEFAULT_OUTPUT_WIDTH
}
fn default_output_height() -> u32 {
    DEFAULT_OUTPUT_HEIGHT
}

fn is_default_media_scale(mode: &MediaScaleMode) -> bool {
    *mode == MediaScaleMode::default()
}

/// How newly placed visual media is scaled onto the program canvas.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum MediaScaleMode {
    /// One source pixel maps to one canvas pixel (PR “Set to Frame Size”).
    #[default]
    Pixel,
    /// Aspect-fit so the media fills the program frame (PR “Scale to Frame Size”).
    MatchFrame,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub version: u32,
    pub fps: f64,
    pub playhead: f64,
    /// Program / composition pixel width (export & preview frame).
    #[serde(default = "default_output_width")]
    pub width: u32,
    /// Program / composition pixel height.
    #[serde(default = "default_output_height")]
    pub height: u32,
    /// Fixed workspace length in seconds. Drives timeline axis and playback end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Default scale applied when placing library media on the timeline.
    #[serde(default, skip_serializing_if = "is_default_media_scale")]
    pub media_scale: MediaScaleMode,
    #[serde(default)]
    pub media: Vec<MediaItem>,
    /// Virtual library folders. Disk media paths stay flat under `media/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<MediaFolder>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Styles referenced by text clips, copied in when first used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_styles: Vec<TextStyle>,
    /// Motion presets used by clips and not built in, copied in on first use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub motions: Vec<todo_motion::MotionPreset>,
}

fn normalize_tracks(mut tracks: Vec<Track>) -> Vec<Track> {
    for track in &mut tracks {
        track.clips = track.clips.drain(..).map(normalize_existing).collect();
    }
    // Text above Video above Audio above Sfx; keep relative order inside each kind.
    tracks.sort_by_key(|track| track.kind.section_rank());
    tracks
}

pub fn create_project(
    fps: Option<f64>,
    playhead: Option<f64>,
    duration: Option<f64>,
    media: Option<Vec<MediaItem>>,
    tracks: Option<Vec<Track>>,
) -> Project {
    Project {
        version: 1,
        fps: fps.unwrap_or(30.0),
        playhead: playhead.unwrap_or(0.0),
        width: DEFAULT_OUTPUT_WIDTH,
        height: DEFAULT_OUTPUT_HEIGHT,
        duration: Some(
            duration
                .filter(|d| d.is_finite() && *d > 0.0)
                .unwrap_or(DEFAULT_WORKSPACE_SECS),
        ),
        media_scale: MediaScaleMode::default(),
        media: media.unwrap_or_default(),
        folders: Vec::new(),
        tracks: normalize_tracks(tracks.unwrap_or_else(create_default_tracks)),
        text_styles: Vec::new(),
        motions: Vec::new(),
    }
}

impl Project {
    /// Program-canvas pixel size used for preview framing and future export.
    pub fn output_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn text_style(&self, id: &str) -> Option<&TextStyle> {
        self.text_styles.iter().find(|style| style.id == id)
    }
}
