use super::*;
use todo_core::find_clip;

fn late_clip() -> Project {
    let mut project = with_clip();
    project.duration = Some(300.0);
    project.media[0].duration = 900.0;
    project.tracks[0].clips[0].start = 225.0;
    project.tracks[0].clips[0].duration = 90.0;
    project
}

#[test]
fn overflow_is_scrollable_without_rescaling_the_fixed_300_second_workspace() {
    let mut project = late_clip();
    project.tracks[0].clips[0].duration = 120.0;
    let mut editor = TestEditor::new(project, "test");
    editor.apply(Intent::Viewport {
        scroll_left: 0.0,
        width: LABEL_W + 900.0,
    });
    let timeline = editor.view().timeline;
    assert_eq!(timeline.px_per_sec, 3.0);
    assert_eq!(timeline.axis, 345.0);
    assert_eq!(timeline.lane_width, 1035.0);
    assert_eq!(timeline.board_width, LABEL_W + 1035.0);
    assert!(timeline.ticks.iter().any(|tick| tick.time > 300.0));
    assert_eq!(editor.view().duration, 300.0);
}

#[test]
fn repeated_drags_after_3m45s_keep_pixel_time_mapping_and_settings_stable() {
    let base = late_clip();
    let mut editor = TestEditor::new(base.clone(), "test");
    editor.apply(Intent::Viewport {
        scroll_left: 0.0,
        width: LABEL_W + 900.0,
    });
    editor.apply(Intent::ToggleSnapping);
    let origin_x = LABEL_W + 225.0 * 3.0 + 20.0;
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: origin_x,
        scroll_left: 0.0,
        local_x: 20.0,
        width_px: 270.0,
        alt: false,
    });
    for (delta, scroll) in [(5.0, 0.0), (15.0, 20.0), (25.0, 40.0)] {
        editor.apply(Intent::PointerMove {
            view_x: origin_x + delta * 3.0 - scroll,
            scroll_left: scroll,
            track_index: Some(0),
        });
        assert_eq!(editor.project().duration, Some(300.0));
        assert_eq!(editor.view().timeline.px_per_sec, 3.0);
        assert_eq!(
            find_clip(editor.project(), "c1").unwrap().clip.start,
            225.0 + delta
        );
        assert_eq!(
            find_clip(editor.project(), "c1").unwrap().clip.duration,
            90.0
        );
        assert_eq!(editor.view().timeline.axis, 315.0 + delta);
    }
    editor.apply(Intent::PointerUp);
    editor.apply(Intent::Undo);
    assert_eq!(editor.project(), &base);
}

#[test]
fn a_late_clip_can_snap_its_trailing_edge_to_the_fixed_workspace_end() {
    let mut editor = TestEditor::new(late_clip(), "test");
    editor.apply(Intent::Viewport {
        scroll_left: 0.0,
        width: LABEL_W + 900.0,
    });
    let x = LABEL_W + 225.0 * 3.0 + 20.0;
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: x,
        scroll_left: 0.0,
        local_x: 20.0,
        width_px: 270.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: x - 14.9 * 3.0,
        scroll_left: 0.0,
        track_index: Some(0),
    });
    assert_eq!(editor.view().timeline.snap_target, Some(300.0));
    assert_eq!(
        todo_core::clip_end(&find_clip(editor.project(), "c1").unwrap().clip),
        300.0
    );
    assert_eq!(editor.project().duration, Some(300.0));
}
