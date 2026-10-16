use todo_application::*;
use todo_core::{create_project, place_media, MediaKind};

fn project() -> Project {
    let mut project = create_project(None, None, Some(60.0), None, None);
    project.media.push(MediaItem {
        id: "m".into(),
        name: "clip.mp4".into(),
        path: Some("clip.mp4".into()),
        duration: 4.0,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
    });
    place_media(project, "m", Some("v1"), 0.0).unwrap()
}
fn effect_ticket(update: &AppUpdate) -> Ticket {
    update
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Project { ticket, .. } | Effect::Media { ticket, .. } => Some(*ticket),
            _ => None,
        })
        .unwrap()
}

#[test]
fn opacity_and_axis_updates_emit_transform_without_seeking_or_reopening_video() {
    let mut editor = EditorSession::new(project(), "test");
    let _ = editor.sync();
    let clip_id = editor
        .project()
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .next()
        .unwrap()
        .id
        .clone();
    let transform = ClipTransform {
        scale_x: 0.5,
        scale_y: 1.25,
        opacity: 0.35,
        ..Default::default()
    };
    let update = editor.apply(EditorCommand::Edit(EditCommand::SetTransform {
        clip_id,
        transform,
    }));
    assert_eq!(
        update.commands,
        vec![PreviewCommand::Layers(vec![PreviewLayer {
            path: "clip.mp4".into(),
            media_time: 0.0,
            aspect: Some((1920, 1080)),
            transform,
        }])]
    );
    let update = editor.apply(EditorCommand::Undo);
    assert!(update.commands.iter().any(|c| matches!(
        c,
        PreviewCommand::Layers(layers)
            if layers.len() == 1 && layers[0].transform == ClipTransform::default()
    )));
}

#[test]
fn one_transaction_is_one_undo_and_paste_preserves_sequence_duration() {
    let mut editor = EditorSession::new(project(), "test");
    let id = editor
        .project()
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .next()
        .unwrap()
        .id
        .clone();
    let _ = editor.apply(EditorCommand::BeginEdit);
    for delta in [1.0, 2.0] {
        let _ = editor.apply(EditorCommand::Edit(EditCommand::MoveClips {
            clip_ids: vec![id.clone()],
            delta,
            track_id: None,
        }));
    }
    let _ = editor.apply(EditorCommand::CommitEdit);
    assert_eq!(
        todo_core::find_clip(editor.project(), &id)
            .unwrap()
            .clip
            .start,
        2.0
    );
    let _ = editor.apply(EditorCommand::Undo);
    assert_eq!(
        todo_core::find_clip(editor.project(), &id)
            .unwrap()
            .clip
            .start,
        0.0
    );
    assert!(!editor.can_undo());
    let _ = editor.apply(EditorCommand::Copy { clip_ids: vec![id] });
    let _ = editor.apply(EditorCommand::Seek(59.0));
    let _ = editor.apply(EditorCommand::Paste);
    assert_eq!(editor.project().duration, Some(60.0));
}

#[test]
fn inserting_a_track_is_one_undoable_edit_and_redo_restores_its_position() {
    let original = project();
    let mut editor = EditorSession::new(original.clone(), "test");
    let update = editor.apply(EditorCommand::Edit(EditCommand::InsertTrackBelow {
        track_id: "v1".into(),
    }));
    assert!(update.content_changed);
    assert!(update.new_clips.is_empty());
    assert_eq!(
        editor
            .project()
            .tracks
            .iter()
            .map(|track| track.id.as_str())
            .collect::<Vec<_>>(),
        ["v1", "v2", "a1", "sfx1"]
    );
    let inserted = editor.project().clone();
    assert!(editor.dirty());
    assert!(editor.can_undo());
    let _ = editor.apply(EditorCommand::Undo);
    assert_eq!(editor.project(), &original);
    assert!(!editor.dirty());
    assert!(!editor.can_undo());
    assert!(editor.can_redo());
    let _ = editor.apply(EditorCommand::Redo);
    assert_eq!(editor.project(), &inserted);
    assert!(editor.dirty());
    assert!(editor.can_undo());
    assert!(!editor.can_redo());
}

#[test]
fn stale_open_result_cannot_replace_a_newer_requested_project() {
    let mut workspace = Workspace::default();
    let first = effect_ticket(&workspace.dispatch(AppAction::OpenProject("old".into())));
    let next = effect_ticket(&workspace.dispatch(AppAction::OpenProject("new".into())));
    workspace.on_event(AppEvent::Project {
        ticket: first,
        result: Ok(ProjectOutput::Opened {
            name: "old".into(),
            project: project(),
        }),
    });
    assert!(workspace.editor().is_none());
    workspace.on_event(AppEvent::Project {
        ticket: next,
        result: Ok(ProjectOutput::Opened {
            name: "new".into(),
            project: project(),
        }),
    });
    assert_eq!(workspace.editor().unwrap().project_name(), "new");
    workspace.on_event(AppEvent::Project {
        ticket: first,
        result: Ok(ProjectOutput::Opened {
            name: "old".into(),
            project: project(),
        }),
    });
    assert_eq!(workspace.editor().unwrap().project_name(), "new");
}

#[test]
fn failed_back_save_retains_the_editor_and_unsaved_content() {
    let mut workspace = Workspace::with_project(project(), "test");
    workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Video,
        },
    )));
    let ticket = effect_ticket(&workspace.dispatch(AppAction::Back));
    assert_eq!(workspace.route(), Route::Editor);
    workspace.on_event(AppEvent::Project {
        ticket,
        result: Err("disk full".into()),
    });
    assert_eq!(workspace.route(), Route::Editor);
    assert!(workspace.editor().unwrap().dirty());
}

#[test]
fn close_waits_for_the_exact_save() {
    let mut workspace = Workspace::with_project(project(), "test");
    workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Video,
        },
    )));
    let update = workspace.dispatch(AppAction::RequestClose);
    assert!(workspace.close_prompt());
    assert!(!update
        .effects
        .iter()
        .any(|e| matches!(e, Effect::CloseWindow)));
    let update = workspace.dispatch(AppAction::ConfirmClose(CloseChoice::Save));
    let ticket = effect_ticket(&update);
    let saved = workspace.editor().unwrap().project().clone();
    assert!(!update
        .effects
        .iter()
        .any(|e| matches!(e, Effect::CloseWindow)));
    let update = workspace.on_event(AppEvent::Project {
        ticket,
        result: Ok(ProjectOutput::Saved(saved)),
    });
    assert!(update
        .effects
        .iter()
        .any(|e| matches!(e, Effect::CloseWindow)));
}

#[test]
fn newer_edits_during_a_back_save_are_saved_before_leaving() {
    let mut workspace = Workspace::with_project(project(), "test");
    workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Video,
        },
    )));
    let first = workspace.dispatch(AppAction::Back);
    let ticket = effect_ticket(&first);
    let saved = workspace.editor().unwrap().project().clone();
    workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Audio,
        },
    )));
    let update = workspace.on_event(AppEvent::Project {
        ticket,
        result: Ok(ProjectOutput::Saved(saved)),
    });
    assert_eq!(
        workspace.route(),
        Route::Editor,
        "newer edits must not be discarded"
    );
    let ticket = effect_ticket(&update);
    let saved = workspace.editor().unwrap().project().clone();
    workspace.on_event(AppEvent::Project {
        ticket,
        result: Ok(ProjectOutput::Saved(saved)),
    });
    assert_eq!(workspace.route(), Route::Projects);
}

#[test]
fn leaving_a_clean_editor_forgets_pending_save_tickets() {
    let mut workspace = Workspace::with_project(project(), "test");
    let update = workspace.dispatch(AppAction::Save);
    let ticket = effect_ticket(&update);
    let saved = workspace.editor().unwrap().project().clone();
    workspace.dispatch(AppAction::Back);
    assert!(
        !workspace.saving(),
        "previous session saves must not keep the home screen busy"
    );
    workspace.on_event(AppEvent::Project {
        ticket,
        result: Ok(ProjectOutput::Saved(saved)),
    });
    assert!(!workspace.saving());
}

#[test]
fn requesting_close_finishes_the_active_edit_and_pauses_playback() {
    let mut workspace = Workspace::with_project(project(), "test");
    workspace.dispatch(AppAction::Editor(EditorCommand::TogglePlay));
    workspace.dispatch(AppAction::Editor(EditorCommand::BeginEdit));
    workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Video,
        },
    )));
    workspace.dispatch(AppAction::RequestClose);
    let editor = workspace.editor().unwrap();
    assert!(!editor.playing());
    assert!(!editor.transaction_active());
    assert!(workspace.close_prompt());
    workspace.dispatch(AppAction::ConfirmClose(CloseChoice::Cancel));
    assert!(workspace.editor().unwrap().dirty());
}

#[test]
fn resource_failure_has_backoff_and_does_not_cancel_an_import() {
    let mut workspace = Workspace::with_project(project(), "test");
    workspace.dispatch(AppAction::ImportFiles(vec!["next.mp4".into()]));
    let requests = vec![AssetRequest::Peaks {
        path: "clip.mp4".into(),
        bins: 128,
    }];
    let update = workspace.dispatch(AppAction::RequestAssets {
        now: 0.0,
        requests: requests.clone(),
    });
    let ticket = effect_ticket(&update);
    workspace.on_event(AppEvent::Media {
        ticket,
        result: Err("decode failed".into()),
    });
    assert!(workspace.editor().unwrap().importing());
    assert!(workspace
        .dispatch(AppAction::RequestAssets {
            now: 0.1,
            requests: requests.clone()
        })
        .effects
        .is_empty());
    assert!(!workspace
        .dispatch(AppAction::RequestAssets { now: 3.0, requests })
        .effects
        .is_empty());
}
