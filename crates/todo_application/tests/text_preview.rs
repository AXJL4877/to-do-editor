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
fn text_waits_for_its_raster_then_composites_above_video() {
    let mut workspace = workspace();
    let update = add_text(&mut workspace, None);
    // The stack is held back until the text image exists.
    assert_eq!(layers(&update), None);
    let jobs = raster_jobs(&update);
    assert_eq!(jobs.len(), 1);
    let (ticket, raster) = &jobs[0];
    assert_eq!(raster.canvas, (1920, 1080));
    assert!(raster.reveal.is_none());
    assert!(workspace.busy());

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
