pub use crate::inspector::{ClipProperties, SourceProperties, StyleOption, TextProperties};
mod library;
mod messages;
pub(crate) use library::LibraryState;
pub use messages::status_text;
use todo_core::{ClipTransform, MediaKind};

use todo_core::{Project, TrackKind};

use crate::input::{EditorFocus, TimelineTool};
use todo_core::workspace_duration;

use crate::timeline::{playhead_x, timeline_scale, DragGesture, DragKind, LABEL_W};

/// Visible lane range in pixels, origin at the start of the lane (after the labels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub left: f64,
    pub right: f64,
}

impl Viewport {
    pub fn from_scroll(scroll_left: f64, client_width: f64) -> Self {
        Self {
            left: (scroll_left - LABEL_W).max(0.0),
            right: (scroll_left + client_width - LABEL_W).max(0.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub time: f64,
    pub x: f64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipView {
    pub id: String,
    /// Text clips have no source media, thumbnails or waveform.
    pub text: bool,
    pub name: String,
    pub start: f64,
    pub duration: f64,
    pub offset: f64,
    pub rate: f64,
    pub left: f64,
    pub width: f64,
    pub selected: bool,
    pub audio: bool,
    pub media_path: Option<String>,
    /// `has_audio != Some(false)`. The UI loads peaks when this is set on an audio clip.
    pub has_audio: bool,
    pub wave_left: f64,
    pub wave_width: f64,
    pub transform: ClipTransform,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackView {
    pub id: String,
    pub name: String,
    pub kind: TrackKind,
    pub muted: bool,
    pub locked: bool,
    /// Video rows hide the mute button.
    pub mute_control: bool,
    pub can_move_up: bool,
    pub can_move_down: bool,
    pub clips: Vec<ClipView>,
}

/// Rubber-band overlay in lane pixels × track row indices.
#[derive(Debug, Clone, PartialEq)]
pub struct MarqueeView {
    pub x0: f64,
    pub x1: f64,
    pub track_lo: usize,
    pub track_hi: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineViewModel {
    pub move_targets: Vec<MoveTargetView>,
    pub selected_ids: Vec<String>,
    pub active_track_id: Option<String>,
    pub linked_selection: bool,
    pub snapping: bool,
    pub fps: f64,
    pub can_delete: bool,
    pub tool: TimelineTool,
    pub razor_mode: bool,
    pub selected_id: Option<String>,
    pub px_per_sec: f64,
    /// Configured composition end. The scrollable axis may extend past it.
    pub workspace_end: f64,
    pub axis: f64,
    pub lane_width: f64,
    pub board_width: f64,
    pub playhead_x: f64,
    pub label_w: f64,
    pub scroll_left: f64,
    pub viewport: Viewport,
    pub ticks: Vec<Tick>,
    pub tracks: Vec<TrackView>,
    pub drag: Option<DragKind>,
    pub marquee: Option<MarqueeView>,
    pub snap_target: Option<f64>,
    pub drop_preview: Option<DropPreview>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditorViewModel {
    pub library: LibraryViewModel,
    pub focus: EditorFocus,
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_paste: bool,
    pub dirty: bool,
    pub status: Option<String>,
    pub project_name: String,
    pub clip_name: Option<String>,
    pub current_time: f64,
    pub duration: f64,
    /// Fixed user-set sequence length (`project.duration`), if any.
    pub sequence_duration: Option<f64>,
    /// Program-canvas pixel size (`project.width` × `project.height`).
    pub output_width: u32,
    pub output_height: u32,
    /// How newly placed media is scaled onto the program canvas.
    pub media_scale: todo_core::MediaScaleMode,
    pub time_label: String,
    pub duration_label: String,
    pub playing: bool,
    pub can_play: bool,
    pub importing: bool,
    pub monitor_active: bool,
    /// Program vs source monitor pane.
    pub monitor_pane: todo_application::MonitorPane,
    /// True when library media is loaded into the source monitor.
    pub source_available: bool,
    pub poll_clock: bool,
    pub clock_epoch: u64,
    pub timeline: TimelineViewModel,
    pub preview: PreviewView,
    pub properties: Option<ClipProperties>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryItemView {
    pub item: LibraryMediaView,
    pub uses: usize,
    pub selected: bool,
    pub locked_references: bool,
    /// Set when the right pane is showing cross-folder search hits.
    pub folder_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryFolderView {
    pub id: String,
    pub name: String,
    pub child_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LibraryEntryView {
    Folder(LibraryFolderView),
    Media(LibraryItemView),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FolderNodeView {
    pub id: String,
    pub name: String,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
    pub selected: bool,
    pub children: Vec<FolderNodeView>,
}

/// VS Code–style explorer row: folders and media share one indented tree.
#[derive(Debug, Clone, PartialEq)]
pub enum ExplorerNode {
    Folder {
        id: String,
        name: String,
        depth: usize,
        expanded: bool,
        selected: bool,
        children: Vec<ExplorerNode>,
    },
    Media {
        depth: usize,
        item: LibraryItemView,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LibraryViewModel {
    /// Unified explorer tree (folders + nested media).
    pub explorer: Vec<ExplorerNode>,
    pub tree: Vec<FolderNodeView>,
    pub entries: Vec<LibraryEntryView>,
    /// Media rows only (current folder or search). Kept for drag/list helpers.
    pub items: Vec<LibraryItemView>,
    pub selected_id: Option<String>,
    pub selected_folder_id: Option<String>,
    pub query: String,
    pub searching: bool,
}

impl TimelineViewModel {
    #[allow(clippy::too_many_arguments)] // Explicit timeline geometry at this projection/input boundary.
    pub fn from_timeline(
        project: &Project,
        playhead: f64,
        tool: TimelineTool,
        selected_id: Option<&str>,
        zoom: f64,
        scroll_left: f64,
        view_width: f64,
        drag: Option<DragKind>,
        marquee: Option<MarqueeView>,
    ) -> Self {
        let workspace_end = workspace_duration(project);
        let axis = todo_core::timeline_extent(project);
        let scale = timeline_scale(workspace_end, view_width, zoom);
        let px_per_sec = scale.px_per_sec;
        let lane_width = scale.lane_width.max(axis * px_per_sec);
        let viewport = Viewport::from_scroll(scroll_left, view_width);
        let tracks = project
            .tracks
            .iter()
            .map(|track| {
                let audio = !track.kind.is_visual();
                let clips = track
                    .clips
                    .iter()
                    .map(|clip| {
                        let source = clip.media();
                        let media = source.and_then(|source| {
                            project.media.iter().find(|item| item.id == source.media_id)
                        });
                        let (wave_left, wave_width) = wave_span(
                            clip.start,
                            clip.duration,
                            px_per_sec,
                            viewport.left,
                            viewport.right,
                        );
                        ClipView {
                            id: clip.id.clone(),
                            text: clip.is_text(),
                            name: todo_core::clip_display_name(clip, media),
                            start: clip.start,
                            duration: clip.duration,
                            offset: source.map_or(0.0, |source| source.offset),
                            rate: source.map_or(1.0, |source| source.rate),
                            left: clip.start * px_per_sec,
                            width: (clip.duration * px_per_sec).max(4.0),
                            selected: selected_id == Some(clip.id.as_str()),
                            audio,
                            media_path: media.and_then(|item| item.path.clone()),
                            has_audio: media.is_some_and(|item| item.has_audio != Some(false)),
                            wave_left,
                            wave_width,
                            transform: clip.transform,
                        }
                    })
                    .collect();
                TrackView {
                    id: track.id.clone(),
                    name: track.name.clone(),
                    kind: track.kind,
                    muted: track.muted,
                    locked: track.locked,
                    mute_control: audio,
                    can_move_up: todo_core::can_move_track_up(project, &track.id),
                    can_move_down: todo_core::can_move_track_down(project, &track.id),
                    clips,
                }
            })
            .collect();
        Self {
            move_targets: Vec::new(),
            selected_ids: selected_id
                .map(|id| vec![id.to_owned()])
                .unwrap_or_default(),
            active_track_id: None,
            linked_selection: true,
            snapping: true,
            fps: project.fps,
            can_delete: selected_id.is_some(),
            tool,
            razor_mode: tool == TimelineTool::Razor,
            selected_id: selected_id.map(str::to_string),
            px_per_sec,
            workspace_end,
            axis,
            lane_width,
            board_width: LABEL_W + lane_width,
            playhead_x: playhead_x(playhead, px_per_sec),
            label_w: LABEL_W,
            scroll_left,
            viewport,
            ticks: ticks(axis, px_per_sec, project.fps),
            tracks,
            drag,
            marquee,
            snap_target: None,
            drop_preview: None,
        }
    }
}

impl MarqueeView {
    pub fn from_gesture(gesture: &DragGesture, px_per_sec: f64) -> Option<Self> {
        let DragGesture::Marquee {
            origin_time,
            origin_track,
            current_time,
            current_track,
            ..
        } = gesture
        else {
            return None;
        };
        let x0 = origin_time.max(0.0) * px_per_sec;
        let x1 = current_time.max(0.0) * px_per_sec;
        let (track_lo, track_hi) = if origin_track <= current_track {
            (*origin_track, *current_track)
        } else {
            (*current_track, *origin_track)
        };
        Some(Self {
            x0: x0.min(x1),
            x1: x0.max(x1),
            track_lo,
            track_hi,
        })
    }
}

const TICK_MIN_PX: f64 = 56.0;
const COARSE_TICK_STEPS: [f64; 12] = [
    0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
];

fn sane_fps(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    }
}

/// Major ruler step in seconds for the current zoom. Includes 1/2/5/10-frame
/// steps so zoomed-in boards do not collapse to duplicate whole-second labels.
pub fn tick_step(px_per_sec: f64, fps: f64) -> f64 {
    if !px_per_sec.is_finite() || px_per_sec <= 0.0 {
        return 600.0;
    }
    let frame = 1.0 / sane_fps(fps);
    let mut steps = Vec::with_capacity(16);
    for n in [1.0_f64, 2.0, 5.0, 10.0] {
        steps.push(frame * n);
    }
    steps.extend_from_slice(&COARSE_TICK_STEPS);
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    steps.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    for step in steps {
        if step * px_per_sec >= TICK_MIN_PX {
            return step;
        }
    }
    600.0
}

/// Whole-second clock (`m:ss`) for transport / coarse ticks.
pub fn fmt_tick(sec: f64) -> String {
    let sec = if sec.is_finite() && sec >= 0.0 {
        sec
    } else {
        0.0
    };
    let minutes = (sec / 60.0).floor() as i64;
    let seconds = (sec % 60.0).floor() as i64;
    format!("{minutes}:{seconds:02}")
}

/// Ruler label: `m:ss` when step ≥ 1s, otherwise `m:ss:ff` (frames).
pub fn fmt_ruler_tick(sec: f64, fps: f64, show_frames: bool) -> String {
    if !show_frames {
        return fmt_tick(sec);
    }
    let sec = if sec.is_finite() && sec >= 0.0 {
        sec
    } else {
        0.0
    };
    let fps = sane_fps(fps);
    let fps_i = fps.round().max(1.0) as i64;
    let total_frames = (sec * fps).round() as i64;
    let frames = total_frames.rem_euclid(fps_i);
    let total_secs = total_frames / fps_i;
    let seconds = total_secs.rem_euclid(60);
    let minutes = total_secs / 60;
    format!("{minutes}:{seconds:02}:{frames:02}")
}

pub fn fmt_clock(sec: f64) -> String {
    if !sec.is_finite() || sec < 0.0 {
        "0:00".to_string()
    } else {
        fmt_tick(sec)
    }
}

fn ticks(axis: f64, px_per_sec: f64, fps: f64) -> Vec<Tick> {
    let fps = sane_fps(fps);
    let step = tick_step(px_per_sec, fps);
    if step <= 0.0 || !axis.is_finite() || axis < 0.0 {
        return Vec::new();
    }
    let show_frames = step + 1e-9 < 1.0;
    let mut out = Vec::new();
    let count = (axis / step).floor() as i64;
    for i in 0..=count {
        let raw = i as f64 * step;
        let time = if show_frames {
            (raw * fps).round() / fps
        } else {
            (raw * 1000.0).round() / 1000.0
        };
        if time > axis + 1e-6 {
            break;
        }
        out.push(Tick {
            x: time * px_per_sec,
            label: fmt_ruler_tick(time, fps, show_frames),
            time,
        });
    }
    out
}

/// Clip-local waveform window, matching `waveformViewport`.
pub fn wave_span(
    start: f64,
    duration: f64,
    px_per_sec: f64,
    view_left: f64,
    view_right: f64,
) -> (f64, f64) {
    let clip_left = start * px_per_sec;
    let clip_width = duration * px_per_sec;
    let left = 0.0_f64.max(clip_width.min(view_left - clip_left));
    let right = left.max(clip_width.min(view_right - clip_left));
    if !left.is_finite() || !right.is_finite() || !clip_width.is_finite() || clip_width <= 0.0 {
        (0.0, 0.0)
    } else {
        (left, right - left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoomed_ticks_use_unique_frame_labels() {
        // ~10px per frame at 30fps → one-frame step once labels fit.
        let px_per_sec = 30.0 * 60.0; // 60px/frame
        let step = tick_step(px_per_sec, 30.0);
        assert!((step - 1.0 / 30.0).abs() < 1e-9, "step {step}");
        let marks = ticks(1.0, px_per_sec, 30.0);
        assert!(marks.len() > 10);
        let labels: Vec<_> = marks.iter().map(|t| t.label.as_str()).collect();
        assert!(labels.contains(&"0:00:00"));
        assert!(labels.contains(&"0:00:01"));
        assert!(labels.contains(&"0:00:15"));
        let unique = labels.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            unique.len(),
            labels.len(),
            "duplicate ruler labels: {labels:?}"
        );
    }

    #[test]
    fn coarse_ticks_keep_whole_second_labels() {
        let step = tick_step(48.0, 30.0);
        assert!((step - 2.0).abs() < 1e-9, "step {step}");
        let marks = ticks(10.0, 48.0, 30.0);
        assert_eq!(marks[0].label, "0:00");
        assert!(marks.iter().any(|t| t.label == "0:02"));
        assert!(marks
            .iter()
            .all(|t| !t.label.contains(':') || t.label.matches(':').count() == 1));
    }

    #[test]
    fn fmt_ruler_tick_shows_frames_when_asked() {
        assert_eq!(fmt_ruler_tick(0.0, 30.0, true), "0:00:00");
        assert_eq!(fmt_ruler_tick(1.0 / 30.0, 30.0, true), "0:00:01");
        assert_eq!(fmt_ruler_tick(1.0, 30.0, true), "0:01:00");
        assert_eq!(fmt_ruler_tick(65.0, 30.0, false), "1:05");
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryMediaView {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub duration: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub kind: MediaKind,
    pub visual: bool,
    pub has_audio: bool,
    pub place_duration: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MoveTargetView {
    pub track_id: String,
    pub name: String,
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct DropPreview {
    pub media_id: String,
    pub track_id: String,
    pub start: f64,
    pub duration: f64,
    pub allowed: bool,
    pub snap_target: Option<f64>,
}
/// Smart-align guide in normalized preview-window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignGuideView {
    /// `true` = vertical line (fixed X); `false` = horizontal (fixed Y).
    pub vertical: bool,
    pub position: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewView {
    pub clip_id: Option<String>,
    pub transform: ClipTransform,
    pub editable: bool,
    pub aspect: Option<(u32, u32)>,
    /// Current selected picture bounds in normalized preview-window coordinates.
    /// Uses decoded aspect when available, otherwise source dimensions.
    /// Unknown source geometry has no box, avoiding a guessed host aspect.
    pub selection: Option<todo_core::TransformBounds>,
    /// Active align guides while dragging a transform (window coordinates).
    pub align_guides: Vec<AlignGuideView>,
    pub viewport: crate::PreviewViewport,
    pub projection: Option<crate::PreviewProjection>,
}
