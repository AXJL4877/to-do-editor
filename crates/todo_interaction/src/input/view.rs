use super::EditorController;
use crate::presentation::*;
use crate::timeline::DragSession;
use crate::transform::AlignAxis;
use todo_application::{AssetRequest, EditorSession};
use todo_core::{find_clip, project_duration, resolve_layers};

impl EditorController {
    pub fn view(&self, editor: &EditorSession) -> EditorViewModel {
        let project = editor.project();
        let picture = editor.picture();
        let drag = self.timeline.drag();
        let mut timeline = TimelineViewModel::from_timeline(
            project,
            editor.playhead(),
            self.timeline.tool(),
            self.timeline.selected_id(),
            self.timeline.zoom(),
            self.timeline.scroll_left(),
            self.timeline.view_width(),
            drag.map(DragSession::kind),
            drag.and_then(|d| {
                MarqueeView::from_gesture(d.gesture(), self.timeline.px_per_sec(project))
            }),
        );
        timeline.selected_ids = self.timeline.selected_ids().to_vec();
        let source = self
            .timeline
            .selected_id()
            .and_then(|id| find_clip(project, id))
            .map(|f| f.track_id);
        timeline.move_targets = project
            .tracks
            .iter()
            .map(|target| {
                let would_reassign = timeline.selected_ids.iter().any(|id| {
                    find_clip(project, id).is_some_and(|found| {
                        found.track_id != target.id && found.track.kind.same_family(target.kind)
                    })
                });
                MoveTargetView {
                    track_id: target.id.clone(),
                    name: target.name.clone(),
                    enabled: source.is_some()
                        && would_reassign
                        && todo_core::can_move_clips(project, &timeline.selected_ids, &target.id)
                            .is_ok(),
                }
            })
            .collect();
        timeline.active_track_id = self.timeline.active_track_id().map(str::to_owned);
        timeline.linked_selection = self.timeline.linked_selection();
        timeline.snapping = self.timeline.snapping();
        timeline.snap_target = self
            .timeline
            .snap_target
            .or_else(|| self.drop_preview.as_ref().and_then(|p| p.snap_target));
        timeline.drop_preview = self.drop_preview.clone();
        timeline.can_delete = !timeline.selected_ids.is_empty()
            && project
                .tracks
                .iter()
                .filter(|t| {
                    t.clips
                        .iter()
                        .any(|c| timeline.selected_ids.contains(&c.id))
                })
                .all(|t| !t.locked);
        for track in &mut timeline.tracks {
            for clip in &mut track.clips {
                clip.selected = timeline.selected_ids.contains(&clip.id);
            }
        }
        let properties = crate::inspector::project_properties(project, self.timeline.selected_id());
        let duration = project_duration(project);
        EditorViewModel {
            library: self.library.view(project),
            focus: self.focus,
            can_undo: editor.can_undo(),
            can_redo: editor.can_redo(),
            can_paste: editor.can_paste(),
            dirty: editor.dirty(),
            status: editor.status().map(status_text),
            project_name: editor.project_name().into(),
            clip_name: picture.name,
            current_time: editor.playhead(),
            duration,
            sequence_duration: project.duration,
            output_width: project.width,
            output_height: project.height,
            time_label: fmt_clock(editor.playhead()),
            duration_label: fmt_clock(duration),
            playing: editor.playing(),
            can_play: editor.can_play(),
            importing: editor.importing(),
            monitor_active: picture.active,
            poll_clock: editor.playing(),
            clock_epoch: editor.clock_epoch(),
            timeline,
            preview: self.preview_view(editor),
            properties,
        }
    }

    /// Project the selected visual layer (or topmost cover) after viewport/input
    /// changes, without rebuilding timeline capabilities each frame.
    pub fn preview_view(&self, editor: &EditorSession) -> PreviewView {
        let picture = editor.picture();
        let layers = resolve_layers(editor.project(), editor.playhead());
        // Prefer a selected layer so lower-track video stays a movable element.
        let focus = self
            .timeline
            .selected_id()
            .and_then(|id| layers.iter().find(|hit| hit.clip.id == id))
            .or_else(|| layers.last());
        let (clip_id, transform, aspect, editable, selection) = if let Some(hit) = focus {
            let locked =
                find_clip(editor.project(), &hit.clip.id).is_some_and(|found| found.track.locked);
            let selected = self.timeline.selected_ids().contains(&hit.clip.id);
            let selection = selected
                .then(|| {
                    let projection = self
                        .preview_viewport
                        .projection(self.preview_size, self.output_frame)?;
                    let canvas = projection.canvas_size(self.preview_size);
                    let base = self.element_base(editor, hit, canvas)?;
                    hit.clip
                        .transform
                        .bounds_for_base(base)
                        .and_then(|bounds| projection.project_bounds(bounds))
                })
                .flatten()
                .filter(|_| !self.preview_viewport.output_only());
            (
                Some(hit.clip.id.clone()),
                hit.clip.transform,
                hit.source_size,
                !locked,
                selection,
            )
        } else {
            (
                picture.clip_id.clone(),
                picture.transform,
                picture.aspect,
                false,
                None,
            )
        };
        let projection = self
            .preview_viewport
            .projection(self.preview_size, self.output_frame);
        let align_guides = self
            .preview_drag
            .as_ref()
            .map(|drag| {
                drag.guides
                    .iter()
                    .filter_map(|guide| {
                        let projection = projection?;
                        let position = match guide.axis {
                            AlignAxis::Vertical => projection.to_view((guide.position, 0.0)).0,
                            AlignAxis::Horizontal => projection.to_view((0.0, guide.position)).1,
                        };
                        position.is_finite().then_some(AlignGuideView {
                            vertical: matches!(guide.axis, AlignAxis::Vertical),
                            position,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        PreviewView {
            clip_id,
            transform,
            editable,
            aspect,
            selection,
            align_guides,
            viewport: self.preview_viewport,
            projection,
        }
    }

    pub fn asset_requests(&self, view: &EditorViewModel) -> Vec<AssetRequest> {
        let mut requests = Vec::new();
        for clip in view.timeline.tracks.iter().flat_map(|t| &t.clips) {
            if clip.audio && clip.has_audio && clip.wave_width > 0.0 {
                if let Some(path) = &clip.media_path {
                    requests.push(AssetRequest::Peaks {
                        path: path.clone(),
                        bins: 2048,
                    });
                }
            }
        }
        for entry in &view.library.items {
            if entry.item.visual {
                if let Some(path) = &entry.item.path {
                    requests.push(AssetRequest::Thumbnail {
                        path: path.clone(),
                        width: 160,
                    });
                }
            }
        }
        requests
    }
}
