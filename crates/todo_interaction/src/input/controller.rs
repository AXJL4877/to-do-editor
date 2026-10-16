use super::*;
use crate::timeline::{DragKind, TimelineOutcome};
use todo_application::{AppAction, AppUpdate, EditorCommand, Workspace};
use todo_core::{place_duration, EditCommand, DEFAULT_OUTPUT_HEIGHT, DEFAULT_OUTPUT_WIDTH};

pub struct EditorController {
    pub(crate) timeline: TimelineController,
    pub(crate) library: LibraryState,
    pub(crate) focus: EditorFocus,
    pub(crate) drop_preview: Option<DropPreview>,
    pub(super) preview_drag: Option<PreviewDrag>,
    pub(super) preview_size: (f64, f64),
    /// Cached program-frame aspect units from the open project.
    pub(super) output_frame: (f64, f64),
    pub(super) preview_viewport: crate::PreviewViewport,
    pub(super) preview_pan: Option<(f64, f64)>,
    pub(super) preview_source_aspect: Option<(String, f64)>,
}
impl Default for EditorController {
    fn default() -> Self {
        Self::new()
    }
}
impl EditorController {
    pub fn new() -> Self {
        Self {
            timeline: TimelineController::new(),
            library: LibraryState::default(),
            focus: EditorFocus::Timeline,
            drop_preview: None,
            preview_drag: None,
            preview_size: (640.0, 360.0),
            output_frame: (
                f64::from(DEFAULT_OUTPUT_WIDTH),
                f64::from(DEFAULT_OUTPUT_HEIGHT),
            ),
            preview_viewport: crate::PreviewViewport::default(),
            preview_pan: None,
            preview_source_aspect: None,
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new();
    }
    pub fn sync_output_frame(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.output_frame = (f64::from(width), f64::from(height));
        }
    }
    pub fn reconcile(&mut self, workspace: &Workspace) {
        if let Some(editor) = workspace.editor() {
            let project = editor.project();
            self.timeline.reconcile(project);
            self.sync_output_frame(project.width, project.height);
        }
    }
    pub fn select_inserted(&mut self, workspace: &Workspace, ids: &[String]) {
        if let (Some(editor), Some(id)) = (workspace.editor(), ids.first()) {
            self.timeline.select_clip(editor.project(), id, false);
            self.focus = EditorFocus::Timeline;
        }
    }
    pub fn apply(&mut self, workspace: &mut Workspace, intent: Intent) -> AppUpdate {
        if self.preview_navigation(&intent) {
            return AppUpdate::default();
        }
        if let Intent::PreviewPointer { kind, x, y } = intent {
            return self.preview_pointer(workspace, kind, x, y);
        }
        if let Intent::PreviewViewport { width, height } = intent {
            if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
                self.preview_size = (width, height);
            }
            return AppUpdate::default();
        }
        if let Intent::PreviewSourceAspect { path, aspect } = intent {
            if aspect.is_finite()
                && aspect > 0.0
                && workspace.editor().is_some_and(|editor| {
                    editor.picture().path.as_deref() == Some(path.as_str())
                        || editor.shown_layers().iter().any(|layer| layer.path == path)
                })
            {
                self.preview_source_aspect = Some((path, aspect));
            }
            return AppUpdate::default();
        }
        let end_gesture = matches!(
            &intent,
            Intent::DeleteSelection { .. }
                | Intent::SelectClip { .. }
                | Intent::SelectAll
                | Intent::ClearSelection
                | Intent::AddTrack { .. }
                | Intent::InsertTrackBelow { .. }
                | Intent::MoveTrack { .. }
                | Intent::ToggleLock { .. }
                | Intent::PlaceMedia { .. }
                | Intent::AddText
                | Intent::RemoveMedia { .. }
                | Intent::MoveSelectionToTrack { .. }
                | Intent::Paste
                | Intent::Save
                | Intent::Back
                | Intent::RequestClose
                | Intent::RazorAtPlayhead
                | Intent::DeleteTrack { .. }
                | Intent::ToggleMute { .. }
                | Intent::ImportPressed
        );
        if end_gesture {
            let _ = workspace.dispatch(AppAction::Editor(EditorCommand::CommitEdit));
            self.timeline.cancel_drag();
            self.preview_drag = None;
            self.preview_pan = None;
        }
        let update = match intent {
            Intent::Focus(focus) => {
                self.focus = focus;
                AppUpdate::default()
            }
            Intent::LibrarySelect(id) => {
                self.library.selected = Some(id);
                self.focus = EditorFocus::Library;
                AppUpdate::default()
            }
            Intent::LibraryQuery(query) => {
                self.library.query = query;
                AppUpdate::default()
            }
            Intent::DeleteSelection { ripple } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::DeleteClips {
                    clip_ids: self.timeline.selected_ids().to_vec(),
                    ripple,
                }),
            )),
            Intent::AddTrack { kind } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::AddTrack { kind }),
            )),
            Intent::InsertTrackBelow { track_id } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::InsertTrackBelow { track_id }),
            )),
            Intent::MoveTrack { track_id, up } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::MoveTrack { track_id, up }),
            )),
            Intent::ToggleLock { track_id } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::ToggleLock { track_id }),
            )),
            Intent::MoveSelectionToTrack { track_id } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::MoveClips {
                    clip_ids: self.timeline.selected_ids().to_vec(),
                    delta: 0.0,
                    track_id: Some(track_id),
                }),
            )),
            Intent::PlaceMedia {
                media_id,
                track_id,
                time,
            } => {
                let Some(editor) = workspace.editor() else {
                    return AppUpdate::default();
                };
                let time = time
                    .map(|time| self.snapped_placement(editor, &media_id, time).0)
                    .unwrap_or_else(|| todo_core::content_duration(editor.project()));
                self.drop_preview = None;
                let mut update = workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
                    EditCommand::PlaceMedia {
                        media_id,
                        track_id,
                        time,
                    },
                )));
                if !update.new_clips.is_empty() {
                    update.effects.extend(
                        workspace
                            .dispatch(AppAction::Editor(EditorCommand::Seek(time)))
                            .effects,
                    );
                }
                update
            }
            Intent::RemoveMedia { media_id, cascade } => {
                workspace.dispatch(AppAction::Editor(EditorCommand::RemoveMedia {
                    media_id,
                    cascade,
                }))
            }
            Intent::MediaDragHover {
                media_id,
                track_id,
                time,
            } => {
                if let Some(editor) = workspace.editor() {
                    let (start, target) = self.snapped_placement(editor, &media_id, time);
                    let duration = editor
                        .project()
                        .media
                        .iter()
                        .find(|m| m.id == media_id)
                        .map(place_duration)
                        .unwrap_or(0.0);
                    self.drop_preview = Some(DropPreview {
                        allowed: todo_core::can_place_media(
                            editor.project(),
                            &media_id,
                            Some(&track_id),
                            start,
                        )
                        .is_ok(),
                        media_id,
                        track_id,
                        start,
                        duration,
                        snap_target: target,
                    });
                }
                AppUpdate::default()
            }
            Intent::MediaDragEnd => {
                self.drop_preview = None;
                AppUpdate::default()
            }
            Intent::Undo => {
                self.timeline.cancel_drag();
                self.preview_drag = None;
                workspace.dispatch(AppAction::Editor(EditorCommand::Undo))
            }
            Intent::Redo => workspace.dispatch(AppAction::Editor(EditorCommand::Redo)),
            Intent::Copy => workspace.dispatch(AppAction::Editor(EditorCommand::Copy {
                clip_ids: self.timeline.selected_ids().to_vec(),
            })),
            Intent::Paste => workspace.dispatch(AppAction::Editor(EditorCommand::Paste)),
            Intent::TogglePlay => workspace.dispatch(AppAction::Editor(EditorCommand::TogglePlay)),
            Intent::Seek(time) => workspace.dispatch(AppAction::Editor(EditorCommand::Seek(time))),
            Intent::BeginEdit | Intent::CommitEdit
                if self.preview_drag.is_some() || self.timeline.drag().is_some() =>
            {
                AppUpdate::default()
            }
            Intent::BeginEdit => workspace.dispatch(AppAction::Editor(EditorCommand::BeginEdit)),
            Intent::CommitEdit => workspace.dispatch(AppAction::Editor(EditorCommand::CommitEdit)),
            Intent::SetClipDuration { clip_id, seconds } => {
                self.property_duration(workspace, clip_id, seconds)
            }
            Intent::SetClipTransform { clip_id, transform } => {
                self.property_transform(workspace, clip_id, transform)
            }
            Intent::ResetTransform { clip_id } => {
                self.property_transform(workspace, clip_id, ClipTransform::default())
            }
            Intent::AddText => {
                let time = workspace.editor().map_or(0.0, |editor| editor.playhead());
                workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
                    EditCommand::AddText {
                        track_id: None,
                        time,
                        duration: todo_core::DEFAULT_TEXT_DURATION_SECS,
                        content: todo_core::TextContent::new(
                            "新文字",
                            todo_core::DEFAULT_TEXT_STYLE_ID,
                        ),
                    },
                )))
            }
            Intent::SetClipText { clip_id, content } => {
                self.property_text(workspace, clip_id, content)
            }
            Intent::SetProjectDuration { seconds } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::SetDuration { seconds }),
            )),
            Intent::SetProjectOutput { width, height } => {
                let update = workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
                    EditCommand::SetOutputSize { width, height },
                )));
                if let Some(editor) = workspace.editor() {
                    self.sync_output_frame(editor.project().width, editor.project().height);
                }
                update
            }
            Intent::SetProjectFps { fps } => workspace.dispatch(AppAction::Editor(
                EditorCommand::Edit(EditCommand::SetFps { fps }),
            )),
            Intent::ImportPressed => workspace.dispatch(AppAction::ChooseImport),
            Intent::Save => workspace.dispatch(AppAction::Save),
            Intent::Back => workspace.dispatch(AppAction::Back),
            Intent::RequestClose => workspace.dispatch(AppAction::RequestClose),
            Intent::RefreshProjects => workspace.dispatch(AppAction::RefreshProjects),
            Intent::CreateProject { name } => workspace.dispatch(AppAction::CreateProject(name)),
            Intent::OpenProject { name } => workspace.dispatch(AppAction::OpenProject(name)),
            Intent::DeleteProject { name } => workspace.dispatch(AppAction::DeleteProject(name)),
            Intent::ChooseProjectFile => workspace.dispatch(AppAction::ChooseProjectFile),
            Intent::ConfirmClose(choice) => workspace.dispatch(AppAction::ConfirmClose(choice)),
            other @ (Intent::SelectClip { .. }
            | Intent::SelectAll
            | Intent::ClearSelection
            | Intent::ToggleLinkedSelection
            | Intent::ToggleSnapping
            | Intent::BoardPointerDown { .. }
            | Intent::LanePointerDown { .. }
            | Intent::MarqueePointerDown { .. }
            | Intent::ClipPointerDown { .. }
            | Intent::PointerMove { .. }
            | Intent::PointerUp
            | Intent::ToolClicked(_)
            | Intent::RazorAtPlayhead
            | Intent::ToggleMute { .. }
            | Intent::DeleteTrack { .. }
            | Intent::Key { .. }
            | Intent::Wheel { .. }
            | Intent::Viewport { .. }) => self.timeline_input(workspace, other),
            Intent::PreviewPointer { .. }
            | Intent::PreviewViewport { .. }
            | Intent::PreviewSourceAspect { .. }
            | Intent::PreviewZoom(_)
            | Intent::PreviewWheel { .. }
            | Intent::PreviewPan { .. }
            | Intent::PreviewFit
            | Intent::PreviewOutputOnly(_) => {
                unreachable!("handled above")
            }
        };
        self.reconcile(workspace);
        self.select_inserted(workspace, &update.new_clips);
        update
    }
    fn snapped_placement(
        &self,
        editor: &todo_application::EditorSession,
        id: &str,
        time: f64,
    ) -> (f64, Option<f64>) {
        let duration = editor
            .project()
            .media
            .iter()
            .find(|m| m.id == id)
            .map(place_duration)
            .unwrap_or(0.0);
        let hit = crate::timeline::snapping::snap_position(
            editor.project(),
            time,
            duration,
            &[],
            editor.playhead(),
            self.timeline.px_per_sec(editor.project()),
            self.timeline.snapping(),
        );
        (hit.value, hit.target)
    }
    fn timeline_input(&mut self, workspace: &mut Workspace, intent: Intent) -> AppUpdate {
        if workspace.editor().is_none() {
            return AppUpdate::default();
        }
        let was_scrub = self
            .timeline
            .drag()
            .is_some_and(|d| d.kind() == DragKind::Seek);
        let release = matches!(&intent, Intent::PointerUp);
        let escape = matches!(
            &intent,
            Intent::Key {
                code: KeyCode::Escape,
                typing: false
            }
        );
        if matches!(&intent, Intent::ClipPointerDown { .. })
            && self.timeline.tool() == TimelineTool::Select
        {
            let _ = workspace.dispatch(AppAction::Editor(EditorCommand::CommitEdit));
            let _ = workspace.dispatch(AppAction::Editor(EditorCommand::BeginEdit));
        }
        let editor = workspace.editor().expect("checked editor");
        let outcome = self
            .timeline
            .handle(editor.project(), editor.playhead(), &intent);
        let mut update = match outcome {
            TimelineOutcome::None | TimelineOutcome::Zoom(_) => AppUpdate::default(),
            TimelineOutcome::Scrub(time) => {
                workspace.dispatch(AppAction::Editor(EditorCommand::Seek(time)))
            }
            TimelineOutcome::Edit(command) => {
                workspace.dispatch(AppAction::Editor(EditorCommand::Edit(command)))
            }
        };
        if release {
            update.effects.extend(
                workspace
                    .dispatch(AppAction::Editor(EditorCommand::CommitEdit))
                    .effects,
            );
        }
        if release && was_scrub {
            update.effects.extend(
                workspace
                    .dispatch(AppAction::Editor(EditorCommand::RefreshPicture))
                    .effects,
            );
        }
        if escape {
            self.preview_drag = None;
            self.preview_pan = None;
            self.drop_preview = None;
            update.effects.extend(
                workspace
                    .dispatch(AppAction::Editor(EditorCommand::CancelEdit))
                    .effects,
            );
        }
        if matches!(
            &intent,
            Intent::BoardPointerDown { .. }
                | Intent::LanePointerDown { .. }
                | Intent::MarqueePointerDown { .. }
                | Intent::ClipPointerDown { .. }
                | Intent::SelectClip { .. }
                | Intent::SelectAll
        ) {
            self.focus = EditorFocus::Timeline;
        }
        update
    }
}
