use std::io::Write;
use std::time::{Duration, Instant};

use crate::PreviewError;

/// Repo-relative location of `preview_host.exe`.
pub const HOST_RELATIVE: &str = "native/build/bin/preview_host.exe";

/// Same cap as the native preview host stdin protocol.
pub const MAX_AUDIO_CLIPS: usize = 4096;

/// Mirrors `todo_core::MAX_PREVIEW_LAYERS` (checked by a `todo_app` test) and the
/// native `VideoEngine::kMaxLayers`.
pub const MAX_PREVIEW_LAYERS: usize = 16;

/// One visual layer for `layers-begin` / `layer` / `layers-commit`.
///
/// Wire: `layer <kind> <media_time> <x> <y> <scale> <scale_x> <scale_y> <opacity> <path>`.
/// `path` is the rest of the line and may contain spaces. A reveal layer's
/// kind is `reveal:<soft>` and its media time is the progress 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewLayer {
    pub path: String,
    pub media_time: f64,
    pub kind: LayerKind,
    pub transform: VisualTransform,
    /// Reveal soft edge as a fraction of the duration.
    pub soft: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerKind {
    #[default]
    Video,
    Image,
    Packed,
    Reveal,
    Text,
}

impl LayerKind {
    fn wire(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Image => "image",
            Self::Packed => "packed",
            Self::Reveal => "reveal",
            Self::Text => "text",
        }
    }
}

/// `readAudioClock` drops a sample older than this.
pub const CLOCK_STALE: Duration = Duration::from_secs(1);

pub(crate) const MAX_STDOUT_LINE: usize = 4096;

/// One source clip on the native mixer timeline.
///
/// Wire: `audio-clip <start> <duration> <offset> <rate> <gain> <fade_in> <fade_out> <path>`.
/// `path` is the rest of the line and may contain spaces.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub path: String,
    pub start: f64,
    pub duration: f64,
    pub offset: f64,
    pub rate: f64,
    pub gain: f64,
    pub fade_in: f64,
    pub fade_out: f64,
}

/// Last `audio-clock` line, stamped when it was read.
///
/// Wire: `audio-clock <seconds> <playing 0|1> <available 0|1>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockSample {
    pub time: f64,
    pub playing: bool,
    pub available: bool,
    pub received_at: Instant,
}

/// Clock after the same extrapolation as Electron `readAudioClock`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioClock {
    pub time: f64,
    pub playing: bool,
    pub available: bool,
}

impl ClockSample {
    /// `None` when `now` is more than [`CLOCK_STALE`] after the sample.
    /// Playing samples advance by the elapsed seconds. Paused samples do not.
    pub fn extrapolated(&self, now: Instant) -> Option<AudioClock> {
        let elapsed = now
            .saturating_duration_since(self.received_at)
            .as_secs_f64();
        if elapsed > CLOCK_STALE.as_secs_f64() {
            return None;
        }
        let time = if self.playing {
            self.time + elapsed
        } else {
            self.time
        };
        Some(AudioClock {
            time,
            playing: self.playing,
            available: self.available,
        })
    }
}

/// Physical screen rectangle reported by the host.
///
/// Wire: `preview-bounds <x> <y> <w> <h> [<visible 0|1>]`.
/// `visible` is `None` when the host omits the flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewBounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub visible: Option<bool>,
}

/// Popup rectangle in physical screen pixels.
///
/// Wire: `size-screen <x> <y> <w> <h> <visible 0|1>`.
/// `x` and `y` may be negative on a multi-monitor desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenSlot {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
}

/// Rectangle of the preview window that must stay transparent to the
/// editor underneath (an egui popup, menu or tooltip overlapping it).
/// Physical pixels relative to the preview window's top-left corner.
///
/// Wire: `clip <count> (<x> <y> <w> <h>)*`; `clip 0` restores the full window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Most cut-outs one `clip` command carries; more popups than that at once
/// fall back to hiding the preview.
pub const MAX_CLIP_RECTS: usize = 16;

/// Slot inside the owner window, distinct from [`ScreenSlot`].
///
/// Wire: `layout <x> <y> <w> <h>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Visual placement relative to a centered source aspect fit. Offsets are
/// canvas fractions; positive axis multipliers resize around the center.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualTransform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub opacity: f64,
}

impl Default for VisualTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
            opacity: 1.0,
        }
    }
}

/// Temporary normalized selection bounds, independent of image opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformGizmo {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub editable: bool,
}

/// Soft cap for live align guides drawn over the preview.
pub const MAX_ALIGN_GUIDES: usize = 4;

/// One smart-align guide in normalized preview-window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignGuide {
    /// `true` = vertical line (fixed X); `false` = horizontal (fixed Y).
    pub vertical: bool,
    pub position: f64,
}

/// Stdout event. The mutex copy is authoritative if the channel is full.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HostEvent {
    PreviewNavigation(crate::PreviewNavigation),
    AudioClock(ClockSample),
    PreviewBounds(PreviewBounds),
    SourceAspect {
        generation: u64,
        aspect: f64,
    },
    PreviewPointer {
        kind: PreviewPointerKind,
        x: f64,
        y: f64,
    },
}

/// Captured pointer phase from the native preview canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPointerKind {
    Down,
    Move,
    Up,
}

pub(crate) enum ParsedLine {
    Navigation(crate::PreviewNavigation),
    SourceAspect {
        generation: u64,
        aspect: f64,
    },
    Clock {
        time: f64,
        playing: bool,
        available: bool,
    },
    Bounds(PreviewBounds),
    Pointer {
        kind: PreviewPointerKind,
        x: f64,
        y: f64,
    },
}

pub(crate) fn parse_host_line(line: &str) -> Option<ParsedLine> {
    let line = line.trim();
    if line.is_empty() || line.len() > MAX_STDOUT_LINE {
        return None;
    }
    let mut parts = line.split_whitespace();
    match parts.next()? {
        "preview-navigation" => {
            crate::viewport::parse_navigation(parts).map(ParsedLine::Navigation)
        }
        "source-aspect" => {
            let generation = parts.next()?.parse::<u64>().ok()?;
            let aspect = parts.next()?.parse::<f64>().ok()?;
            if !aspect.is_finite() || aspect <= 0.0 || parts.next().is_some() {
                return None;
            }
            Some(ParsedLine::SourceAspect { generation, aspect })
        }
        "audio-clock" => {
            let time = parts.next()?.parse::<f64>().ok()?;
            let playing = flag(parts.next()?)?;
            let available = flag(parts.next()?)?;
            if parts.next().is_some() || !time.is_finite() || time < 0.0 {
                return None;
            }
            Some(ParsedLine::Clock {
                time,
                playing,
                available,
            })
        }
        "preview-bounds" => {
            let x = parts.next()?.parse::<f64>().ok()?;
            let y = parts.next()?.parse::<f64>().ok()?;
            let w = parts.next()?.parse::<f64>().ok()?;
            let h = parts.next()?.parse::<f64>().ok()?;
            if !finite4(x, y, w, h) || w < 0.0 || h < 0.0 {
                return None;
            }
            let visible = match parts.next() {
                None => None,
                Some(token) => Some(flag(token)?),
            };
            if parts.next().is_some() {
                return None;
            }
            Some(ParsedLine::Bounds(PreviewBounds {
                x,
                y,
                w,
                h,
                visible,
            }))
        }
        "preview-pointer" => {
            let kind = match parts.next()? {
                "down" => PreviewPointerKind::Down,
                "move" => PreviewPointerKind::Move,
                "up" => PreviewPointerKind::Up,
                _ => return None,
            };
            let x = parts.next()?.parse::<f64>().ok()?;
            let y = parts.next()?.parse::<f64>().ok()?;
            if !x.is_finite() || !y.is_finite() || parts.next().is_some() {
                return None;
            }
            Some(ParsedLine::Pointer { kind, x, y })
        }
        _ => None,
    }
}

fn flag(token: &str) -> Option<bool> {
    match token {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

fn finite4(a: f64, b: f64, c: f64, d: f64) -> bool {
    a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite()
}

pub(crate) fn encode_hwnd(buf: &mut Vec<u8>, hwnd: u64) {
    let _ = writeln!(buf, "hwnd {hwnd:x}");
}

pub(crate) fn encode_path(buf: &mut Vec<u8>, path: Option<&str>) -> Result<(), PreviewError> {
    match path {
        None | Some("") => {
            let _ = writeln!(buf, "path");
            Ok(())
        }
        Some(path) => {
            if has_control(path) {
                return Err(PreviewError::Invalid(
                    "media path contains a newline or NUL",
                ));
            }
            let _ = write!(buf, "path ");
            buf.extend_from_slice(path.as_bytes());
            buf.push(b'\n');
            Ok(())
        }
    }
}

pub(crate) fn encode_path_token(
    buf: &mut Vec<u8>,
    path: Option<&str>,
    generation: u64,
) -> Result<(), PreviewError> {
    if path.is_some_and(has_control) {
        return Err(PreviewError::Invalid(
            "media path contains a newline or NUL",
        ));
    }
    let _ = write!(buf, "path-token {generation}");
    if let Some(path) = path.filter(|path| !path.is_empty()) {
        buf.push(b' ');
        buf.extend_from_slice(path.as_bytes());
    }
    buf.push(b'\n');
    Ok(())
}

pub(crate) fn encode_seek(buf: &mut Vec<u8>, seconds: f64) -> Result<(), PreviewError> {
    if !seconds.is_finite() {
        return Err(PreviewError::Invalid("seek time must be finite"));
    }
    let _ = writeln!(buf, "seek {}", seconds.max(0.0));
    Ok(())
}

pub(crate) fn encode_aspect(
    buf: &mut Vec<u8>,
    width: f64,
    height: f64,
) -> Result<(), PreviewError> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(PreviewError::Invalid(
            "aspect width and height must be finite and > 0",
        ));
    }
    let _ = writeln!(buf, "aspect {width} {height}");
    Ok(())
}

pub(crate) fn encode_transform(
    buf: &mut Vec<u8>,
    x: f64,
    y: f64,
    scale: f64,
) -> Result<(), PreviewError> {
    if !x.is_finite() || !y.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return Err(PreviewError::Invalid(
            "transform values must be finite and scale > 0",
        ));
    }
    let _ = writeln!(buf, "transform {x} {y} {scale}");
    Ok(())
}

pub(crate) fn encode_visual_transform(
    buf: &mut Vec<u8>,
    value: VisualTransform,
) -> Result<(), PreviewError> {
    if !finite4(value.x, value.y, value.scale, value.scale_x)
        || !value.scale_y.is_finite()
        || !value.opacity.is_finite()
        || value.scale <= 0.0
        || value.scale_x <= 0.0
        || value.scale_y <= 0.0
        || !(0.0..=1.0).contains(&value.opacity)
    {
        return Err(PreviewError::Invalid(
            "transform must be finite, scales > 0 and opacity in 0..=1",
        ));
    }
    let _ = writeln!(
        buf,
        "transform {} {} {} {} {} {}",
        value.x, value.y, value.scale, value.scale_x, value.scale_y, value.opacity
    );
    Ok(())
}

pub(crate) fn encode_gizmo(
    buf: &mut Vec<u8>,
    value: Option<TransformGizmo>,
) -> Result<(), PreviewError> {
    if let Some(value) = value {
        if !finite4(value.left, value.top, value.right, value.bottom)
            || value.left >= value.right
            || value.top >= value.bottom
            || !(value.right - value.left).is_finite()
            || !(value.bottom - value.top).is_finite()
        {
            return Err(PreviewError::Invalid(
                "gizmo bounds must be finite with positive dimensions",
            ));
        }
        let _ = writeln!(
            buf,
            "gizmo {} {} {} {} {}",
            value.left,
            value.top,
            value.right,
            value.bottom,
            u8::from(value.editable)
        );
    } else {
        buf.extend_from_slice(b"gizmo\n");
    }
    Ok(())
}

/// `guides` clears. Otherwise `guides <axis> <pos>…` with axis `0`=vertical, `1`=horizontal.
pub(crate) fn encode_guides(buf: &mut Vec<u8>, guides: &[AlignGuide]) -> Result<(), PreviewError> {
    if guides.is_empty() {
        buf.extend_from_slice(b"guides\n");
        return Ok(());
    }
    if guides.len() > MAX_ALIGN_GUIDES {
        return Err(PreviewError::Invalid("too many align guides"));
    }
    if guides.iter().any(|g| !g.position.is_finite()) {
        return Err(PreviewError::Invalid("guide position must be finite"));
    }
    buf.extend_from_slice(b"guides");
    for guide in guides {
        let axis = if guide.vertical { 0_u8 } else { 1_u8 };
        let _ = write!(buf, " {} {}", axis, guide.position);
    }
    buf.extend_from_slice(b"\n");
    Ok(())
}

pub(crate) fn encode_layout(buf: &mut Vec<u8>, rect: LayoutRect) -> Result<(), PreviewError> {
    if !finite4(rect.x, rect.y, rect.w, rect.h) {
        return Err(PreviewError::Invalid("layout values must be finite"));
    }
    if rect.w < 0.0 || rect.h < 0.0 {
        return Err(PreviewError::Invalid(
            "layout width and height must be >= 0",
        ));
    }
    let _ = writeln!(buf, "layout {} {} {} {}", rect.x, rect.y, rect.w, rect.h);
    Ok(())
}

pub(crate) fn encode_size_screen(buf: &mut Vec<u8>, slot: ScreenSlot) -> Result<(), PreviewError> {
    if slot.w < 0 || slot.h < 0 {
        return Err(PreviewError::Invalid(
            "screen width and height must be >= 0",
        ));
    }
    let visible = u8::from(slot.visible);
    let _ = writeln!(
        buf,
        "size-screen {} {} {} {} {visible}",
        slot.x, slot.y, slot.w, slot.h
    );
    Ok(())
}

pub(crate) fn encode_clip(buf: &mut Vec<u8>, rects: &[ClipRect]) -> Result<(), PreviewError> {
    if rects.len() > MAX_CLIP_RECTS {
        return Err(PreviewError::Invalid("too many clip rectangles"));
    }
    if rects.iter().any(|r| r.w <= 0 || r.h <= 0) {
        return Err(PreviewError::Invalid("clip rectangles must not be empty"));
    }
    let _ = write!(buf, "clip {}", rects.len());
    for r in rects {
        let _ = write!(buf, " {} {} {} {}", r.x, r.y, r.w, r.h);
    }
    buf.push(b'\n');
    Ok(())
}

pub(crate) fn encode_layers(
    buf: &mut Vec<u8>,
    layers: &[PreviewLayer],
) -> Result<(), PreviewError> {
    if layers.len() > MAX_PREVIEW_LAYERS {
        return Err(PreviewError::Invalid("too many preview layers"));
    }
    buf.extend_from_slice(b"layers-begin\n");
    for layer in layers {
        if !layer.media_time.is_finite() || layer.media_time < 0.0 {
            return Err(PreviewError::Invalid(
                "layer media_time must be finite and >= 0",
            ));
        }
        let t = layer.transform;
        if !finite4(t.x, t.y, t.scale, t.scale_x)
            || !t.scale_y.is_finite()
            || !t.opacity.is_finite()
            || t.scale <= 0.0
            || t.scale_x <= 0.0
            || t.scale_y <= 0.0
            || !(0.0..=1.0).contains(&t.opacity)
        {
            return Err(PreviewError::Invalid(
                "layer transform must be finite, scales > 0 and opacity in 0..=1",
            ));
        }
        if layer.path.is_empty() || layer.path.contains('\n') || layer.path.contains('\r') {
            return Err(PreviewError::Invalid("layer path must be a single line"));
        }
        let kind = match layer.kind {
            LayerKind::Reveal => format!("reveal:{}", layer.soft.clamp(0.0, 1.0)),
            other => other.wire().to_owned(),
        };
        let _ = writeln!(
            buf,
            "layer {} {} {} {} {} {} {} {} {}",
            kind, layer.media_time, t.x, t.y, t.scale, t.scale_x, t.scale_y, t.opacity, layer.path
        );
    }
    buf.extend_from_slice(b"layers-commit\n");
    Ok(())
}

pub(crate) fn encode_audio(buf: &mut Vec<u8>, clips: &[AudioClip]) -> Result<(), PreviewError> {
    if clips.len() > MAX_AUDIO_CLIPS {
        return Err(PreviewError::Invalid("audio clip limit is 4096"));
    }
    for clip in clips {
        validate_clip(clip)?;
    }
    buf.extend_from_slice(b"audio-begin\n");
    for clip in clips {
        let _ = write!(
            buf,
            "audio-clip {} {} {} {} {} {} {} ",
            clip.start,
            clip.duration,
            clip.offset,
            clip.rate,
            clip.gain,
            clip.fade_in,
            clip.fade_out
        );
        buf.extend_from_slice(clip.path.as_bytes());
        buf.push(b'\n');
    }
    buf.extend_from_slice(b"audio-commit\n");
    Ok(())
}

pub(crate) fn encode_transport(
    buf: &mut Vec<u8>,
    time: f64,
    playing: bool,
) -> Result<(), PreviewError> {
    if !time.is_finite() {
        return Err(PreviewError::Invalid("transport time must be finite"));
    }
    let _ = writeln!(buf, "transport {} {}", time.max(0.0), u8::from(playing));
    Ok(())
}

fn validate_clip(clip: &AudioClip) -> Result<(), PreviewError> {
    if !clip.start.is_finite()
        || !clip.duration.is_finite()
        || !clip.offset.is_finite()
        || !clip.rate.is_finite()
        || !clip.gain.is_finite()
    {
        return Err(PreviewError::Invalid("audio clip has a non-finite value"));
    }
    if clip.path.is_empty() {
        return Err(PreviewError::Invalid("audio clip path is empty"));
    }
    if has_control(&clip.path) {
        return Err(PreviewError::Invalid(
            "audio clip path contains a newline or NUL",
        ));
    }
    if clip.start < 0.0 {
        return Err(PreviewError::Invalid("audio clip start must be >= 0"));
    }
    if clip.duration <= 0.0 {
        return Err(PreviewError::Invalid("audio clip duration must be > 0"));
    }
    if clip.offset < 0.0 {
        return Err(PreviewError::Invalid("audio clip offset must be >= 0"));
    }
    if !(0.1..=8.0).contains(&clip.rate) {
        return Err(PreviewError::Invalid("audio clip rate must be in 0.1..=8"));
    }
    if !(0.0..=4.0).contains(&clip.gain) {
        return Err(PreviewError::Invalid("audio clip gain must be in 0..=4"));
    }
    if !clip.fade_in.is_finite()
        || !clip.fade_out.is_finite()
        || clip.fade_in < 0.0
        || clip.fade_out < 0.0
        || clip.fade_in + clip.fade_out > clip.duration
    {
        return Err(PreviewError::Invalid(
            "audio clip fades must be finite, >= 0 and fit duration",
        ));
    }
    Ok(())
}

fn has_control(text: &str) -> bool {
    text.bytes().any(|b| b == b'\n' || b == b'\r' || b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn line(encode: impl FnOnce(&mut Vec<u8>) -> Result<(), PreviewError>) -> String {
        let mut buf = Vec::new();
        encode(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn stdin_lines_match_electron() {
        assert_eq!(
            line(|b| {
                encode_hwnd(b, 0xff);
                Ok(())
            }),
            "hwnd ff\n"
        );
        assert_eq!(
            line(|b| {
                encode_hwnd(b, 0x10000);
                Ok(())
            }),
            "hwnd 10000\n"
        );
        assert_eq!(line(|b| encode_path(b, None)), "path\n");
        assert_eq!(line(|b| encode_path(b, Some(""))), "path\n");
        assert_eq!(
            line(|b| encode_path(b, Some("media/a b.mp4"))),
            "path media/a b.mp4\n"
        );
        assert_eq!(line(|b| encode_seek(b, 1.25)), "seek 1.25\n");
        assert_eq!(line(|b| encode_seek(b, -3.0)), "seek 0\n");
        assert_eq!(line(|b| encode_aspect(b, 16.0, 9.0)), "aspect 16 9\n");
        assert_eq!(
            line(|b| encode_layout(
                b,
                LayoutRect {
                    x: 1.5,
                    y: 2.0,
                    w: 3.0,
                    h: 4.0,
                },
            )),
            "layout 1.5 2 3 4\n"
        );
        assert_eq!(
            line(|b| encode_size_screen(
                b,
                ScreenSlot {
                    x: -10,
                    y: 20,
                    w: 640,
                    h: 360,
                    visible: true,
                },
            )),
            "size-screen -10 20 640 360 1\n"
        );
        assert_eq!(line(|b| encode_transport(b, 0.0, true)), "transport 0 1\n");
        assert_eq!(
            line(|b| encode_transport(b, -1.0, false)),
            "transport 0 0\n"
        );
    }

    #[test]
    fn audio_scene_keeps_spaces_in_path() {
        let clips = [AudioClip {
            path: "C:/media/a b.wav".into(),
            start: 1.0,
            duration: 2.5,
            offset: 0.25,
            rate: 1.0,
            gain: 0.5,
            fade_in: 0.0,
            fade_out: 0.0,
        }];
        assert_eq!(
            line(|b| encode_audio(b, &clips)),
            "audio-begin\naudio-clip 1 2.5 0.25 1 0.5 0 0 C:/media/a b.wav\naudio-commit\n"
        );
        assert_eq!(
            line(|b| encode_audio(b, &[])),
            "audio-begin\naudio-commit\n"
        );
    }

    #[test]
    fn audio_limits_match_electron() {
        let mut clip = AudioClip {
            path: "a.wav".into(),
            start: 0.0,
            duration: 1.0,
            offset: 0.0,
            rate: 1.0,
            gain: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
        };
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_ok());
        clip.rate = 0.1;
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_ok());
        clip.rate = 8.0;
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_ok());
        clip.rate = 8.01;
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_err());
        clip.rate = 0.09;
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_err());
        clip.rate = 1.0;
        clip.gain = 4.0;
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_ok());
        clip.gain = 4.01;
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_err());
        clip.gain = 1.0;
        clip.duration = 0.0;
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_err());
        clip.duration = 1.0;
        clip.path = "bad\npath".into();
        assert!(encode_audio(&mut Vec::new(), &[clip.clone()]).is_err());
        clip.path = "a.wav".into();
        let many = vec![clip; MAX_AUDIO_CLIPS + 1];
        assert!(encode_audio(&mut Vec::new(), &many).is_err());
        assert!(encode_path(&mut Vec::new(), Some("a\nb")).is_err());
        assert!(encode_seek(&mut Vec::new(), f64::NAN).is_err());
        assert!(encode_aspect(&mut Vec::new(), 0.0, 9.0).is_err());
        assert!(encode_size_screen(
            &mut Vec::new(),
            ScreenSlot {
                x: 0,
                y: 0,
                w: -1,
                h: 1,
                visible: false,
            },
        )
        .is_err());
    }

    #[test]
    fn clip_lines() {
        let mut buf = Vec::new();
        encode_clip(&mut buf, &[]).unwrap();
        encode_clip(
            &mut buf,
            &[ClipRect {
                x: -4,
                y: 10,
                w: 200,
                h: 30,
            }],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "clip 0\nclip 1 -4 10 200 30\n"
        );
        let empty = ClipRect {
            x: 0,
            y: 0,
            w: 0,
            h: 5,
        };
        assert!(encode_clip(&mut Vec::new(), &[empty]).is_err());
        let one = ClipRect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        };
        assert!(encode_clip(&mut Vec::new(), &[one; MAX_CLIP_RECTS + 1]).is_err());
    }

    #[test]
    fn stdout_parse() {
        match parse_host_line("audio-clock 1.500000000 1 0").unwrap() {
            ParsedLine::Clock {
                time,
                playing,
                available,
            } => {
                assert!((time - 1.5).abs() < 1e-12);
                assert!(playing && !available);
            }
            _ => panic!("clock"),
        }
        assert!(parse_host_line("audio-clock -1 1 1").is_none());
        assert!(parse_host_line("audio-clock 1 2 1").is_none());
        assert!(parse_host_line("audio-clock 1 1").is_none());
        assert!(parse_host_line("audio-clock 1 1 1 extra").is_none());
        assert!(parse_host_line("nope").is_none());

        match parse_host_line("preview-bounds -8 9 100 80").unwrap() {
            ParsedLine::Bounds(b) => {
                assert_eq!((b.x, b.y, b.w, b.h), (-8.0, 9.0, 100.0, 80.0));
                assert_eq!(b.visible, None);
            }
            _ => panic!("bounds"),
        }
        match parse_host_line("  preview-bounds 1 2 3 4 0  ").unwrap() {
            ParsedLine::Bounds(b) => assert_eq!(b.visible, Some(false)),
            _ => panic!("bounds"),
        }
        assert!(parse_host_line("preview-bounds 1 2 -3 4").is_none());
        assert!(parse_host_line("preview-bounds 1 2 3 4 2").is_none());
    }

    #[test]
    fn preview_pointer_events_accept_captured_coordinates() {
        for kind in ["down", "move", "up"] {
            assert!(parse_host_line(&format!("preview-pointer {kind} -0.25 1.5")).is_some());
        }
        for line in [
            "preview-pointer cancel 0 0",
            "preview-pointer down NaN 0",
            "preview-pointer move 0 inf",
            "preview-pointer up 0",
            "preview-pointer up 0 0 extra",
        ] {
            assert!(parse_host_line(line).is_none(), "accepted {line}");
        }
    }

    #[test]
    fn layers_wire_encodes_bottom_to_top_stack() {
        let encoded = line(|b| {
            encode_layers(
                b,
                &[
                    PreviewLayer {
                        path: r"C:\a b.mp4".into(),
                        media_time: 1.25,
                        soft: 0.0,
                        kind: LayerKind::Video,
                        transform: VisualTransform {
                            x: 0.1,
                            y: -0.2,
                            scale: 1.0,
                            scale_x: 1.0,
                            scale_y: 1.0,
                            opacity: 1.0,
                        },
                    },
                    PreviewLayer {
                        path: "top.png".into(),
                        media_time: 0.0,
                        soft: 0.0,
                        kind: LayerKind::Image,
                        transform: VisualTransform {
                            x: 0.0,
                            y: 0.0,
                            scale: 0.5,
                            scale_x: 1.0,
                            scale_y: 1.0,
                            opacity: 0.5,
                        },
                    },
                ],
            )
        });
        assert_eq!(
            encoded,
            "layers-begin\nlayer video 1.25 0.1 -0.2 1 1 1 1 C:\\a b.mp4\nlayer image 0 0 0 0.5 1 1 0.5 top.png\nlayers-commit\n"
        );
        assert!(encode_layers(
            &mut Vec::new(),
            &vec![
                PreviewLayer {
                    path: "x".into(),
                    media_time: 0.0,
                    soft: 0.0,
                    kind: LayerKind::Video,
                    transform: VisualTransform {
                        x: 0.0,
                        y: 0.0,
                        scale: 1.0,
                        scale_x: 1.0,
                        scale_y: 1.0,
                        opacity: 1.0,
                    },
                };
                MAX_PREVIEW_LAYERS + 1
            ],
        )
        .is_err());
    }

    #[test]
    fn transform_wire_validates_before_appending() {
        assert_eq!(
            line(|b| encode_transform(b, 0.25, -0.5, 2.0)),
            "transform 0.25 -0.5 2\n"
        );
        assert_eq!(
            line(|b| encode_transform(b, 0.0, 0.0, 1.0)),
            "transform 0 0 1\n"
        );
        for (x, y, scale) in [
            (f64::NAN, 0.0, 1.0),
            (0.0, f64::INFINITY, 1.0),
            (0.0, 0.0, f64::NEG_INFINITY),
            (0.0, 0.0, 0.0),
            (0.0, 0.0, -1.0),
        ] {
            let mut buf = b"existing\n".to_vec();
            assert!(encode_transform(&mut buf, x, y, scale).is_err());
            assert_eq!(buf, b"existing\n");
        }
    }

    #[test]
    fn visual_transform_and_gizmo_wire_are_atomic() {
        assert_eq!(
            VisualTransform::default(),
            VisualTransform {
                x: 0.0,
                y: 0.0,
                scale: 1.0,
                scale_x: 1.0,
                scale_y: 1.0,
                opacity: 1.0,
            }
        );
        let transform = VisualTransform {
            x: 0.25,
            y: -0.5,
            scale: 2.0,
            scale_x: 0.5,
            scale_y: 3.0,
            opacity: 0.25,
        };
        assert_eq!(
            line(|b| encode_visual_transform(b, transform)),
            "transform 0.25 -0.5 2 0.5 3 0.25\n"
        );
        for invalid in [
            VisualTransform {
                x: f64::NAN,
                ..transform
            },
            VisualTransform {
                y: f64::INFINITY,
                ..transform
            },
            VisualTransform {
                scale: 0.0,
                ..transform
            },
            VisualTransform {
                scale_x: -1.0,
                ..transform
            },
            VisualTransform {
                scale_y: f64::NAN,
                ..transform
            },
            VisualTransform {
                opacity: -0.1,
                ..transform
            },
            VisualTransform {
                opacity: 1.1,
                ..transform
            },
            VisualTransform {
                opacity: f64::NAN,
                ..transform
            },
        ] {
            let mut buf = b"existing\n".to_vec();
            assert!(encode_visual_transform(&mut buf, invalid).is_err());
            assert_eq!(buf, b"existing\n");
        }
        let gizmo = TransformGizmo {
            left: -0.25,
            top: 0.1,
            right: 1.25,
            bottom: 0.9,
            editable: true,
        };
        assert_eq!(
            line(|b| encode_gizmo(b, Some(gizmo))),
            "gizmo -0.25 0.1 1.25 0.9 1\n"
        );
        assert_eq!(
            line(|b| encode_gizmo(
                b,
                Some(TransformGizmo {
                    editable: false,
                    ..gizmo
                })
            )),
            "gizmo -0.25 0.1 1.25 0.9 0\n"
        );
        assert_eq!(line(|b| encode_gizmo(b, None)), "gizmo\n");
        for invalid in [
            TransformGizmo {
                left: f64::NAN,
                ..gizmo
            },
            TransformGizmo {
                bottom: f64::INFINITY,
                ..gizmo
            },
            TransformGizmo {
                right: -0.25,
                ..gizmo
            },
            TransformGizmo { top: 1.0, ..gizmo },
        ] {
            let mut buf = b"existing\n".to_vec();
            assert!(encode_gizmo(&mut buf, Some(invalid)).is_err());
            assert_eq!(buf, b"existing\n");
        }
        assert_eq!(line(|b| encode_guides(b, &[])), "guides\n");
        assert_eq!(
            line(|b| encode_guides(
                b,
                &[
                    AlignGuide {
                        vertical: true,
                        position: 0.5,
                    },
                    AlignGuide {
                        vertical: false,
                        position: 0.25,
                    },
                ]
            )),
            "guides 0 0.5 1 0.25\n"
        );
        let mut buf = b"existing\n".to_vec();
        assert!(encode_guides(
            &mut buf,
            &[AlignGuide {
                vertical: true,
                position: f64::NAN,
            }]
        )
        .is_err());
        assert_eq!(buf, b"existing\n");
        let too_many: Vec<_> = (0..MAX_ALIGN_GUIDES + 1)
            .map(|i| AlignGuide {
                vertical: true,
                position: i as f64 * 0.1,
            })
            .collect();
        assert!(encode_guides(&mut buf, &too_many).is_err());
        assert_eq!(buf, b"existing\n");
    }

    #[test]
    fn source_generation_path_and_display_aspect_protocol() {
        assert_eq!(
            line(|b| encode_path_token(b, Some("C:/a b.mp4"), 42)),
            "path-token 42 C:/a b.mp4\n"
        );
        assert_eq!(
            line(|b| encode_path_token(b, None, u64::MAX)),
            "path-token 18446744073709551615\n"
        );
        assert_eq!(
            line(|b| encode_path_token(b, Some(""), 0)),
            "path-token 0\n"
        );
        let mut buffer = b"existing\n".to_vec();
        assert!(encode_path_token(&mut buffer, Some("bad\npath"), 1).is_err());
        assert_eq!(buffer, b"existing\n");
        match parse_host_line("source-aspect 42 1.3333333333333333").unwrap() {
            ParsedLine::SourceAspect { generation, aspect } => {
                assert_eq!(generation, 42);
                assert!((aspect - 4.0 / 3.0).abs() < 1e-12);
            }
            _ => panic!("source aspect"),
        }
        for invalid in [
            "source-aspect -1 1",
            "source-aspect 1 0",
            "source-aspect 1 -1",
            "source-aspect 1 NaN",
            "source-aspect 1 inf",
            "source-aspect 1 1 extra",
            "source-aspect 18446744073709551616 1",
        ] {
            assert!(parse_host_line(invalid).is_none(), "accepted {invalid}");
        }
    }

    #[test]
    fn clock_extrapolation_matches_electron() {
        let t0 = Instant::now();
        let playing = ClockSample {
            time: 10.0,
            playing: true,
            available: true,
            received_at: t0,
        };
        let at = playing
            .extrapolated(t0 + Duration::from_millis(500))
            .unwrap();
        assert!((at.time - 10.5).abs() < 0.02);
        assert!(playing
            .extrapolated(t0 + Duration::from_millis(1000))
            .is_some());
        assert!(playing
            .extrapolated(t0 + Duration::from_millis(1001))
            .is_none());
        let paused = ClockSample {
            playing: false,
            available: false,
            ..playing
        };
        let held = paused
            .extrapolated(t0 + Duration::from_millis(400))
            .unwrap();
        assert!((held.time - 10.0).abs() < 1e-9);
        assert!(!held.playing && !held.available);
    }
}
