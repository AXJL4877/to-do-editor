use todo_application::{EditorCommand, EditorSession, PreviewCommand};
use todo_core::{create_project, place_media, EditCommand, MediaItem, MediaKind, TrackKind};

fn editor() -> EditorSession {
    let mut project = create_project(None, None, Some(60.0), None, None);
    project.media.push(MediaItem {
        id: "video".into(),
        name: "video.mp4".into(),
        path: Some("video.mp4".into()),
        duration: 10.0,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    EditorSession::new(
        place_media(project, "video", Some("v1"), 0.0).unwrap(),
        "test",
    )
}

#[test]
fn repeated_scrubs_keep_project_storage_in_place() {
    let mut editor = editor();
    let media = editor.project().media.as_ptr();
    let tracks = editor.project().tracks.as_ptr();
    let path = editor.project().media[0].path.as_ref().unwrap().as_ptr();
    for time in [1.0, 2.0, 0.5, 9.0, 0.0].into_iter().cycle().take(100) {
        let _ = editor.apply(EditorCommand::Seek(time));
        assert_eq!(
            editor.project().media.as_ptr(),
            media,
            "seek copied media storage"
        );
        assert_eq!(
            editor.project().tracks.as_ptr(),
            tracks,
            "seek copied track storage"
        );
        assert_eq!(
            editor.project().media[0].path.as_ref().unwrap().as_ptr(),
            path
        );
    }
}

#[test]
fn scrub_clamps_without_dirtying_content_or_changing_edit_history() {
    let mut editor = editor();
    for (time, expected) in [
        (-1.0, 0.0),
        (f64::EPSILON / 2.0, 0.0),
        (1.234_567, 1.234_567),
        (f64::INFINITY, 60.0),
        (f64::NEG_INFINITY, 0.0),
        (f64::NAN, 0.0),
    ] {
        let update = editor.apply(EditorCommand::Seek(time));
        assert_eq!(editor.playhead(), expected);
        assert_eq!(editor.project().playhead, expected);
        assert!(!update.content_changed);
        assert_eq!(editor.revision(), 0);
        assert!(!editor.dirty());
        assert!(!editor.can_undo());
        assert!(!editor.can_redo());
    }
    let update = editor.apply(EditorCommand::Seek(1.234_567));
    assert!(update.commands.iter().any(|command| matches!(
        command,
        PreviewCommand::Transport { time, playing: false } if *time == 1.234_567
    )));
    assert!(update.commands.iter().any(|command| matches!(
        command,
        PreviewCommand::Layers(layers) if layers[0].media_time == 1.234_567
    )));
    let _ = editor.apply(EditorCommand::Edit(EditCommand::AddTrack {
        kind: TrackKind::Video,
    }));
    let revision = editor.revision();
    let _ = editor.apply(EditorCommand::Seek(2.0));
    assert!(editor.dirty());
    assert!(editor.can_undo());
    assert_eq!(editor.revision(), revision);
    let _ = editor.apply(EditorCommand::Undo);
    assert!(!editor.dirty());
    assert!(!editor.can_undo());
    assert!(editor.can_redo());
    let _ = editor.apply(EditorCommand::Seek(3.0));
    assert!(editor.can_redo());
}

#[test]
fn scrub_synchronizes_the_playhead_before_later_edits() {
    let mut editor = editor();
    let _ = editor.apply(EditorCommand::TogglePlay);
    let _ = editor.apply(EditorCommand::Frame { now: 1.0 });
    let _ = editor.apply(EditorCommand::Frame { now: 2.0 });
    assert_eq!(editor.playhead(), 1.0);
    let _ = editor.apply(EditorCommand::Seek(2.5));
    assert!(!editor.playing());
    let _ = editor.apply(EditorCommand::Edit(EditCommand::AddTrack {
        kind: TrackKind::Video,
    }));
    assert_eq!(editor.playhead(), 2.5);
    assert_eq!(editor.project().playhead, 2.5);
}
