//! Run with `cargo run -p todo_application --example control`.
//! The in-memory media fixture avoids filesystem/network/preview dependencies.
use todo_application::{control::*, Workspace};
use todo_core::{create_project, MediaItem, MediaKind};

fn main() {
    let media = MediaItem {
        id: "A".into(),
        name: "A.mp4".into(),
        path: Some("A.mp4".into()),
        duration: 30.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
};
    let project = create_project(Some(30.0), None, None, Some(vec![media]), None);
    let mut workspace = Workspace::with_project(project, "shared-control-demo");
    let context = workspace.control_context().unwrap();
    let request = ControlRequest {
        request_id: "place-and-cut".into(),
        session_id: context.session_id,
        expected_revision: Some(context.revision),
        command: ControlCommand::ApplyEdits {
            dry_run: false,
            edits: vec![
                ControlEdit::PlaceMedia {
                    media_id: "A".into(),
                    track_id: Some("v1".into()),
                    timeline_start: 10.0,
                    source_range: Some(SourceRange {
                        source_in: 3.0,
                        source_out: 8.0,
                    }),
                },
                ControlEdit::Split {
                    clips: vec![ClipRef::Created { step: 0 }],
                    timeline_time: 12.0,
                },
            ],
        },
    };
    println!(
        "Request:\n{}",
        serde_json::to_string_pretty(&request).unwrap()
    );
    let outcome = workspace.control(request);
    println!(
        "Response:\n{}",
        serde_json::to_string_pretty(&outcome.response).unwrap()
    );
    // A real host forwards outcome.update to its normal runtime effect handler.
    assert!(matches!(outcome.response.outcome, ControlStatus::Ok { .. }));
    let context = workspace.control_context().unwrap();
    assert_eq!(context.revision, 1);
    assert_eq!(
        context
            .project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .count(),
        4
    );
    let undo = workspace.control(ControlRequest {
        request_id: "undo".into(),
        session_id: context.session_id,
        expected_revision: Some(context.revision),
        command: ControlCommand::Undo,
    });
    assert!(matches!(undo.response.outcome, ControlStatus::Ok { .. }));
    assert!(workspace
        .editor()
        .unwrap()
        .project()
        .tracks
        .iter()
        .all(|track| track.clips.is_empty()));
    println!("One undo restored the timeline; media remains in the library.");
}
