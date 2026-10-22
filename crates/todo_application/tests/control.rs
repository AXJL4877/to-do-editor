use todo_application::{control::*, *};
use todo_core::{create_project, find_clip, MediaKind};

fn workspace() -> Workspace {
    let mut project = create_project(Some(30.0), None, None, None, None);
    project.media.push(MediaItem {
        id: "m".into(),
        name: "A.mp4".into(),
        path: Some("A.mp4".into()),
        duration: 30.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    Workspace::with_project(project, "control-test")
}

fn request(workspace: &Workspace, id: &str, command: ControlCommand) -> ControlRequest {
    let context = workspace.control_context().unwrap();
    ControlRequest {
        request_id: id.into(),
        session_id: context.session_id,
        expected_revision: Some(context.revision),
        command,
    }
}

fn batch(edits: Vec<ControlEdit>) -> ControlCommand {
    ControlCommand::ApplyEdits {
        edits,
        dry_run: false,
    }
}

fn place() -> ControlEdit {
    ControlEdit::PlaceMedia {
        media_id: "m".into(),
        track_id: Some("v1".into()),
        timeline_start: 10.0,
        source_range: Some(SourceRange {
            source_in: 3.0,
            source_out: 8.0,
        }),
    }
}

fn ok(response: &ControlResponse) -> &ControlResult {
    match &response.outcome {
        ControlStatus::Ok { result } => result,
        ControlStatus::Error { error } => panic!("unexpected control error: {error:?}"),
    }
}

fn error(response: &ControlResponse) -> &ControlError {
    match &response.outcome {
        ControlStatus::Error { error } => error,
        ControlStatus::Ok { result } => panic!("expected error, got {result:?}"),
    }
}

#[test]
fn place_and_split_batch_is_sequential_and_one_undo_restores_the_project() {
    let mut ws = workspace();
    let original = ws.editor().unwrap().project().clone();
    let command = batch(vec![
        place(),
        ControlEdit::Split {
            clips: vec![ClipRef::Created { step: 0 }],
            timeline_time: 12.0,
        },
    ]);
    let outcome = ws.control(request(&ws, "cut", command));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    assert!(receipt.changed);
    assert!(!receipt.dry_run);
    assert_eq!(receipt.steps.len(), 2);
    assert_eq!(receipt.steps[0].created_clips.len(), 2);
    assert_eq!(receipt.steps[1].splits.len(), 2);
    assert_eq!(ws.control_context().unwrap().revision, 1);
    assert_eq!(outcome.update.new_clips.len(), 4);
    assert!(outcome
        .update
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Preview(_))));
    let clips: Vec<_> = ws
        .editor()
        .unwrap()
        .project()
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .collect();
    assert_eq!(clips.len(), 4);
    for split in &receipt.steps[1].splits {
        let left = find_clip(ws.editor().unwrap().project(), &split.left_id)
            .unwrap()
            .clip;
        let right = find_clip(ws.editor().unwrap().project(), &split.right_id)
            .unwrap()
            .clip;
        assert_eq!(
            (left.start, left.media().unwrap().offset, left.duration),
            (10.0, 3.0, 2.0)
        );
        assert_eq!(
            (right.start, right.media().unwrap().offset, right.duration),
            (12.0, 5.0, 3.0)
        );
    }
    let undo = ws.control(request(&ws, "undo", ControlCommand::Undo));
    assert!(matches!(
        ok(&undo.response),
        ControlResult::History { changed: true }
    ));
    assert_eq!(ws.editor().unwrap().project(), &original);
    assert!(!ws.editor().unwrap().can_undo());
    assert_eq!(ws.control_context().unwrap().revision, 2);
    let redo = ws.control(request(&ws, "redo", ControlCommand::Redo));
    assert!(matches!(
        ok(&redo.response),
        ControlResult::History { changed: true }
    ));
    assert_eq!(
        ws.editor()
            .unwrap()
            .project()
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .count(),
        4
    );
}

#[test]
fn failed_step_rolls_back_every_previous_edit_and_history_entry() {
    let mut ws = workspace();
    let original = ws.editor().unwrap().project().clone();
    let outcome = ws.control(request(
        &ws,
        "invalid",
        batch(vec![
            place(),
            ControlEdit::Split {
                clips: vec![ClipRef::Id {
                    id: "missing".into(),
                }],
                timeline_time: 12.0,
            },
        ]),
    ));
    let error = error(&outcome.response);
    assert_eq!(error.code, "clip_not_found");
    assert_eq!(error.step, Some(1));
    assert_eq!(error.object_id.as_deref(), Some("missing"));
    assert!(outcome.update.effects.is_empty());
    assert_eq!(ws.editor().unwrap().project(), &original);
    assert!(!ws.editor().unwrap().can_undo());
    assert_eq!(ws.control_context().unwrap().revision, 0);
}

#[test]
fn dry_run_returns_changes_without_affecting_project_history_or_revision() {
    let mut ws = workspace();
    let original = ws.editor().unwrap().project().clone();
    let outcome = ws.control(request(
        &ws,
        "preview",
        ControlCommand::ApplyEdits {
            edits: vec![place()],
            dry_run: true,
        },
    ));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    assert!(receipt.dry_run && receipt.changed);
    assert_eq!(receipt.steps[0].created_clips.len(), 2);
    assert_eq!(ws.editor().unwrap().project(), &original);
    assert_eq!(ws.control_context().unwrap().revision, 0);
    assert!(!ws.editor().unwrap().can_undo());
    assert!(outcome.update.effects.is_empty());
}

#[test]
fn retries_return_the_original_receipt_without_repeating_effects_even_after_an_edit() {
    let mut ws = workspace();
    let req = request(&ws, "place", batch(vec![place()]));
    let first = ws.control(req.clone());
    ok(&first.response);
    ws.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Video,
        },
    )));
    let retried = ws.control(req.clone());
    assert_eq!(first.response, retried.response);
    assert!(retried.update.effects.is_empty());
    assert!(retried.update.new_clips.is_empty());
    assert_eq!(ws.control_context().unwrap().revision, 2);
    let mut conflict = req;
    conflict.command = ControlCommand::Undo;
    assert_eq!(
        error(&ws.control(conflict).response).code,
        "request_id_conflict"
    );
}

#[test]
fn stale_missing_revision_and_active_gestures_reject_control_mutations() {
    let mut ws = workspace();
    let stale = request(&ws, "stale", batch(vec![place()]));
    ws.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Video,
        },
    )));
    assert_eq!(error(&ws.control(stale).response).code, "revision_conflict");
    let mut missing = request(&ws, "missing-revision", ControlCommand::Undo);
    missing.expected_revision = None;
    assert_eq!(
        error(&ws.control(missing).response).code,
        "revision_required"
    );
    ws.dispatch(AppAction::Editor(EditorCommand::BeginEdit));
    let outcome = ws.control(request(&ws, "busy", batch(vec![place()])));
    assert_eq!(error(&outcome.response).code, "editor_busy");
    assert!(ws.editor().unwrap().transaction_active());
    ws.dispatch(AppAction::Editor(EditorCommand::CancelEdit));
}

#[test]
fn gesture_changes_and_cancel_increment_revision_but_playhead_and_save_do_not() {
    let mut ws = workspace();
    ws.dispatch(AppAction::Editor(EditorCommand::Seek(2.0)));
    assert_eq!(ws.control_context().unwrap().revision, 0);
    ws.dispatch(AppAction::Editor(EditorCommand::BeginEdit));
    ws.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddTrack {
            kind: TrackKind::Video,
        },
    )));
    assert_eq!(ws.control_context().unwrap().revision, 1);
    ws.dispatch(AppAction::Editor(EditorCommand::CancelEdit));
    assert_eq!(ws.control_context().unwrap().revision, 2);
    let snapshot = ws.editor().unwrap().project().clone();
    ws.dispatch(AppAction::Editor(EditorCommand::Saved(snapshot)));
    assert_eq!(ws.control_context().unwrap().revision, 2);
}

#[test]
fn queries_are_live_and_wire_format_rejects_unknown_fields() {
    let mut ws = workspace();
    let req = request(&ws, "query", ControlCommand::GetContext);
    let first = ws.control(req.clone());
    let json = serde_json::to_value(&first.response).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["apiVersion"], 1);
    assert_eq!(json["requestId"], "query");
    ws.dispatch(AppAction::Editor(EditorCommand::Seek(9.0)));
    let second = ws.control(req.clone());
    let ControlResult::Context { context } = ok(&second.response) else {
        panic!()
    };
    assert_eq!(context.playhead, 9.0);
    let mut wire = serde_json::to_value(req).unwrap();
    wire["unrecognized"] = true.into();
    assert!(serde_json::from_value::<ControlRequest>(wire).is_err());
    let parsed: ControlRequest = serde_json::from_str(r#"{"requestId":"json","sessionId":1,"expectedRevision":0,"command":{"type":"apply_edits","edits":[{"type":"place_media","mediaId":"m","trackId":"v1","timelineStart":10,"sourceRange":{"sourceIn":3,"sourceOut":8}}]}}"#).unwrap();
    assert!(matches!(
        parsed.command,
        ControlCommand::ApplyEdits { dry_run: false, .. }
    ));
}

#[test]
fn invalid_forward_references_and_empty_batches_do_not_edit() {
    let mut ws = workspace();
    let outcome = ws.control(request(
        &ws,
        "forward",
        batch(vec![ControlEdit::Split {
            clips: vec![ClipRef::Created { step: 0 }],
            timeline_time: 12.0,
        }]),
    ));
    assert_eq!(error(&outcome.response).code, "invalid_clip_reference");
    assert_eq!(
        error(&ws.control(request(&ws, "empty", batch(vec![]))).response).code,
        "invalid_request"
    );
    assert!(!ws.editor().unwrap().can_undo());
}

fn import_ticket(outcome: &ControlOutcome) -> Ticket {
    outcome
        .update
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Media {
                ticket,
                task: MediaTask::Import { .. },
            } => Some(*ticket),
            _ => None,
        })
        .unwrap()
}

fn imported(id: &str) -> MediaItem {
    MediaItem {
        id: id.into(),
        name: format!("{id}.mp4"),
        path: Some(format!("{id}.mp4")),
        duration: 20.0,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    }
}

#[test]
fn control_import_only_registers_library_items_and_reports_partial_failures() {
    let mut ws = workspace();
    let req = request(
        &ws,
        "import",
        ControlCommand::ImportMedia {
            paths: vec!["A.mp4".into(), "bad.mp4".into()],
            folder_id: None,
        },
    );
    let outcome = ws.control(req.clone());
    let ticket = import_ticket(&outcome);
    assert!(
        matches!(ok(&outcome.response), ControlResult::ImportStarted { job } if job.status == ImportStatus::Running && job.total == 2)
    );
    assert!(ws.control(req).update.effects.is_empty());
    assert_eq!(
        error(
            &ws.control(request(&ws, "edit-during-import", batch(vec![place()])))
                .response
        )
        .code,
        "editor_busy"
    );
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::Imported(imported("new"))),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::Imported(imported("new"))),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Err("bad.mp4: invalid media".into()),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::ImportFinished),
    });
    assert!(!ws.editor().unwrap().importing());
    assert_eq!(ws.editor().unwrap().project().media.len(), 2);
    assert!(ws
        .editor()
        .unwrap()
        .project()
        .tracks
        .iter()
        .all(|track| track.clips.is_empty()));
    let outcome = ws.control(request(
        &ws,
        "job",
        ControlCommand::GetImportJob { job_id: ticket.id },
    ));
    let ControlResult::ImportJob { job } = ok(&outcome.response) else {
        panic!()
    };
    assert_eq!(job.status, ImportStatus::CompletedWithErrors);
    assert_eq!(job.completed, 2);
    assert_eq!(job.media_ids, ["new"]);
    assert_eq!(job.errors.len(), 1);
    assert_eq!(ws.control_context().unwrap().revision, 1);
}

#[test]
fn manual_import_still_appends_to_the_timeline() {
    let mut ws = workspace();
    let update = ws.dispatch(AppAction::ImportFiles(vec!["A.mp4".into()]));
    let ticket = update
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Media { ticket, .. } => Some(*ticket),
            _ => None,
        })
        .unwrap();
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::Imported(imported("new"))),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::ImportFinished),
    });
    assert_eq!(
        ws.editor()
            .unwrap()
            .project()
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .count(),
        1
    );
}

#[test]
fn aborted_import_finishes_the_job_and_releases_the_editor() {
    let mut ws = workspace();
    let outcome = ws.control(request(
        &ws,
        "import",
        ControlCommand::ImportMedia {
            paths: vec!["A.mp4".into()],
            folder_id: None,
        },
    ));
    let ticket = import_ticket(&outcome);
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::ImportAborted {
            message: "worker stopped".into(),
        }),
    });
    assert!(!ws.editor().unwrap().importing());
    let outcome = ws.control(request(
        &ws,
        "job",
        ControlCommand::GetImportJob { job_id: ticket.id },
    ));
    let ControlResult::ImportJob { job } = ok(&outcome.response) else {
        panic!()
    };
    assert_eq!(job.status, ImportStatus::Failed);
    assert_eq!(job.errors, ["worker stopped"]);
    assert!(matches!(
        ok(&ws
            .control(request(&ws, "edit", batch(vec![place()])))
            .response),
        ControlResult::Edits { .. }
    ));
}

#[test]
fn reopening_clears_request_history_and_ignores_old_import_results() {
    let mut ws = workspace();
    let req = request(
        &ws,
        "old",
        ControlCommand::ImportMedia {
            paths: vec!["A.mp4".into()],
            folder_id: None,
        },
    );
    let old = import_ticket(&ws.control(req.clone()));
    let update = ws.dispatch(AppAction::OpenProject("other".into()));
    let ticket = update
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Project { ticket, .. } => Some(*ticket),
            _ => None,
        })
        .unwrap();
    ws.on_event(AppEvent::Project {
        ticket,
        result: Ok(ProjectOutput::Opened {
            name: "other".into(),
            project: workspace().editor().unwrap().project().clone(),
        }),
    });
    ws.on_event(AppEvent::Media {
        ticket: old,
        result: Ok(MediaOutput::Imported(imported("stale"))),
    });
    assert_eq!(ws.editor().unwrap().project().media.len(), 1);
    assert_eq!(error(&ws.control(req).response).code, "session_conflict");
    assert!(ws.control_context().unwrap().imports.is_empty());
}

#[test]
fn evicted_request_ids_are_rejected_instead_of_being_executed_again() {
    let mut ws = workspace();
    let first = request(&ws, "old", ControlCommand::Undo);
    ok(&ws.control(first.clone()).response);
    for index in 0..130 {
        let req = request(&ws, &format!("undo-{index}"), ControlCommand::Undo);
        ok(&ws.control(req).response);
    }
    assert_eq!(error(&ws.control(first).response).code, "request_expired");
    assert!(!ws.editor().unwrap().can_undo());
}

#[test]
fn move_trim_transform_and_linked_delete_use_domain_rules_and_return_actual_changes() {
    let mut ws = workspace();
    let transform = todo_core::Transform {
        opacity: 0.5,
        ..Default::default()
    };
    let outcome = ws.control(request(
        &ws,
        "edit",
        batch(vec![
            place(),
            ControlEdit::MoveClips {
                clips: vec![ClipRef::Created { step: 0 }],
                delta: 2.0,
                track_id: Some("v1".into()),
                linked: true,
            },
            ControlEdit::TrimClip {
                clip: ClipRef::Created { step: 0 },
                edge: ClipEdge::Out,
                timeline_time: 16.0,
                linked: true,
            },
            ControlEdit::SetTransform {
                clip: ClipRef::Created { step: 0 },
                transform,
            },
        ]),
    ));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    assert_eq!(receipt.steps[1].updated_clips.len(), 2);
    assert_eq!(receipt.steps[2].updated_clips.len(), 2);
    assert_eq!(receipt.steps[3].updated_clips.len(), 1);
    let video_id = receipt.steps[0].created_clips[0].clip.id.clone();
    let video = find_clip(ws.editor().unwrap().project(), &video_id)
        .unwrap()
        .clip;
    assert_eq!(
        (video.start, video.media().unwrap().offset, video.duration),
        (12.0, 3.0, 4.0)
    );
    assert_eq!(video.transform.opacity, 0.5);
    let outcome = ws.control(request(
        &ws,
        "delete",
        batch(vec![ControlEdit::DeleteClips {
            clips: vec![ClipRef::Id { id: video_id }],
            ripple: false,
            linked: true,
        }]),
    ));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    assert_eq!(receipt.steps[0].deleted_clips.len(), 2);
    assert!(ws
        .editor()
        .unwrap()
        .project()
        .tracks
        .iter()
        .all(|track| track.clips.is_empty()));
}

#[test]
fn locked_tracks_and_outside_split_return_errors_or_explicit_no_change() {
    let mut ws = workspace();
    let outcome = ws.control(request(&ws, "place", batch(vec![place()])));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    let id = receipt.steps[0].created_clips[0].clip.id.clone();
    let outcome = ws.control(request(
        &ws,
        "outside",
        batch(vec![ControlEdit::Split {
            clips: vec![ClipRef::Id { id: id.clone() }],
            timeline_time: 50.0,
        }]),
    ));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    assert!(!receipt.changed);
    assert!(receipt.steps[0].splits.is_empty());
    assert_eq!(ws.control_context().unwrap().revision, 1);
    ws.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::ToggleLock {
            track_id: "a1".into(),
        },
    )));
    let outcome = ws.control(request(
        &ws,
        "locked",
        batch(vec![ControlEdit::Split {
            clips: vec![ClipRef::Id { id }],
            timeline_time: 12.0,
        }]),
    ));
    assert_eq!(error(&outcome.response).code, "track_locked");
    assert_eq!(error(&outcome.response).object_id.as_deref(), Some("a1"));
    assert_eq!(ws.control_context().unwrap().revision, 2);
}

#[test]
fn pending_project_transition_blocks_control_until_the_open_finishes() {
    let mut ws = workspace();
    ws.dispatch(AppAction::OpenProject("next".into()));
    let outcome = ws.control(request(&ws, "busy", batch(vec![place()])));
    assert_eq!(error(&outcome.response).code, "editor_busy");
    assert!(!ws.editor().unwrap().can_undo());
    assert!(ws.control_context().unwrap().busy);
}

#[test]
fn import_to_library_then_place_and_cut_uses_the_returned_media_id() {
    let mut ws = workspace();
    let outcome = ws.control(request(
        &ws,
        "import",
        ControlCommand::ImportMedia {
            paths: vec!["A.mp4".into()],
            folder_id: None,
        },
    ));
    let ticket = import_ticket(&outcome);
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::Imported(imported("returned"))),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::ImportFinished),
    });
    let mut placement = place();
    if let ControlEdit::PlaceMedia { media_id, .. } = &mut placement {
        *media_id = "returned".into();
    }
    let outcome = ws.control(request(
        &ws,
        "edit",
        batch(vec![
            placement,
            ControlEdit::Split {
                clips: vec![ClipRef::Created { step: 0 }],
                timeline_time: 12.0,
            },
        ]),
    ));
    ok(&outcome.response);
    assert_eq!(
        ws.editor()
            .unwrap()
            .project()
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .count(),
        2
    );
    ok(&ws
        .control(request(&ws, "undo", ControlCommand::Undo))
        .response);
    assert!(ws
        .editor()
        .unwrap()
        .project()
        .tracks
        .iter()
        .all(|track| track.clips.is_empty()));
    assert!(ws
        .editor()
        .unwrap()
        .project()
        .media
        .iter()
        .any(|media| media.id == "returned"));
}

#[test]
fn invalid_import_metadata_does_not_corrupt_the_project_and_finishes_with_a_failure() {
    let mut ws = workspace();
    let outcome = ws.control(request(
        &ws,
        "import",
        ControlCommand::ImportMedia {
            paths: vec!["A.mp4".into()],
            folder_id: None,
        },
    ));
    let ticket = import_ticket(&outcome);
    let mut media = imported("bad");
    media.duration = -1.0;
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::Imported(media)),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::ImportFinished),
    });
    assert_eq!(ws.editor().unwrap().project().media.len(), 1);
    assert_eq!(ws.control_context().unwrap().revision, 0);
    let outcome = ws.control(request(
        &ws,
        "job",
        ControlCommand::GetImportJob { job_id: ticket.id },
    ));
    assert!(
        matches!(ok(&outcome.response), ControlResult::ImportJob { job } if job.status == ImportStatus::Failed && job.errors == ["invalid_duration"])
    );
}

#[test]
fn manual_import_receipt_reports_its_own_result_after_a_previous_domain_error() {
    let mut ws = workspace();
    ws.dispatch(AppAction::Editor(EditorCommand::Edit(EditCommand::Split {
        clip_ids: vec!["missing".into()],
        time: 1.0,
    })));
    let update = ws.dispatch(AppAction::ImportFiles(vec!["A.mp4".into()]));
    let ticket = update
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Media { ticket, .. } => Some(*ticket),
            _ => None,
        })
        .unwrap();
    for _ in 0..2 {
        ws.on_event(AppEvent::Media {
            ticket,
            result: Ok(MediaOutput::Imported(imported("new"))),
        });
    }
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::ImportFinished),
    });
    let job = ws.control_context().unwrap().imports.pop().unwrap();
    assert_eq!(job.status, ImportStatus::Completed);
    assert_eq!(job.media_ids, ["new"]);
    assert_eq!(job.completed, 1);
    assert!(job.errors.is_empty());
    assert_eq!(
        ws.editor()
            .unwrap()
            .project()
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .count(),
        1
    );
}

#[test]
fn manual_import_rejects_bad_metadata_and_retains_a_short_import_when_placement_fails() {
    let mut ws = workspace();
    let update = ws.dispatch(AppAction::ImportFiles(vec![
        "bad.mp4".into(),
        "short.mp4".into(),
    ]));
    let ticket = update
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Media { ticket, .. } => Some(*ticket),
            _ => None,
        })
        .unwrap();
    let mut bad = imported("bad");
    bad.duration = -1.0;
    let mut short = imported("short");
    short.duration = 0.01;
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::Imported(bad)),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::Imported(short)),
    });
    ws.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::ImportFinished),
    });
    todo_core::validate_project(ws.editor().unwrap().project()).unwrap();
    assert_eq!(ws.editor().unwrap().project().media.len(), 2);
    assert!(ws
        .editor()
        .unwrap()
        .project()
        .media
        .iter()
        .any(|item| item.id == "short"));
    let job = ws.control_context().unwrap().imports.pop().unwrap();
    assert_eq!(job.status, ImportStatus::CompletedWithErrors);
    assert_eq!(job.media_ids, ["short"]);
    assert_eq!(job.completed, 2);
    assert_eq!(job.errors.len(), 2);
}

#[test]
fn transform_wire_payload_requires_known_camel_case_fields_and_a_complete_transform() {
    let base = serde_json::json!({
        "requestId": "transform", "sessionId": 1, "expectedRevision": 0,
        "command": { "type": "apply_edits", "edits": [{
            "type": "set_transform", "clip": { "type": "id", "id": "picture" },
            "transform": { "x": 0.0, "y": 0.0, "scale": 1.0, "scaleX": 2.0, "scaleY": 1.0, "opacity": 0.5 }
        }]}
    });
    let valid: ControlRequest = serde_json::from_value(base.clone()).unwrap();
    assert!(
        matches!(&valid.command, ControlCommand::ApplyEdits { edits, .. }
        if matches!(edits[0], ControlEdit::SetTransform { transform, .. } if transform.scale_x == 2.0 && transform.opacity == 0.5))
    );
    let mut unknown = base.clone();
    unknown["command"]["edits"][0]["transform"]["scale_x"] = 3.0.into();
    assert!(serde_json::from_value::<ControlRequest>(unknown).is_err());
    let mut partial = base;
    partial["command"]["edits"][0]["transform"] = serde_json::json!({"scaleX": 2.0});
    assert!(serde_json::from_value::<ControlRequest>(partial).is_err());
}

#[test]
fn json_text_edits_create_split_and_restyle_text_with_raster_jobs() {
    let mut ws = workspace();
    let edits: Vec<ControlEdit> = serde_json::from_value(serde_json::json!([
        { "type": "add_text", "trackId": null, "timelineStart": 1.0, "duration": 4.0,
          "content": { "text": "钱越来越 不值钱了", "styleId": "grunge-red-echo",
                       "animation": { "intro": { "kind": "perChar", "duration": 1.0, "charDuration": 0.2 } } },
          "transform": { "x": 0.0, "y": 0.4, "scale": 1.0, "scaleX": 1.0, "scaleY": 1.0, "opacity": 1.0 } },
        { "type": "split", "clips": [{ "type": "created", "step": 0 }], "timelineTime": 3.0 },
        { "type": "set_text", "clip": { "type": "created", "step": 0 },
          "content": { "text": "新的", "styleId": "clean-white-outline" } }
    ]))
    .unwrap();
    let outcome = ws.control(request(&ws, "text", batch(edits)));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    assert_eq!(receipt.steps[0].created_clips.len(), 1);
    assert_eq!(receipt.steps[1].splits.len(), 1);
    let project = ws.editor().unwrap().project();
    let text_id = &receipt.steps[0].created_clips[0].clip.id;
    let text = find_clip(project, text_id).unwrap();
    assert_eq!(text.track.kind, TrackKind::Text);
    assert_eq!(text.clip.text().unwrap().text, "新的");
    assert_eq!(text.clip.transform.y, 0.4);
    assert_eq!(project.text_styles.len(), 2);
    let right = &receipt.steps[1].splits[0].right_id;
    assert!(find_clip(project, right)
        .unwrap()
        .clip
        .text()
        .unwrap()
        .animation
        .intro
        .is_none());

    let rejected: Result<Vec<ControlEdit>, _> = serde_json::from_value(serde_json::json!([
        { "type": "add_text", "timelineStart": 0.0, "duration": 1.0,
          "content": { "text": "x", "styleId": "s" }, "color": "red" }
    ]));
    assert!(rejected.is_err(), "unknown fields are refused");
}

#[test]
fn split_then_transition_and_library_presets_copy_into_the_project() {
    let mut ws = workspace();
    let mut custom = todo_core::motion::builtin_motions()
        .iter()
        .find(|preset| preset.id == "crossfade")
        .unwrap()
        .clone();
    custom.id = "studio.bubbleFade".into();
    ws.set_motion_library(vec![custom.clone()]);
    // Wire format as an agent writes it.
    let edits: Vec<ControlEdit> = serde_json::from_value(serde_json::json!([
        { "type": "place_media", "mediaId": "m", "trackId": "v1", "timelineStart": 0.0,
          "sourceRange": { "sourceIn": 0.0, "sourceOut": 6.0 } },
        { "type": "split", "clips": [{ "type": "created", "step": 0 }], "timelineTime": 3.0 },
        { "type": "set_intro", "clip": { "type": "created", "step": 0 },
          "intro": { "kind": "pop", "duration": 0.3 } }
    ]))
    .unwrap();
    let outcome = ws.control(request(&ws, "cut", batch(edits)));
    let ControlResult::Edits { receipt } = ok(&outcome.response) else {
        panic!()
    };
    let right = receipt.steps[1].splits[0].right_id.clone();
    let transition: ControlEdit = serde_json::from_value(serde_json::json!(
        { "type": "set_transition", "clip": { "type": "id", "id": right },
          "transition": { "kind": "studio.bubbleFade", "duration": 0.4 } }
    ))
    .unwrap();
    let outcome = ws.control(request(&ws, "fade", batch(vec![transition])));
    ok(&outcome.response);
    let project = ws.editor().unwrap().project();
    assert_eq!(project.motions, vec![custom]);
    let layers = todo_core::resolve_layers(project, 3.0);
    assert_eq!(layers.len(), 2, "both sides of the cut are drawn");

    let view = project_view(project, Some(3.0));
    let incoming = view.elements.iter().find(|e| e.clip_id == right).unwrap();
    assert!(incoming.transition.as_ref().unwrap().active);
    assert_eq!(incoming.visible, Some(true));

    // An unknown preset is rejected with the motion's id.
    let bad: ControlEdit = serde_json::from_value(serde_json::json!(
        { "type": "set_transition", "clip": { "type": "id", "id": right },
          "transition": { "kind": "nope", "duration": 0.4 } }
    ))
    .unwrap();
    let outcome = ws.control(request(&ws, "bad", batch(vec![bad])));
    assert_eq!(error(&outcome.response).code, "motion_not_found");
}

#[test]
fn get_production_context_empty_and_host_commands_are_explicit() {
    let mut ws = workspace();
    let outcome = ws.control(request(
        &ws,
        "prod-ctx",
        ControlCommand::GetProductionContext {
            production_id: None,
        },
    ));
    let ControlResult::ProductionContext { context } = ok(&outcome.response) else {
        panic!("expected production context");
    };
    assert!(context.productions.is_empty());

    let dry = ws.control(request(
        &ws,
        "wf-dry",
        ControlCommand::StartWorkflow {
            extension_id: "studio.demo".into(),
            input: None,
            params: None,
            dry_run: true,
        },
    ));
    assert_eq!(error(&dry.response).code, "workflow_host_required");

    let list = ws.control(request(&ws, "ext", ControlCommand::ListExtensions));
    assert_eq!(error(&list.response).code, "workflow_host_required");
}

#[test]
fn get_context_accepts_session_zero_bootstrap() {
    let mut ws = workspace();
    let live = ws.control_context().unwrap().session_id;
    assert_ne!(live, 0);
    let outcome = ws.control(ControlRequest {
        request_id: "boot".into(),
        session_id: 0,
        expected_revision: None,
        command: ControlCommand::GetContext,
    });
    let ControlResult::Context { context } = ok(&outcome.response) else {
        panic!("expected context, got {:?}", outcome.response);
    };
    assert_eq!(context.session_id, live);
}

#[test]
fn mutations_still_reject_wrong_session() {
    let mut ws = workspace();
    let outcome = ws.control(ControlRequest {
        request_id: "bad".into(),
        session_id: 0,
        expected_revision: None,
        command: ControlCommand::Undo,
    });
    assert_eq!(error(&outcome.response).code, "session_conflict");
}
