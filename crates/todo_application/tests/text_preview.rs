use todo_application::*;
use todo_core::{create_project, place_media, MediaKind, TextContent, TextIntro, TransformBounds};

fn workspace() -> Workspace {
    let mut project = create_project(None, None, Some(60.0), None, None);
    project.media.push(MediaItem {
        id: "m".into(),
        name: "clip.mp4".into(),
        path: Some("clip.mp4".into()),
        duration: 10.0,
        has_audio: Some(false),
        width: Some(1280),
        height: Some(720),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    let project = place_media(project, "m", Some("v1"), 0.0).unwrap();
    let mut workspace = Workspace::with_project(project, "test");
    let _ = workspace.resync_preview();
    workspace
}

fn add_text(workspace: &mut Workspace, intro: Option<TextIntro>) -> AppUpdate {
    let mut content = TextContent::new("钱越来越 不值钱了", todo_core::DEFAULT_TEXT_STYLE_ID);
    content.animation.intro = intro;
    workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddText {
            track_id: None,
            time: 0.0,
            duration: 3.0,
            content,
        },
    )))
}

const BLOCK: TransformBounds = TransformBounds {
    left: 0.1,
    top: 0.45,
    right: 0.9,
    bottom: 0.55,
};

fn raster_jobs(update: &AppUpdate) -> Vec<(Ticket, TextRaster)> {
    update
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Media {
                ticket,
                task: MediaTask::TextRaster(raster),
            } => Some((*ticket, raster.as_ref().clone())),
            _ => None,
        })
        .collect()
}

fn layers(update: &AppUpdate) -> Option<Vec<PreviewLayer>> {
    update.effects.iter().find_map(|effect| match effect {
        Effect::Preview(commands) => commands.iter().find_map(|command| match command {
            PreviewCommand::Layers(layers) => Some(layers.clone()),
            _ => None,
        }),
        _ => None,
    })
}

#[test]
fn pending_text_allows_video_to_progress_then_composites_above_video() {
    let mut workspace = workspace();
    let update = add_text(&mut workspace, None);
    // The unchanged video stack need not be sent again while text is pending.
    assert_eq!(layers(&update), None);
    let jobs = raster_jobs(&update);
    assert_eq!(jobs.len(), 1);
    let (ticket, raster) = &jobs[0];
    assert_eq!(raster.canvas, (1920, 1080));
    assert!(raster.reveal.is_none());
    assert!(workspace.busy());

    let seek = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.5)));
    let stack = layers(&seek).expect("video must advance before the first text raster exists");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].path, "clip.mp4");
    assert_eq!(stack[0].media_time, 0.5);

    let done = workspace.on_event(AppEvent::Media {
        ticket: *ticket,
        result: Ok(MediaOutput::TextRaster {
            path: "cache/text.png".into(),
            block: BLOCK,
        }),
    });
    let stack = layers(&done).expect("stack sent after raster");
    assert_eq!(stack.len(), 2);
    assert_eq!(stack[0].path, "clip.mp4");
    assert_eq!(stack[1].path, "cache/text.png");
    assert_eq!(stack[1].aspect, Some((1920, 1080)));
    assert!(!workspace.busy());
    let editor = workspace.editor().unwrap();
    let hit = todo_core::resolve_layers(editor.project(), editor.playhead()).remove(1);
    let base = editor.text_base(&hit).unwrap();
    assert!((base.width - 0.8).abs() < 1e-9 && (base.height - 0.1).abs() < 1e-9);

    // Unchanged text reuses the ready raster: no new job when the picture refreshes.
    let refresh = workspace.dispatch(AppAction::Editor(EditorCommand::RefreshPicture));
    assert!(raster_jobs(&refresh).is_empty());
    assert_eq!(layers(&refresh).unwrap()[1].path, "cache/text.png");
}

fn reveal_intro() -> Option<TextIntro> {
    Some(TextIntro::PerChar {
        duration: 1.0,
        char_duration: 0.2,
    })
}

fn complete_raster(workspace: &mut Workspace, ticket: Ticket, path: &str) -> AppUpdate {
    workspace.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::TextRaster {
            path: path.into(),
            block: BLOCK,
        }),
    })
}

fn text_clip(workspace: &Workspace) -> (String, TextContent) {
    workspace
        .editor()
        .unwrap()
        .project()
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find_map(|clip| {
            clip.text()
                .map(|content| (clip.id.clone(), content.clone()))
        })
        .unwrap()
}

#[test]
fn pending_reveal_reuses_the_shown_caption_with_current_transform() {
    let mut workspace = workspace();
    let first = add_text(&mut workspace, reveal_intro());
    complete_raster(&mut workspace, raster_jobs(&first)[0].0, "frame-0.png");
    let pending = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.5)));
    let stack = layers(&pending).expect("pending reveal must not freeze video");
    assert_eq!(stack.len(), 2);
    assert_eq!(stack[0].media_time, 0.5);
    assert_eq!(stack[1].path, "frame-0.png");

    let (clip_id, _) = text_clip(&workspace);
    let transform = ClipTransform {
        opacity: 0.4,
        x: 0.2,
        ..Default::default()
    };
    let moved = workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::SetTransform { clip_id, transform },
    )));
    let stack = layers(&moved).expect("fallback uses the current transform");
    assert_eq!(stack[1].path, "frame-0.png");
    assert_eq!(stack[1].transform, transform);
    let done = complete_raster(&mut workspace, raster_jobs(&pending)[0].0, "frame-15.png");
    assert_eq!(layers(&done).unwrap()[1].path, "frame-15.png");
}

#[test]
fn late_reveal_completions_do_not_regress_the_shown_caption() {
    let mut workspace = workspace();
    let first = add_text(&mut workspace, reveal_intro());
    complete_raster(&mut workspace, raster_jobs(&first)[0].0, "frame-0.png");
    let old = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.25)));
    let newer = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.5)));
    complete_raster(&mut workspace, raster_jobs(&newer)[0].0, "frame-15.png");
    let pending = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.75)));
    assert_eq!(layers(&pending).unwrap()[1].path, "frame-15.png");
    let late = complete_raster(&mut workspace, raster_jobs(&old)[0].0, "frame-7.png");
    assert!(layers(&late).is_none());
    assert_eq!(
        workspace.editor().unwrap().shown_layers()[0].media_time,
        0.75
    );
    assert_eq!(
        workspace.editor().unwrap().shown_layers()[1].path,
        "frame-15.png"
    );
    let exact = complete_raster(&mut workspace, raster_jobs(&pending)[0].0, "frame-22.png");
    assert_eq!(layers(&exact).unwrap()[1].path, "frame-22.png");
    // Reverse seeks may select an older ready frame when it is the current request.
    let reverse = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.25)));
    assert_eq!(layers(&reverse).unwrap()[1].path, "frame-7.png");
}

#[test]
fn incompatible_text_edits_clear_the_old_caption_while_rendering() {
    for change in ["content", "layout", "style", "canvas"] {
        let mut workspace = workspace();
        let first = add_text(&mut workspace, reveal_intro());
        complete_raster(&mut workspace, raster_jobs(&first)[0].0, "old-caption.png");
        let (clip_id, mut content) = text_clip(&workspace);
        let edit = match change {
            "content" => {
                content.text = "new caption".into();
                EditCommand::SetText { clip_id, content }
            }
            "layout" => {
                content.layout.font_size = 0.2;
                EditCommand::SetText { clip_id, content }
            }
            "style" => {
                content.style_id = "clean-white-outline".into();
                EditCommand::SetText { clip_id, content }
            }
            "canvas" => EditCommand::SetOutputSize {
                width: 1280,
                height: 720,
            },
            _ => unreachable!(),
        };
        let update = workspace.dispatch(AppAction::Editor(EditorCommand::Edit(edit)));
        let stack =
            layers(&update).unwrap_or_else(|| panic!("{change} must clear the old caption"));
        assert_eq!(stack.len(), 1, "{change}");
        assert_eq!(stack[0].path, "clip.mp4");
        assert_eq!(raster_jobs(&update).len(), 1, "{change}");
    }
}

#[test]
fn a_new_clip_cannot_borrow_another_clips_reveal_frame() {
    let mut workspace = workspace();
    let first = add_text(&mut workspace, reveal_intro());
    complete_raster(&mut workspace, raster_jobs(&first)[0].0, "old-clip.png");
    let (old_id, content) = text_clip(&workspace);
    workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
        EditCommand::AddText {
            track_id: None,
            time: 3.0,
            duration: 3.0,
            content,
        },
    )));
    let next = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(3.5)));
    let stack =
        layers(&next).expect("the new caption's pending raster must not retain the old clip");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].media_time, 3.5);
    assert_ne!(
        workspace.editor().unwrap().picture().clip_id.as_deref(),
        Some(old_id.as_str())
    );
}

#[test]
fn switching_to_source_clears_caption_fallback_before_returning_to_program() {
    let mut workspace = workspace();
    let first = add_text(&mut workspace, reveal_intro());
    complete_raster(
        &mut workspace,
        raster_jobs(&first)[0].0,
        "before-source.png",
    );
    workspace.dispatch(AppAction::Editor(EditorCommand::OpenSourceMonitor {
        media_id: "m".into(),
    }));
    let program = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.5)));
    let stack = layers(&program).expect("program video must replace the source immediately");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].media_time, 0.5);
}

#[test]
fn opening_another_project_cannot_receive_a_previous_projects_caption() {
    let mut workspace = workspace();
    let first = add_text(&mut workspace, reveal_intro());
    let previous_ticket = raster_jobs(&first)[0].0;
    let next_project = workspace.editor().unwrap().project().clone();
    let open = workspace.dispatch(AppAction::OpenProject("next".into()));
    let ticket = open
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Project { ticket, .. } => Some(*ticket),
            _ => None,
        })
        .unwrap();
    let opened = workspace.on_event(AppEvent::Project {
        ticket,
        result: Ok(ProjectOutput::Opened {
            name: "next".into(),
            project: next_project,
        }),
    });
    let current_ticket = raster_jobs(&opened)[0].0;
    assert_eq!(workspace.editor().unwrap().shown_layers().len(), 1);
    let stale = complete_raster(&mut workspace, previous_ticket, "previous-project.png");
    assert!(stale.effects.is_empty());
    assert_eq!(workspace.editor().unwrap().shown_layers().len(), 1);
    let current = complete_raster(&mut workspace, current_ticket, "current-project.png");
    assert_eq!(layers(&current).unwrap()[1].path, "current-project.png");
}

#[test]
fn fade_scales_layer_opacity_without_new_rasters() {
    let mut workspace = workspace();
    let update = add_text(&mut workspace, Some(TextIntro::Fade { duration: 2.0 }));
    let (ticket, _) = raster_jobs(&update)[0].clone();
    let _ = workspace.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::TextRaster {
            path: "t.png".into(),
            block: BLOCK,
        }),
    });
    let update = workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.5)));
    assert!(raster_jobs(&update).is_empty());
    let stack = layers(&update).unwrap();
    assert!((stack[1].transform.opacity - 0.25).abs() < 1e-9);
}

#[test]
fn per_char_intro_requests_one_raster_per_output_frame() {
    let mut workspace = workspace();
    let intro = TextIntro::PerChar {
        duration: 1.0,
        char_duration: 0.2,
    };
    let update = add_text(&mut workspace, Some(intro));
    let first = raster_jobs(&update);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].1.reveal.unwrap().time, 0.0);
    // 0.51 s and 0.52 s fall into the same 30 fps frame (15).
    let a = raster_jobs(&workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.51))));
    let b = raster_jobs(&workspace.dispatch(AppAction::Editor(EditorCommand::Seek(0.52))));
    assert_eq!(a.len(), 1);
    assert!(b.is_empty());
    assert!((a[0].1.reveal.unwrap().time - 0.5).abs() < 1e-9);
    // After the intro the raster is the static one.
    let late = raster_jobs(&workspace.dispatch(AppAction::Editor(EditorCommand::Seek(2.0))));
    assert_eq!(late.len(), 1);
    assert!(late[0].1.reveal.is_none());
}

#[test]
fn failed_raster_drops_only_the_text_layer() {
    let mut workspace = workspace();
    let update = add_text(&mut workspace, None);
    let (ticket, _) = raster_jobs(&update)[0].clone();
    let _ = workspace.on_event(AppEvent::Media {
        ticket,
        result: Err("font not found".into()),
    });
    // The video-only stack is already on screen, so nothing is re-sent.
    let shown = workspace.editor().unwrap().shown_layers();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].path, "clip.mp4");
    assert!(!workspace.busy());
    // The failure is not retried on every redraw.
    let refresh = workspace.dispatch(AppAction::Editor(EditorCommand::RefreshPicture));
    assert!(raster_jobs(&refresh).is_empty());
}
