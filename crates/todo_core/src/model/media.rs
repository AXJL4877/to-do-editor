use serde::{Deserialize, Serialize};

/// Default timeline hold for still images when duration is missing/invalid.
pub const DEFAULT_STILL_DURATION_SECS: f64 = 5.0;

/// Library / probe classification. Stills are visual but not video.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AlphaPacking {
    Stacked,
    /// A `todo-reveal/1` atlas: the picture appears over its duration.
    Reveal,
}

/// What a reveal layer needs besides its atlas: the soft-edge scale and the
/// chalk hand that follows the pen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RevealInfo {
    pub soft_max: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand: Option<RevealHand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RevealHand {
    /// Hand image, relative to the atlas's directory.
    pub file: String,
    /// Pixel size of the hand image.
    pub width: u32,
    pub height: u32,
    /// Pen tip inside the hand image, 0..1.
    pub anchor: [f64; 2],
    /// Hand height as a multiple of the element's height.
    pub scale: f64,
    /// `[seconds, x, y]`, the tip inside the element (0..1).
    pub path: Vec<[f64; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Set by probe when known. Legacy projects omit it — see [`media_kind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MediaKind>,
    /// Packed-alpha storage scheme, when a probe identified one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<AlphaPacking>,
    /// Playback data of a reveal atlas (`alpha == Reveal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<RevealInfo>,
    /// Virtual library folder; `None` is the root. Legacy projects omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
}

const IMAGE_EXTS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif", ".tif", ".tiff",
];
const VIDEO_EXTS: &[&str] = &[
    ".mp4", ".mov", ".mkv", ".webm", ".avi", ".m4v", ".wmv", ".mpeg", ".mpg",
];
const AUDIO_EXTS: &[&str] = &[
    ".wav", ".mp3", ".flac", ".aac", ".m4a", ".ogg", ".aiff", ".aif",
];

fn path_hint(item: &MediaItem) -> String {
    item.path
        .as_deref()
        .unwrap_or(&item.name)
        .to_ascii_lowercase()
}

fn ext_matches(path: &str, exts: &[&str]) -> bool {
    exts.iter().any(|suffix| path.ends_with(suffix))
}

/// Resolve library kind: explicit `kind` wins; otherwise path / dimensions heuristics.
/// Images are never inferred as video solely from having width×height.
pub fn media_kind(item: &MediaItem) -> MediaKind {
    if let Some(kind) = item.kind {
        return kind;
    }
    let path = path_hint(item);
    if ext_matches(&path, IMAGE_EXTS) {
        return MediaKind::Image;
    }
    if ext_matches(&path, AUDIO_EXTS) {
        return MediaKind::Audio;
    }
    let has_frame = item.width.is_some_and(|w| w > 0) && item.height.is_some_and(|h| h > 0);
    if has_frame {
        // Pixels without an image extension: treat as video (legacy probed clips).
        return MediaKind::Video;
    }
    // Legacy audio-only: no frame buffer, hasAudio — even if the name looks like .mp4.
    if item.has_audio == Some(true) {
        return MediaKind::Audio;
    }
    if ext_matches(&path, VIDEO_EXTS) {
        return MediaKind::Video;
    }
    MediaKind::Video
}

pub fn is_image_media(item: &MediaItem) -> bool {
    media_kind(item) == MediaKind::Image
}

/// Moving / video containers only — stills are [`is_image_media`], not video.
pub fn is_video_media(item: &MediaItem) -> bool {
    media_kind(item) == MediaKind::Video
}

/// Video or still — may land on a video track.
pub fn is_visual_media(item: &MediaItem) -> bool {
    matches!(media_kind(item), MediaKind::Video | MediaKind::Image)
}

/// Clip duration when placing media.
/// Stills always hold at least [`DEFAULT_STILL_DURATION_SECS`] (ffprobe often
/// reports a single-frame length like 0.04s, which is not a useful timeline hold).
pub fn place_duration(item: &MediaItem) -> f64 {
    if is_image_media(item) {
        let d = if item.duration.is_finite() && item.duration > 0.0 {
            item.duration
        } else {
            DEFAULT_STILL_DURATION_SECS
        };
        d.max(DEFAULT_STILL_DURATION_SECS)
    } else {
        item.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        name: &str,
        path: Option<&str>,
        kind: Option<MediaKind>,
        w: Option<u32>,
        h: Option<u32>,
    ) -> MediaItem {
        MediaItem {
            id: "m".into(),
            name: name.into(),
            path: path.map(str::to_string),
            duration: 5.0,
            has_audio: Some(false),
            width: w,
            height: h,
            kind,
            alpha: None,
            reveal: None,
            folder_id: None,
        }
    }

    #[test]
    fn image_ext_is_not_video_even_with_dimensions() {
        let png = item("a.png", Some("data/a.png"), None, Some(800), Some(600));
        assert!(is_image_media(&png));
        assert!(!is_video_media(&png));
        assert!(is_visual_media(&png));
    }

    #[test]
    fn explicit_kind_wins_over_extension() {
        let forced = item(
            "a.png",
            Some("a.png"),
            Some(MediaKind::Video),
            Some(1),
            Some(1),
        );
        assert!(is_video_media(&forced));
    }

    #[test]
    fn place_duration_falls_back_for_invalid_still() {
        let mut still = item("a.jpg", None, Some(MediaKind::Image), Some(10), Some(10));
        still.duration = 0.0;
        assert!((place_duration(&still) - DEFAULT_STILL_DURATION_SECS).abs() < 1e-9);
    }

    #[test]
    fn place_duration_lifts_ffprobe_frame_length_to_default_hold() {
        let mut still = item("a.jpg", None, Some(MediaKind::Image), Some(10), Some(10));
        still.duration = 0.04;
        assert!((place_duration(&still) - DEFAULT_STILL_DURATION_SECS).abs() < 1e-9);
        still.duration = 12.0;
        assert!((place_duration(&still) - 12.0).abs() < 1e-9);
    }
}
