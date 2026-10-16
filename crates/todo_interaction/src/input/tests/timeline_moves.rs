use super::*;
use todo_core::{find_clip, MediaKind};

fn movie() -> Project {
    let mut project = create_project(None, None, Some(20.0), None, None);
    project.media.push(MediaItem {
        id: "m1".into(),
        name: "movie.mp4".into(),
        path: None,
        duration: 30.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
});
    project
}

fn put(
    mut project: Project,
    track: &str,
    id: &str,
    start: f64,
    duration: f64,
    link: Option<&str>,
) -> Project {
    let mut clip = normalize_clip(id.into(), "m1".into(), start, duration, 0.0, 1.0);
    clip.link_id = link.map(str::to_string);
    project = add_clip(project, track, clip);
    project
}

fn begin(editor: &mut TestEditor, id: &str, start: f64, scroll: f64) -> f64 {
    let x = LABEL_W + start * 48.0 - scroll + 20.0;
    editor.apply(Intent::ClipPointerDown {
        clip_id: id.into(),
        view_x: x,
        scroll_left: scroll,
        local_x: 20.0,
        width_px: 96.0,
        alt: false,
    });
    x
}

#[test]
fn move_can_butt_either_edge_against_an_unquantized_media_boundary() {
    for (moving_start, moving_duration, target_start, target_duration, delta, expected, edge) in [
        (5.0, 1.117, 0.0, 2.013, -2.97, 2.013, 2.013),
        (0.0, 1.117, 3.003, 2.0, 1.87, 1.886, 3.003),
    ] {
        let base = put(
            put(movie(), "v1", "moving", moving_start, moving_duration, None),
            "v1",
            "target",
            target_start,
            target_duration,
            None,
        );
        let count = base.tracks.len();
        let mut editor = TestEditor::new(base, "test");
        let x = begin(&mut editor, "moving", moving_start, 0.0);
        editor.apply(Intent::PointerMove {
            view_x: x + delta * 48.0,
            scroll_left: 0.0,
            track_index: None,
        });
        assert_eq!(editor.view().timeline.snap_target, Some(edge));
        editor.apply(Intent::PointerUp);
        let moved = find_clip(editor.project(), "moving").unwrap();
        assert!(
            (moved.clip.start - expected).abs() < 1e-10,
            "expected {expected}, got {}",
            moved.clip.start
        );
        assert_eq!(moved.track_id, "v1");
        assert_eq!(editor.project().tracks.len(), count);
    }
}

fn pairs() -> Project {
    let mut project = movie();
    for (track, id, start, link) in [
        ("v1", "va", 0.0, "a"),
        ("a1", "aa", 0.0, "a"),
        ("v1", "vb", 6.0, "b"),
        ("a1", "ab", 6.0, "b"),
    ] {
        project = put(project, track, id, start, 4.0, Some(link));
    }
    project
}

#[test]
fn crossing_linked_pairs_is_continuous_and_one_undo_restores_tracks() {
    let base = pairs();
    let mut editor = TestEditor::new(base.clone(), "test");
    let x = begin(&mut editor, "vb", 6.0, 0.0);
    for delta in [-3.0, -4.0] {
        editor.apply(Intent::PointerMove {
            view_x: x + delta * 48.0,
            scroll_left: 0.0,
            track_index: Some(0),
        });
        assert_eq!(
            find_clip(editor.project(), "vb").unwrap().clip.start,
            6.0 + delta
        );
        assert_eq!(
            find_clip(editor.project(), "ab").unwrap().clip.start,
            6.0 + delta
        );
        assert_eq!(find_clip(editor.project(), "vb").unwrap().track_id, "v2");
        assert_eq!(find_clip(editor.project(), "ab").unwrap().track_id, "a2");
    }
    editor.apply(Intent::PointerUp);
    let moved = editor.project().clone();
    editor.apply(Intent::Undo);
    assert_eq!(editor.project(), &base);
    editor.apply(Intent::Redo);
    assert_eq!(editor.project(), &moved);
}

#[test]
fn explicitly_hovering_a_generated_lane_keeps_it_when_the_original_lane_becomes_free() {
    let base = pairs();
    let mut editor = TestEditor::new(base.clone(), "test");
    let x = begin(&mut editor, "vb", 6.0, 0.0);
    editor.apply(Intent::PointerMove {
        view_x: x - 4.0 * 48.0,
        scroll_left: 0.0,
        track_index: Some(0),
    });
    assert_eq!(find_clip(editor.project(), "vb").unwrap().track_id, "v2");
    let generated = editor
        .project()
        .tracks
        .iter()
        .position(|t| t.id == "v2")
        .unwrap();
    editor.apply(Intent::PointerMove {
        view_x: x,
        scroll_left: 0.0,
        track_index: Some(generated),
    });
    editor.apply(Intent::PointerUp);
    let found = find_clip(editor.project(), "vb").unwrap();
    assert_eq!(found.track_id, "v2");
    assert_eq!(found.clip.start, 6.0);
    assert_eq!(find_clip(editor.project(), "ab").unwrap().track_id, "a1");
    let moved = editor.project().clone();
    editor.apply(Intent::Undo);
    assert_eq!(editor.project(), &base);
    editor.apply(Intent::Redo);
    assert_eq!(editor.project(), &moved);
}

#[test]
fn escape_discards_generated_destination_tracks_with_the_linked_move() {
    let base = pairs();
    let mut editor = TestEditor::new(base.clone(), "test");
    let x = begin(&mut editor, "vb", 6.0, 0.0);
    editor.apply(Intent::PointerMove {
        view_x: x - 4.0 * 48.0,
        scroll_left: 0.0,
        track_index: Some(0),
    });
    let generated = editor
        .project()
        .tracks
        .iter()
        .position(|t| t.id == "v2")
        .unwrap();
    editor.apply(Intent::PointerMove {
        view_x: x,
        scroll_left: 0.0,
        track_index: Some(generated),
    });
    editor.apply(Intent::Key {
        code: KeyCode::Escape,
        typing: false,
    });
    assert_eq!(editor.project(), &base);
    assert!(editor.view().timeline.drag.is_none());
}

#[test]
fn new_video_lane_does_not_change_the_audio_drop_target_during_a_drag() {
    let mut editor = TestEditor::new(pairs(), "test");
    let x = begin(&mut editor, "ab", 6.0, 0.0);
    for delta in [-3.0, -4.0] {
        let audio_index = editor
            .project()
            .tracks
            .iter()
            .position(|t| t.id == "a1")
            .unwrap();
        editor.apply(Intent::PointerMove {
            view_x: x + delta * 48.0,
            scroll_left: 0.0,
            track_index: Some(audio_index),
        });
        assert_eq!(find_clip(editor.project(), "ab").unwrap().track_id, "a2");
        assert_eq!(
            find_clip(editor.project(), "ab").unwrap().clip.start,
            6.0 + delta
        );
    }
}

#[test]
fn dragging_an_already_selected_linked_video_uses_that_video_as_primary() {
    let base = todo_core::add_track(pairs(), TrackKind::Video);
    let mut editor = TestEditor::new(base, "test");
    editor.apply(Intent::SelectClip {
        clip_id: "ab".into(),
        additive: false,
    });
    let x = begin(&mut editor, "vb", 6.0, 0.0);
    editor.apply(Intent::PointerMove {
        view_x: x,
        scroll_left: 0.0,
        track_index: Some(1),
    });
    editor.apply(Intent::PointerUp);
    assert_eq!(find_clip(editor.project(), "vb").unwrap().track_id, "v2");
    assert_eq!(find_clip(editor.project(), "ab").unwrap().track_id, "a1");
}

#[test]
fn moving_while_scrolling_preserves_the_pointer_anchor_in_timeline_space() {
    let base = put(movie(), "v1", "moving", 5.0, 2.0, None);
    let mut editor = TestEditor::new(base, "test");
    editor.apply(Intent::ToggleSnapping);
    let x = begin(&mut editor, "moving", 5.0, 48.0);
    editor.apply(Intent::PointerMove {
        view_x: x,
        scroll_left: 96.0,
        track_index: None,
    });
    editor.apply(Intent::PointerUp);
    assert_eq!(
        find_clip(editor.project(), "moving").unwrap().clip.start,
        6.0
    );
}
