use todo_core::{find_clip, linked_clip_ids, EditCommand, Project, TrackKind};

use crate::input::{Intent, TimelineTool};

use super::drag_session::{DragGesture, DragSession};
use super::hotkeys::tool_for_key;
use super::marquee::marquee_selection;
use todo_core::workspace_duration;

use super::zoom::{time_at, timeline_scale, wheel_zoom, ZOOM_DEFAULT};

#[derive(Debug, Clone, PartialEq)]
pub enum TimelineOutcome {
    None,
    /// Pointer scrub. The editor stops playback and commits this time.
    Scrub(f64),
    Edit(EditCommand),
    /// Board scroll left after a zoom step.
    Zoom(f64),
}

#[derive(Debug, Clone)]
pub struct TimelineController {
    tool: TimelineTool,
    selected_ids: Vec<String>,
    active_track_id: Option<String>,
    linked_selection: bool,
    snapping: bool,
    drag_base: Option<Project>,
    drag_playhead: f64,
    drag_scroll_left: f64,
    drag: Option<DragSession>,
    /// Fit-to-width multiplier (`1` = workspace fills the lane).
    zoom: f64,
    scroll_left: f64,
    view_width: f64,
    pub(crate) snap_target: Option<f64>,
}

impl Default for TimelineController {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineController {
    pub fn new() -> Self {
        Self {
            tool: TimelineTool::Select,
            selected_ids: Vec::new(),
            active_track_id: None,
            linked_selection: true,
            snapping: true,
            drag_base: None,
            drag_playhead: 0.0,
            drag_scroll_left: 0.0,
            drag: None,
            zoom: ZOOM_DEFAULT,
            scroll_left: 0.0,
            view_width: 0.0,
            snap_target: None,
        }
    }

    pub fn tool(&self) -> TimelineTool {
        self.tool
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected_ids.first().map(String::as_str)
    }

    pub fn selected_ids(&self) -> &[String] {
        &self.selected_ids
    }
    pub fn active_track_id(&self) -> Option<&str> {
        self.active_track_id.as_deref()
    }
    pub fn linked_selection(&self) -> bool {
        self.linked_selection
    }
    pub fn snapping(&self) -> bool {
        self.snapping
    }
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.drag_base = None;
        self.snap_target = None;
    }
    pub fn reconcile(&mut self, project: &Project) {
        self.selected_ids
            .retain(|id| find_clip(project, id).is_some());
        if !project
            .tracks
            .iter()
            .any(|t| Some(t.id.as_str()) == self.active_track_id.as_deref())
        {
            self.active_track_id = project.tracks.first().map(|t| t.id.clone());
        }
    }
    pub fn select_clip(&mut self, project: &Project, id: &str, additive: bool) {
        let Some(found) = find_clip(project, id) else {
            return;
        };
        self.active_track_id = Some(found.track_id);
        let mut group = if self.linked_selection {
            linked_clip_ids(project, &[id.to_owned()])
        } else {
            vec![id.to_owned()]
        };
        group.sort_by_key(|member| member != id);
        if additive && self.selected_ids.contains(&id.to_owned()) {
            self.selected_ids.retain(|id| !group.contains(id));
        } else {
            if !additive {
                self.selected_ids.clear();
            }
            for id in group {
                if !self.selected_ids.contains(&id) {
                    self.selected_ids.push(id);
                }
            }
        }
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn px_per_sec(&self, project: &Project) -> f64 {
        self.scale(project).px_per_sec
    }

    pub fn scroll_left(&self) -> f64 {
        self.scroll_left
    }

    pub fn view_width(&self) -> f64 {
        self.view_width
    }

    fn scale(&self, project: &Project) -> super::zoom::TimelineScale {
        timeline_scale(workspace_duration(project), self.view_width, self.zoom)
    }

    pub fn drag(&self) -> Option<&DragSession> {
        self.drag.as_ref()
    }

    pub fn handle(&mut self, project: &Project, playhead: f64, intent: &Intent) -> TimelineOutcome {
        match intent {
            Intent::SelectClip { clip_id, additive } => {
                self.select_clip(project, clip_id, *additive);
                TimelineOutcome::None
            }
            Intent::SelectAll => {
                self.selected_ids = project
                    .tracks
                    .iter()
                    .filter(|t| !t.locked)
                    .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
                    .collect();
                TimelineOutcome::None
            }
            Intent::ClearSelection => {
                self.selected_ids.clear();
                TimelineOutcome::None
            }
            Intent::ToggleLinkedSelection => {
                self.linked_selection = !self.linked_selection;
                if self.linked_selection {
                    self.selected_ids = linked_clip_ids(project, &self.selected_ids);
                }
                TimelineOutcome::None
            }
            Intent::ToggleSnapping => {
                self.snapping = !self.snapping;
                TimelineOutcome::None
            }
            Intent::BoardPointerDown {
                view_x,
                scroll_left,
            } => self.pointer_down(project, None, *view_x, *scroll_left, playhead),
            Intent::LanePointerDown {
                track_id,
                view_x,
                scroll_left,
            } => self.pointer_down(
                project,
                Some(track_id.as_str()),
                *view_x,
                *scroll_left,
                playhead,
            ),
            Intent::MarqueePointerDown {
                view_x,
                track_index,
                scroll_left,
                additive,
            } => self.marquee_down(project, *view_x, *track_index, *scroll_left, *additive),
            Intent::ClipPointerDown {
                clip_id,
                view_x,
                scroll_left,
                local_x,
                width_px,
                alt,
            } => self.clip_down(
                project,
                clip_id,
                *view_x,
                *scroll_left,
                *local_x,
                *width_px,
                *alt,
                playhead,
            ),
            Intent::PointerMove {
                view_x,
                scroll_left,
                track_index,
            } => {
                self.scroll_left = finite_or(*scroll_left, self.scroll_left);
                self.drag_move(project, *view_x, self.scroll_left, *track_index)
            }
            Intent::PointerUp => {
                self.cancel_drag();
                TimelineOutcome::None
            }
            Intent::ToolClicked(TimelineTool::Select) => {
                self.tool = TimelineTool::Select;
                TimelineOutcome::None
            }
            Intent::ToolClicked(TimelineTool::Razor) => {
                self.tool = if self.tool == TimelineTool::Razor {
                    TimelineTool::Select
                } else {
                    TimelineTool::Razor
                };
                TimelineOutcome::None
            }
            Intent::RazorAtPlayhead => {
                TimelineOutcome::Edit(self.split(project, playhead, None, false))
            }
            Intent::ToggleMute { track_id } => TimelineOutcome::Edit(EditCommand::ToggleMute {
                track_id: track_id.clone(),
            }),
            Intent::DeleteTrack { track_id } => TimelineOutcome::Edit(EditCommand::DeleteTrack {
                track_id: track_id.clone(),
            }),
            Intent::Key { code, typing } => {
                if !typing && *code == crate::KeyCode::Escape {
                    let restored = self
                        .drag
                        .as_ref()
                        .and_then(|session| match session.gesture() {
                            DragGesture::Marquee { base_selection, .. } => {
                                Some(base_selection.clone())
                            }
                            _ => None,
                        });
                    if let Some(base_selection) = restored {
                        self.selected_ids = base_selection;
                    } else {
                        self.selected_ids.clear();
                    }
                    self.cancel_drag();
                }
                if let Some(tool) = tool_for_key(*code, *typing, self.tool) {
                    self.tool = tool;
                }
                TimelineOutcome::None
            }
            Intent::Wheel {
                delta_y,
                view_x,
                scroll_left,
                alt,
            } => {
                self.scroll_left = finite_or(*scroll_left, self.scroll_left);
                match wheel_zoom(
                    self.zoom,
                    *delta_y,
                    *view_x,
                    self.scroll_left,
                    *alt,
                    workspace_duration(project),
                    self.view_width,
                ) {
                    Some(hit) => {
                        self.zoom = hit.zoom;
                        self.scroll_left = hit.scroll_left;
                        TimelineOutcome::Zoom(hit.scroll_left)
                    }
                    None => TimelineOutcome::None,
                }
            }
            Intent::Viewport { scroll_left, width } => {
                self.scroll_left = finite_or(*scroll_left, self.scroll_left);
                self.view_width = finite_or(*width, self.view_width).max(0.0);
                TimelineOutcome::None
            }
            _ => TimelineOutcome::None,
        }
    }

    fn pointer_down(
        &mut self,
        project: &Project,
        track_id: Option<&str>,
        view_x: f64,
        scroll_left: f64,
        playhead: f64,
    ) -> TimelineOutcome {
        if self.drag.is_some() {
            return TimelineOutcome::None;
        }
        self.scroll_left = finite_or(scroll_left, self.scroll_left);
        self.active_track_id = track_id
            .map(str::to_owned)
            .or_else(|| self.active_track_id.clone());
        let pps = self.px_per_sec(project);
        let raw_time = time_at(view_x, self.scroll_left, pps);
        let time = self.frame_time(project, raw_time);
        if self.tool == TimelineTool::Razor && track_id.is_some() {
            // Lane press cuts in place. Ruler / board press scrubs below.
            let hit = super::snapping::snap_position(
                project,
                raw_time,
                0.0,
                &[],
                playhead,
                pps,
                self.snapping,
            );
            self.snap_target = hit.target;
            return TimelineOutcome::Edit(self.split(project, hit.value, track_id, false));
        }
        // Ruler area follows the playhead without cutting.
        self.drag = Some(DragSession::seek());
        self.selected_ids.clear();
        TimelineOutcome::Scrub(time.min(workspace_duration(project)))
    }

    fn marquee_down(
        &mut self,
        project: &Project,
        view_x: f64,
        track_index: usize,
        scroll_left: f64,
        additive: bool,
    ) -> TimelineOutcome {
        if self.drag.is_some() || self.tool != TimelineTool::Select {
            return TimelineOutcome::None;
        }
        self.scroll_left = finite_or(scroll_left, self.scroll_left);
        let track_index = track_index.min(project.tracks.len().saturating_sub(1));
        let pps = self.px_per_sec(project);
        let time = self.frame_time(project, time_at(view_x, self.scroll_left, pps));
        let base = if additive {
            self.selected_ids.clone()
        } else {
            Vec::new()
        };
        if !additive {
            self.selected_ids.clear();
        }
        self.drag = Some(DragSession::marquee(
            time,
            track_index,
            additive,
            base.clone(),
        ));
        self.selected_ids = marquee_selection(
            project,
            time,
            time,
            track_index,
            track_index,
            self.linked_selection,
            additive,
            &base,
        );
        TimelineOutcome::None
    }

    #[allow(clippy::too_many_arguments)] // Explicit timeline geometry at this projection/input boundary.
    fn clip_down(
        &mut self,
        project: &Project,
        clip_id: &str,
        view_x: f64,
        scroll_left: f64,
        local_x: f64,
        width_px: f64,
        alt: bool,
        playhead: f64,
    ) -> TimelineOutcome {
        if self.drag.is_some() {
            return TimelineOutcome::None;
        }
        self.scroll_left = finite_or(scroll_left, self.scroll_left);
        let Some(found) = find_clip(project, clip_id) else {
            return TimelineOutcome::None;
        };
        let track_id = found.track_id.clone();
        let origin_start = found.clip.start;
        let locked = found.track.locked;
        let media_id = found.clip.media_id().map(str::to_owned);
        drop(found);
        if !self.selected_ids.iter().any(|id| id == clip_id) {
            self.select_clip(project, clip_id, false);
        } else if let Some(index) = self.selected_ids.iter().position(|id| id == clip_id) {
            self.selected_ids.swap(0, index);
        }
        self.active_track_id = Some(track_id.clone());

        let raw = time_at(view_x, self.scroll_left, self.px_per_sec(project));
        let hit = super::snapping::snap_position(
            project,
            raw,
            0.0,
            &[],
            playhead,
            self.px_per_sec(project),
            self.snapping,
        );
        let time = hit.value;
        if self.tool == TimelineTool::Razor {
            let ids = if self.linked_selection {
                linked_clip_ids(project, &[clip_id.to_owned()])
            } else {
                vec![clip_id.to_owned()]
            };
            return TimelineOutcome::Edit(EditCommand::Split {
                clip_ids: ids,
                time,
            });
        }
        if locked {
            return TimelineOutcome::None;
        }
        let media_duration = project
            .media
            .iter()
            .find(|item| Some(&item.id) == media_id.as_ref())
            .map(|item| item.duration);
        self.drag = Some(DragSession::begin_clip(
            clip_id.to_string(),
            origin_start,
            view_x,
            local_x,
            width_px,
            alt,
            media_duration,
        ));
        self.drag_base = Some(project.clone());
        self.drag_playhead = playhead;
        self.drag_scroll_left = self.scroll_left;
        TimelineOutcome::None
    }

    fn drag_move(
        &mut self,
        project: &Project,
        view_x: f64,
        scroll_left: f64,
        track_index: Option<usize>,
    ) -> TimelineOutcome {
        let Some(drag) = self.drag.clone() else {
            return TimelineOutcome::None;
        };
        let base = self.drag_base.as_ref().unwrap_or(project);
        let pps = self.px_per_sec(base);
        if let DragGesture::Marquee {
            origin_time,
            origin_track,
            additive,
            base_selection,
            ..
        } = drag.gesture()
        {
            let time = self.frame_time(project, time_at(view_x, scroll_left, pps));
            let track = track_index
                .unwrap_or(*origin_track)
                .min(project.tracks.len().saturating_sub(1));
            if let Some(next) = drag.with_marquee_at(time, track) {
                self.drag = Some(next);
            }
            self.selected_ids = marquee_selection(
                project,
                *origin_time,
                time,
                *origin_track,
                track,
                self.linked_selection,
                *additive,
                base_selection,
            );
            return TimelineOutcome::None;
        }
        if let DragGesture::Move { origin_view_x, .. } = drag.gesture() {
            let raw_delta = (view_x - origin_view_x + scroll_left - self.drag_scroll_left) / pps;
            let hit = super::snapping::snap_group_move(
                base,
                &self.selected_ids,
                raw_delta,
                self.drag_playhead,
                pps,
                self.snapping,
            );
            self.snap_target = hit.target;
            // Pointer indices belong to the displayed project: automatic video
            // lanes can shift the audio section during this very gesture.
            let track_id = track_index.and_then(|index| {
                let target = project.tracks.get(index)?;
                let primary = self.selected_ids.first()?;
                let source = find_clip(base, primary)?;
                let video_to_video =
                    source.track.kind == TrackKind::Video && target.kind == TrackKind::Video;
                let audio_family =
                    source.track.kind != TrackKind::Video && target.kind != TrackKind::Video;
                (video_to_video || audio_family).then(|| target.id.clone())
            });
            return TimelineOutcome::Edit(EditCommand::MoveClips {
                clip_ids: self.selected_ids.clone(),
                delta: hit.value,
                track_id,
            });
        }
        if let DragGesture::Trim { clip_id, edge, .. } | DragGesture::Stretch { clip_id, edge } =
            drag.gesture()
        {
            let ids = if self.linked_selection {
                linked_clip_ids(base, std::slice::from_ref(clip_id))
            } else {
                vec![clip_id.clone()]
            };
            let hit = super::snapping::snap_position(
                base,
                time_at(view_x, scroll_left, pps),
                0.0,
                &ids,
                self.drag_playhead,
                pps,
                self.snapping,
            );
            self.snap_target = hit.target;
            return TimelineOutcome::Edit(
                if matches!(drag.gesture(), DragGesture::Stretch { .. }) {
                    EditCommand::StretchClips {
                        primary_id: clip_id.clone(),
                        edge: *edge,
                        time: hit.value,
                        linked: self.linked_selection,
                    }
                } else {
                    EditCommand::TrimClips {
                        primary_id: clip_id.clone(),
                        edge: *edge,
                        time: hit.value,
                        linked: self.linked_selection,
                    }
                },
            );
        }
        let time = self.frame_time(base, time_at(view_x, scroll_left, pps));
        TimelineOutcome::Scrub(time.min(workspace_duration(project)))
    }

    fn split(
        &mut self,
        project: &Project,
        time: f64,
        track_id: Option<&str>,
        then_seek: bool,
    ) -> EditCommand {
        let time = if then_seek {
            time.clamp(0.0, workspace_duration(project))
        } else {
            time.max(0.0)
        };
        let time = self.frame_time(project, time);
        let ids: Vec<_> = project
            .tracks
            .iter()
            .filter(|t| track_id.map(|id| id == t.id).unwrap_or(true))
            .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
            .collect();
        let ids = if track_id.is_none() && !self.selected_ids.is_empty() {
            self.selected_ids.clone()
        } else {
            ids
        };
        let ids = if self.linked_selection {
            linked_clip_ids(project, &ids)
        } else {
            ids
        };
        EditCommand::Split {
            clip_ids: ids,
            time,
        }
    }

    fn frame_time(&self, project: &Project, time: f64) -> f64 {
        let fps = if project.fps.is_finite() && project.fps > 0.0 {
            project.fps
        } else {
            30.0
        };
        (time.max(0.0) * fps).round() / fps
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}
