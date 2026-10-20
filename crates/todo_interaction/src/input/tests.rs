use super::*;
use crate::input::{Intent, TimelineTool};
use crate::timeline::LABEL_W;
use todo_application::{AppAction, AppUpdate, EditorCommand, Effect, PreviewCommand, Workspace};
use todo_core::{
    add_clip, create_project, normalize_clip, project_duration, MediaItem, Project, TrackKind,
};

mod gestures;
mod library_folders;
mod text;
mod timeline_moves;
mod transform;
mod viewport;
mod workspace_time;

/// Exercise production input and application boundaries together, without a UI toolkit.
struct TestEditor {
    workspace: Workspace,
    controller: EditorController,
}

struct TestUpdate {
    commands: Vec<PreviewCommand>,
}

impl TestUpdate {
    fn from_application(update: AppUpdate) -> Self {
        Self {
            commands: update
                .effects
                .into_iter()
                .filter_map(|effect| match effect {
                    Effect::Preview(commands) => Some(commands),
                    _ => None,
                })
                .flatten()
                .collect(),
        }
    }
}

impl TestEditor {
    fn new(project: Project, name: &str) -> Self {
        let mut controller = EditorController::new();
        controller.sync_output_frame(project.width, project.height);
        Self {
            workspace: Workspace::with_project(project, name),
            controller,
        }
    }
    fn apply(&mut self, intent: Intent) -> TestUpdate {
        TestUpdate::from_application(self.controller.apply(&mut self.workspace, intent))
    }
    fn command(&mut self, command: EditorCommand) -> TestUpdate {
        let update = self.workspace.dispatch(AppAction::Editor(command));
        self.controller.reconcile(&self.workspace);
        self.controller
            .select_inserted(&self.workspace, &update.new_clips);
        TestUpdate::from_application(update)
    }
    fn project(&self) -> &Project {
        self.workspace.editor().unwrap().project()
    }
    fn view(&self) -> crate::EditorViewModel {
        self.controller.view(self.workspace.editor().unwrap(), &[])
    }
    fn playing(&self) -> bool {
        self.workspace.editor().unwrap().playing()
    }
    fn playhead(&self) -> f64 {
        self.workspace.editor().unwrap().playhead()
    }
    fn sync(&mut self) {
        self.workspace.editor_mut().unwrap().sync();
    }
}

fn with_clip() -> Project {
    let mut project = create_project(None, None, Some(4.0), None, None);
    project.media.push(MediaItem {
        id: "m1".into(),
        name: "clip.mp4".into(),
        path: Some("data/a.mp4".into()),
        duration: 4.0,
        has_audio: Some(true),
        width: Some(16),
        height: Some(9),
        kind: None,
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    project = add_clip(
        project,
        "v1",
        normalize_clip("c1".into(), "m1".into(), 0.0, 4.0, 0.0, 1.0),
    );
    add_clip(
        project,
        "a1",
        normalize_clip("a1c".into(), "m1".into(), 0.0, 4.0, 0.0, 1.0),
    )
}

#[test]
fn playback_stops_at_end_and_seek_stops_play() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    assert!(editor
        .apply(Intent::TogglePlay)
        .commands
        .iter()
        .any(|cmd| matches!(cmd, PreviewCommand::Transport { playing: true, .. })));
    assert!(editor.playing());
    editor.command(EditorCommand::Frame { now: 0.0 });
    editor.command(EditorCommand::Frame { now: 4.5 });
    assert!(!editor.playing());
    assert!((editor.playhead() - 4.0).abs() < 1e-3);
    editor.apply(Intent::TogglePlay);
    editor.apply(Intent::Seek(1.0));
    assert!(!editor.playing());
    assert!((editor.playhead() - 1.0).abs() < 1e-6);
}

#[test]
fn import_places_media_and_keeps_av_tracks() {
    let mut editor = TestEditor::new(create_project(None, None, None, None, None), "demo");
    let media = with_clip().media.remove(0);
    editor.command(EditorCommand::Imported(media));
    assert_eq!(editor.project().media.len(), 1);
    assert!(editor
        .project()
        .tracks
        .iter()
        .any(|track| track.kind == TrackKind::Video && !track.clips.is_empty()));
    assert!(editor
        .project()
        .tracks
        .iter()
        .any(|track| track.kind == TrackKind::Audio && !track.clips.is_empty()));
    assert_eq!(editor.view().library.items.len(), 1);
}

#[test]
fn ctrl_marquee_selects_multiple_clips_for_move_and_delete() {
    let mut project = create_project(None, None, Some(20.0), None, None);
    project.media.push(MediaItem {
        id: "m1".into(),
        name: "a.mp4".into(),
        path: None,
        duration: 4.0,
        has_audio: Some(false),
        width: Some(16),
        height: Some(9),
        kind: None,
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    project = add_clip(
        project,
        "v1",
        normalize_clip("c1".into(), "m1".into(), 0.0, 2.0, 0.0, 1.0),
    );
    project = add_clip(
        project,
        "v1",
        normalize_clip("c2".into(), "m1".into(), 3.0, 2.0, 0.0, 1.0),
    );
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::ToggleLinkedSelection); // off — select only video hits
    editor.apply(Intent::MarqueePointerDown {
        view_x: LABEL_W + 24.0, // ~0.5s at 48px/s
        track_index: 0,
        scroll_left: 0.0,
        additive: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 192.0, // ~4s
        scroll_left: 0.0,
        track_index: Some(0),
    });
    editor.apply(Intent::PointerUp);
    let selected = editor.view().timeline.selected_ids;
    assert!(selected.contains(&"c1".into()), "{selected:?}");
    assert!(selected.contains(&"c2".into()), "{selected:?}");

    let start_c1 = todo_core::find_clip(editor.project(), "c1")
        .unwrap()
        .clip
        .start;
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: LABEL_W + 40.0,
        scroll_left: 0.0,
        local_x: 40.0,
        width_px: 96.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 88.0,
        scroll_left: 0.0,
        track_index: None,
    });
    editor.apply(Intent::PointerUp);
    let moved = todo_core::find_clip(editor.project(), "c1")
        .unwrap()
        .clip
        .start;
    let moved2 = todo_core::find_clip(editor.project(), "c2")
        .unwrap()
        .clip
        .start;
    assert!((moved - (start_c1 + 1.0)).abs() < 1e-6);
    assert!((moved2 - 4.0).abs() < 1e-6);

    editor.apply(Intent::DeleteSelection { ripple: false });
    assert!(todo_core::find_clip(editor.project(), "c1").is_none());
    assert!(todo_core::find_clip(editor.project(), "c2").is_none());
}

#[test]
fn set_project_duration_sets_workspace_axis_ceiling() {
    let mut editor = TestEditor::new(create_project(None, None, None, None, None), "demo");
    editor.apply(Intent::SetProjectDuration { seconds: 120.0 });
    assert_eq!(editor.project().duration, Some(120.0));
    assert!((editor.view().duration - 120.0).abs() < 1e-9);
    assert_eq!(editor.view().sequence_duration, Some(120.0));
    let axis = editor.view().timeline.lane_width / editor.view().timeline.px_per_sec;
    assert!(
        (axis - 120.0).abs() < 0.01,
        "timeline axis should equal workspace seconds, got {axis}"
    );
}

#[test]
fn set_project_output_and_fps_update_view_and_history() {
    let mut editor = TestEditor::new(create_project(None, None, None, None, None), "demo");
    assert_eq!(editor.view().output_width, 1920);
    assert_eq!(editor.view().output_height, 1080);
    editor.apply(Intent::SetProjectOutput {
        width: 1080,
        height: 1920,
    });
    assert_eq!(editor.project().output_size(), (1080, 1920));
    assert_eq!(editor.view().output_width, 1080);
    assert_eq!(editor.view().output_height, 1920);
    editor.apply(Intent::SetProjectFps { fps: 24.0 });
    assert_eq!(editor.project().fps, 24.0);
    assert!((editor.view().timeline.fps - 24.0).abs() < 1e-9);
    editor.apply(Intent::Undo);
    assert_eq!(editor.project().fps, 30.0);
    editor.apply(Intent::Undo);
    assert_eq!(editor.project().output_size(), (1920, 1080));
}

#[test]
fn workspace_axis_fills_board_viewport_at_default_zoom() {
    let mut editor = TestEditor::new(create_project(None, None, None, None, None), "demo");
    editor.apply(Intent::SetProjectDuration { seconds: 56.0 });
    let board_w = LABEL_W + 900.0;
    editor.apply(Intent::Viewport {
        scroll_left: 0.0,
        width: board_w,
    });
    let tl = editor.view().timeline;
    assert!(
        (tl.board_width - board_w).abs() < 1e-6,
        "board must fill viewport, got {} vs {}",
        tl.board_width,
        board_w
    );
    assert!((tl.lane_width - 900.0).abs() < 1e-6);
    assert!((tl.px_per_sec - 900.0 / 56.0).abs() < 1e-9);
}

#[test]
fn delete_selection_removes_clips_and_can_be_undone() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    editor.apply(Intent::DeleteSelection { ripple: false });
    assert!(todo_core::find_clip(editor.project(), "c1").is_none());
    assert_eq!(editor.project().media.len(), 1);
    assert!(editor.view().can_undo);
    editor.apply(Intent::Undo);
    assert!(todo_core::find_clip(editor.project(), "c1").is_some());
    editor.apply(Intent::Redo);
    assert!(todo_core::find_clip(editor.project(), "c1").is_none());
}

#[test]
fn one_drag_is_one_history_entry_and_escape_restores_origin() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: LABEL_W + 40.0,
        scroll_left: 0.0,
        local_x: 40.0,
        width_px: 192.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 88.0,
        scroll_left: 0.0,
        track_index: None,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 136.0,
        scroll_left: 0.0,
        track_index: None,
    });
    editor.apply(Intent::PointerUp);
    assert_eq!(
        todo_core::find_clip(editor.project(), "c1")
            .unwrap()
            .clip
            .start,
        2.0
    );
    editor.apply(Intent::Undo);
    assert_eq!(
        todo_core::find_clip(editor.project(), "c1")
            .unwrap()
            .clip
            .start,
        0.0
    );
    assert!(!editor.view().can_undo);
}

fn with_placed_pair() -> TestEditor {
    let mut editor = TestEditor::new(create_project(None, None, None, None, None), "demo");
    // Import now registers media and appends linked picture/audio clips.
    editor.command(EditorCommand::Imported(with_clip().media.remove(0)));
    editor
}

#[test]
fn deleting_linked_media_keeps_library_and_minimum_tracks() {
    let mut editor = with_placed_pair();
    let id = editor
        .project()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .clips[0]
        .id
        .clone();
    editor.apply(Intent::SelectClip {
        clip_id: id,
        additive: false,
    });
    assert_eq!(editor.view().timeline.selected_ids.len(), 2);
    editor.apply(Intent::DeleteSelection { ripple: false });
    assert!(editor.project().tracks.iter().all(|t| t.clips.is_empty()));
    assert_eq!(editor.project().tracks.len(), 2);
    assert_eq!(editor.project().media.len(), 1);
    assert!(project_duration(editor.project()) >= 1.0);
    editor.apply(Intent::Undo);
    assert_eq!(
        editor
            .project()
            .tracks
            .iter()
            .map(|t| t.clips.len())
            .sum::<usize>(),
        2
    );
}

#[test]
fn trim_keeps_linked_audio_video_synchronized() {
    let mut editor = with_placed_pair();
    let id = editor
        .project()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .clips[0]
        .id
        .clone();
    editor.apply(Intent::ClipPointerDown {
        clip_id: id,
        view_x: LABEL_W + 192.0,
        scroll_left: 0.0,
        local_x: 190.0,
        width_px: 192.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 96.0,
        scroll_left: 0.0,
        track_index: None,
    });
    editor.apply(Intent::PointerUp);
    for clip in editor.project().tracks.iter().flat_map(|t| &t.clips) {
        assert_eq!(clip.duration, 2.0);
    }
}

#[test]
fn locked_linked_partner_blocks_trim_atomically() {
    let mut editor = with_placed_pair();
    let audio = editor
        .project()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio && !t.clips.is_empty())
        .unwrap()
        .id
        .clone();
    editor.apply(Intent::ToggleLock { track_id: audio });
    let id = editor
        .project()
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .clips[0]
        .id
        .clone();
    editor.apply(Intent::ClipPointerDown {
        clip_id: id,
        view_x: LABEL_W + 192.0,
        scroll_left: 0.0,
        local_x: 190.0,
        width_px: 192.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 96.0,
        scroll_left: 0.0,
        track_index: None,
    });
    editor.apply(Intent::PointerUp);
    for clip in editor.project().tracks.iter().flat_map(|t| &t.clips) {
        assert_eq!(clip.duration, 4.0);
    }
}

#[test]
fn copy_paste_has_new_ids_and_preserves_pair_linking() {
    let mut editor = with_placed_pair();
    let before: Vec<_> = editor
        .project()
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .cloned()
        .collect();
    editor.apply(Intent::SelectClip {
        clip_id: before[0].id.clone(),
        additive: false,
    });
    editor.apply(Intent::Copy);
    editor.apply(Intent::Seek(4.0));
    editor.apply(Intent::Paste);
    let pasted: Vec<_> = editor
        .project()
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter(|c| c.start == 4.0)
        .collect();
    assert_eq!(pasted.len(), 2);
    assert_eq!(pasted[0].link_id, pasted[1].link_id);
    assert_ne!(pasted[0].link_id, before[0].link_id);
    assert!(pasted
        .iter()
        .all(|c| before.iter().all(|old| old.id != c.id)));
}

#[test]
fn late_save_acknowledgment_does_not_clear_newer_edits() {
    let mut editor = with_placed_pair();
    let saved = editor.project().clone();
    editor.apply(Intent::AddTrack {
        kind: TrackKind::Video,
    });
    editor.command(EditorCommand::Saved(saved));
    assert!(editor.view().dirty);
}

#[test]
fn releasing_playhead_resubmits_the_final_frame() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.sync();
    editor.apply(Intent::BoardPointerDown {
        view_x: LABEL_W + 48.0,
        scroll_left: 0.0,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 120.0,
        scroll_left: 0.0,
        track_index: None,
    });
    let result = editor.apply(Intent::PointerUp);
    assert!(result.commands.iter().any(|command| matches!(
        command,
        PreviewCommand::Layers(layers)
            if layers.len() == 1 && (layers[0].media_time - 2.5).abs() < 1e-9
    )));
    assert!(!editor.playing());
    assert!(editor.controller.timeline.drag().is_none());
    assert!(editor.apply(Intent::PointerUp).commands.is_empty());
}

#[test]
fn releasing_a_clip_drag_does_not_resubmit_the_playhead() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.sync();
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: LABEL_W + 40.0,
        scroll_left: 0.0,
        local_x: 40.0,
        width_px: 192.0,
        alt: false,
    });
    assert!(editor.apply(Intent::PointerUp).commands.is_empty());
}

#[test]
fn razor_splits_one_track_and_keeps_playing() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::TogglePlay);
    editor.apply(Intent::ToolClicked(TimelineTool::Razor));
    let playhead_before = editor.project().playhead;
    editor.apply(Intent::LanePointerDown {
        track_id: "v1".into(),
        view_x: LABEL_W + 96.0,
        scroll_left: 0.0,
    });
    assert!(editor.playing());
    assert!(
        (editor.project().playhead - playhead_before).abs() < 1e-9,
        "razor cut must not seek the playhead"
    );
    assert_eq!(
        editor
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap()
            .clips
            .len(),
        2
    );
    assert_eq!(
        editor
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "a1")
            .unwrap()
            .clips
            .len(),
        1
    );
}

#[test]
fn razor_ruler_scrubs_without_cutting() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::ToolClicked(TimelineTool::Razor));
    let v1_before = editor
        .project()
        .tracks
        .iter()
        .find(|t| t.id == "v1")
        .unwrap()
        .clips
        .len();
    editor.apply(Intent::BoardPointerDown {
        view_x: LABEL_W + 96.0,
        scroll_left: 0.0,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 160.0,
        scroll_left: 0.0,
        track_index: None,
    });
    editor.apply(Intent::PointerUp);
    assert!(
        (editor.project().playhead - 160.0 / 48.0).abs() < 0.05 || editor.project().playhead > 1.0,
        "ruler scrub should move the playhead, got {}",
        editor.project().playhead
    );
    assert_eq!(
        editor
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap()
            .clips
            .len(),
        v1_before,
        "ruler scrub must not razor clips"
    );
}

#[test]
fn move_drag_shifts_start() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: LABEL_W + 40.0,
        scroll_left: 0.0,
        local_x: 40.0,
        width_px: 192.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 40.0 + 48.0,
        scroll_left: 0.0,
        track_index: None,
    });
    let clip = todo_core::find_clip(editor.project(), "c1").unwrap().clip;
    assert!((clip.start - 1.0).abs() < 1e-6, "start {}", clip.start);
}
