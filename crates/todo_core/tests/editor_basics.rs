use todo_core::*;

fn project() -> Project {
    let mut p = create_project(None, None, None, None, None);
    p.media.push(MediaItem {
        id: "m".into(),
        name: "movie".into(),
        path: Some("movie.mp4".into()),
        duration: 20.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: None,
    });
    p
}

fn put(p: Project, track: &str, id: &str, start: f64, duration: f64) -> Project {
    add_clip(
        p,
        track,
        normalize_clip(id.into(), "m".into(), start, duration, 0.0, 1.0),
    )
}

#[test]
fn workspace_is_composition_length_independent_of_unplaced_media() {
    let p = project();
    assert_eq!(workspace_duration(&p), DEFAULT_WORKSPACE_SECS);
    assert_eq!(project_duration(&p), DEFAULT_WORKSPACE_SECS);
    // Library media alone must not grow the workspace axis.
    assert_eq!(p.media[0].duration, 20.0);
    assert_eq!(workspace_duration(&p), DEFAULT_WORKSPACE_SECS);
}

#[test]
fn last_video_and_audio_tracks_cannot_be_deleted() {
    let p = project();
    assert_eq!(delete_track(p.clone(), "v1"), p);
    assert_eq!(delete_track(p.clone(), "a1"), p);
}

#[test]
fn locked_tracks_reject_add_move_trim_and_razor() {
    let mut p = put(project(), "v1", "v", 1.0, 4.0);
    p.tracks[0].locked = true;
    assert_eq!(put(p.clone(), "v1", "other", 8.0, 1.0), p);
    assert_eq!(move_clip(p.clone(), "v", 2.0, None), p);
    assert_eq!(trim(p.clone(), "v", TrimEdge::Out, 3.0, None), p);
    assert_eq!(razor(p.clone(), 3.0, None, || "right".into()), p);
}

#[test]
fn moves_to_missing_or_locked_tracks_preserve_source_clip() {
    let mut p = put(project(), "v1", "v", 1.0, 4.0);
    assert_eq!(move_clip(p.clone(), "v", 2.0, Some("missing")), p);
    p.tracks[1].locked = true;
    assert_eq!(move_clip(p.clone(), "v", 2.0, Some("a1")), p);
}

#[test]
fn hidden_video_tracks_are_not_previewed_even_when_requested_by_id() {
    let mut p = put(project(), "v1", "v", 0.0, 4.0);
    p.tracks[0].muted = true;
    assert!(resolve_preview(&p, 1.0, None, Some(&[TrackKind::Video])).is_none());
    assert!(resolve_preview(&p, 1.0, Some("v1"), None).is_none());
}

#[test]
fn legacy_razor_avoids_duplicate_ids() {
    let p = put(put(project(), "v1", "v", 0.0, 4.0), "a1", "a", 0.0, 4.0);
    let next = razor(p, 2.0, None, || "v".into());
    let ids: std::collections::HashSet<_> = next
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| &c.id))
        .collect();
    assert_eq!(ids.len(), 4);
}

#[test]
fn newly_added_tracks_survive_until_destructive_clip_cleanup() {
    let p = add_track(project(), TrackKind::Video);
    assert_eq!(p.tracks.len(), 4);
    let extra = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video && t.id != "v1")
        .unwrap()
        .id
        .clone();
    let p = put(p, &extra, "v", 0.0, 2.0);
    let next = delete_clips(p, &["v".into()], false);
    assert_eq!(
        next.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .count(),
        1
    );
    assert_eq!(
        next.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .count(),
        1
    );
    assert_eq!(next.media.len(), 1);
}

#[test]
fn cleanup_keeps_locked_empties_and_at_least_one_audio_video() {
    let mut p = add_track(project(), TrackKind::Video);
    let extra = p
        .tracks
        .iter()
        .position(|t| t.kind == TrackKind::Video && t.id != "v1")
        .unwrap();
    p.tracks[extra].locked = true;
    let next = cleanup_empty_tracks(p);
    // Locked empty video + required audio remain; unlocked empty sfx is dropped.
    assert_eq!(next.tracks.len(), 2);
    assert!(next
        .tracks
        .iter()
        .any(|t| t.kind == TrackKind::Video && t.locked));
    assert!(next.tracks.iter().any(|t| t.kind == TrackKind::Audio));
    let ensured = ensure_av_tracks(create_project(None, None, None, None, Some(vec![])));
    assert_eq!(ensured.tracks.len(), 2);
}

#[test]
fn track_controls_keep_content_and_delete_only_extra_tracks() {
    let p = put(add_track(project(), TrackKind::Video), "v1", "v", 0.0, 2.0);
    let next = set_track_muted(set_track_locked(p.clone(), "v1", true), "v1", true);
    assert!(next.tracks[0].locked && next.tracks[0].muted);
    assert_eq!(delete_track(next.clone(), "v1"), next);
    let next = delete_track(p, "v1");
    assert!(find_clip(&next, "v").is_none());
    assert_eq!(next.media.len(), 1);
}

#[test]
fn deletion_is_exact_leaves_gap_and_rejects_any_locked_selection() {
    let p = put(
        put(project(), "v1", "first", 0.0, 2.0),
        "v1",
        "later",
        4.0,
        2.0,
    );
    let next = delete_clips(p.clone(), &["first".into()], false);
    assert_eq!(find_clip(&next, "later").unwrap().clip.start, 4.0);
    assert!(find_clip(&next, "first").is_none());
    let mut locked = put(p, "a1", "audio", 0.0, 2.0);
    locked.tracks[1].locked = true;
    assert_eq!(
        delete_clips(locked.clone(), &["first".into(), "audio".into()], false),
        locked
    );
}

#[test]
fn ripple_merges_selected_intervals_and_shifts_all_affected_tracks() {
    let p = put(
        put(
            put(put(project(), "v1", "one", 2.0, 2.0), "a1", "two", 3.0, 2.0),
            "v1",
            "later",
            7.0,
            2.0,
        ),
        "a1",
        "sound",
        9.0,
        1.0,
    );
    let next = delete_clips(p, &["one".into(), "two".into()], true);
    assert_eq!(find_clip(&next, "later").unwrap().clip.start, 4.0);
    assert_eq!(find_clip(&next, "sound").unwrap().clip.start, 6.0);
}

#[test]
fn ripple_rejects_intersecting_unselected_clips_and_locked_downstream_tracks() {
    let p = put(
        put(project(), "v1", "cut", 2.0, 2.0),
        "a1",
        "cross",
        1.0,
        4.0,
    );
    assert_eq!(delete_clips(p.clone(), &["cut".into()], true), p);
    let mut p = put(
        put(project(), "v1", "cut", 2.0, 2.0),
        "a1",
        "sound",
        6.0,
        2.0,
    );
    p.tracks[1].locked = true;
    assert_eq!(delete_clips(p.clone(), &["cut".into()], true), p);
}

#[test]
fn still_image_places_on_video_track_with_hold_duration_and_no_audio_link() {
    let mut p = create_project(None, None, None, None, None);
    p.media.push(MediaItem {
        id: "img".into(),
        name: "cover.png".into(),
        path: Some("data/cover.png".into()),
        duration: DEFAULT_STILL_DURATION_SECS,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Image),
    });
    assert!(is_image_media(&p.media[0]));
    assert!(!is_video_media(&p.media[0]));
    assert!(is_visual_media(&p.media[0]));

    let p = place_media(p, "img", Some("v1"), 1.0).unwrap();
    let video = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap();
    assert_eq!(video.clips.len(), 1);
    assert!((video.clips[0].start - 1.0).abs() < 1e-9);
    assert!((video.clips[0].duration - DEFAULT_STILL_DURATION_SECS).abs() < 1e-9);
    assert!(video.clips[0].link_id.is_none());
    assert!(p
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .all(|t| t.clips.is_empty()));
}

#[test]
fn still_image_without_duration_uses_default_hold() {
    let mut p = create_project(None, None, None, None, None);
    p.media.push(MediaItem {
        id: "img".into(),
        name: "shot.jpg".into(),
        path: Some("shot.jpg".into()),
        duration: 0.0,
        has_audio: Some(false),
        width: Some(100),
        height: Some(100),
        kind: Some(MediaKind::Image),
    });
    let p = place_media(p, "img", None, 0.0).unwrap();
    let clip = &p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .clips[0];
    assert!((clip.duration - DEFAULT_STILL_DURATION_SECS).abs() < 1e-9);
}

#[test]
fn placement_links_video_audio_and_spills_occupied_track() {
    let p = place_media(project(), "m", Some("v1"), 2.0).unwrap();
    let video = &p.tracks[0].clips[0];
    let audio = &p.tracks[1].clips[0];
    assert_eq!(video.start, 2.0);
    assert_eq!(audio.start, 2.0);
    assert!(video.link_id.is_some());
    assert_eq!(video.link_id, audio.link_id);
    assert_eq!(
        linked_clip_ids(&p, std::slice::from_ref(&video.id)).len(),
        2
    );
    // Dropping onto an occupied track creates / uses another compatible track.
    let spilled = place_media(p, "m", Some("v1"), 2.0).unwrap();
    assert_eq!(
        spilled
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .count(),
        2
    );
}

#[test]
fn placement_uses_free_unlocked_audio_track_and_creates_compatible_tracks() {
    let mut p = project();
    p.tracks[1].locked = true;
    let p = place_media(p, "m", None, 0.0).unwrap();
    assert!(p.tracks[1].clips.is_empty());
    assert!(p
        .tracks
        .iter()
        .any(|t| t.kind == TrackKind::Audio && !t.locked && t.clips.len() == 1));
    let p = place_media(p, "m", None, 0.0).unwrap();
    assert_eq!(
        p.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .count(),
        2
    );
}

#[test]
fn audio_only_media_is_placed_only_on_audio_tracks() {
    let mut p = project();
    p.media[0].width = None;
    p.media[0].height = None;
    assert!(place_media(p.clone(), "m", Some("v1"), 0.0).is_err());
    let p = place_media(p, "m", None, 0.0).unwrap();
    assert!(p.tracks[0].clips.is_empty());
    assert_eq!(p.tracks[1].clips.len(), 1);
}

#[test]
fn removing_media_requires_explicit_cascade_and_rejects_locked_references() {
    let p = put(project(), "v1", "v", 0.0, 2.0);
    assert!(remove_media(p.clone(), "m", false).is_err());
    let mut locked = p.clone();
    locked.tracks[0].locked = true;
    assert!(remove_media(locked, "m", true).is_err());
    let next = remove_media(p, "m", true).unwrap();
    assert!(next.media.is_empty());
    assert!(find_clip(&next, "v").is_none());
    assert_eq!(next.tracks.len(), 2);
}

#[test]
fn group_move_reassigns_every_compatible_clip_onto_the_target_track() {
    let p = put(
        put(add_track(project(), TrackKind::Video), "v1", "a", 0.0, 2.0),
        "v2",
        "b",
        3.0,
        2.0,
    );
    let next = move_clips(p, &["a".into(), "b".into()], 0.0, Some("v1"));
    assert_eq!(find_clip(&next, "a").unwrap().track_id, "v1");
    assert_eq!(find_clip(&next, "b").unwrap().track_id, "v1");
    assert_eq!(
        next.tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap()
            .clips
            .len(),
        2
    );
}

#[test]
fn group_move_clamps_as_a_whole_and_preserves_other_members_tracks() {
    let p = put(
        put(add_track(project(), TrackKind::Video), "v1", "v", 2.0, 2.0),
        "a1",
        "a",
        3.0,
        2.0,
    );
    let target = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video && t.id != "v1")
        .unwrap()
        .id
        .clone();
    let next = move_clips(p, &["v".into(), "a".into()], -5.0, Some(&target));
    assert_eq!(find_clip(&next, "v").unwrap().clip.start, 0.0);
    assert_eq!(find_clip(&next, "v").unwrap().track_id, target);
    assert_eq!(find_clip(&next, "a").unwrap().clip.start, 1.0);
    assert_eq!(find_clip(&next, "a").unwrap().track_id, "a1");
}

#[test]
fn group_move_spills_overlap_but_rejects_incompatible_targets_and_locked_members_atomically() {
    let p = put(
        put(put(project(), "v1", "v", 0.0, 2.0), "v1", "block", 4.0, 2.0),
        "a1",
        "a",
        0.0,
        2.0,
    );
    let moved = move_clips(p.clone(), &["v".into(), "a".into()], 3.0, None);
    assert_eq!(find_clip(&moved, "v").unwrap().track_id, "v2");
    assert_eq!(find_clip(&moved, "a").unwrap().track_id, "a1");
    assert_eq!(find_clip(&moved, "a").unwrap().clip.start, 3.0);
    assert_eq!(move_clips(p.clone(), &["a".into()], 0.0, Some("v1")), p);
    let mut locked = p;
    locked.tracks[1].locked = true;
    assert_eq!(
        move_clips(locked.clone(), &["v".into(), "a".into()], 1.0, None),
        locked
    );
}

#[test]
fn linked_razor_preserves_rate_offsets_and_creates_distinct_pair_links() {
    let mut p = place_media(project(), "m", None, 2.0).unwrap();
    for t in &mut p.tracks {
        for c in &mut t.clips {
            c.media_mut().unwrap().rate = 2.0;
            c.media_mut().unwrap().offset = 1.0;
            c.duration = 6.0;
        }
    }
    let ids: Vec<_> = p
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
        .collect();
    let next = razor_clips(p, 4.0, &ids);
    let left_v = &next.tracks[0].clips[0];
    let right_v = &next.tracks[0].clips[1];
    let left_a = &next.tracks[1].clips[0];
    let right_a = &next.tracks[1].clips[1];
    assert_eq!(left_v.duration, 2.0);
    assert_eq!(right_v.media().unwrap().offset, 5.0);
    assert_eq!(right_v.duration, 4.0);
    assert_eq!(right_v.media().unwrap().rate, 2.0);
    assert_eq!(left_v.link_id, left_a.link_id);
    assert_eq!(right_v.link_id, right_a.link_id);
    assert_ne!(left_v.link_id, right_v.link_id);
}

#[test]
fn linked_razor_refuses_locked_group_and_does_not_cut_unselected_clips() {
    let mut p = place_media(project(), "m", None, 0.0).unwrap();
    let ids: Vec<_> = p
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
        .collect();
    p.tracks[1].locked = true;
    assert_eq!(razor_clips(p.clone(), 2.0, &ids), p);
    p.tracks[1].locked = false;
    let next = razor_clips(p, 2.0, &[ids[0].clone()]);
    assert_eq!(next.tracks[0].clips.len(), 2);
    assert_eq!(next.tracks[1].clips.len(), 1);
}

#[test]
fn link_ids_roundtrip_and_survive_normalization_trim_stretch_and_move() {
    let p = place_media(project(), "m", None, 0.0).unwrap();
    let clip = p.tracks[0].clips[0].clone();
    let json = serde_json::to_value(&p).unwrap();
    assert!(json["tracks"][0]["clips"][0].get("linkId").is_some());
    let back: Project = serde_json::from_value(json).unwrap();
    assert_eq!(back, p);
    let normalized = create_project(None, None, None, None, Some(p.tracks.clone()));
    assert_eq!(normalized.tracks[0].clips[0].link_id, clip.link_id);
    for next in [
        move_clip(p.clone(), &clip.id, 1.0, None),
        trim(p.clone(), &clip.id, TrimEdge::Out, 5.0, None),
        stretch(p, &clip.id, TrimEdge::Out, 10.0),
    ] {
        assert_eq!(
            find_clip(&next, &clip.id).unwrap().clip.link_id,
            clip.link_id
        );
    }
    let legacy = normalize_clip("legacy".into(), "m".into(), 0.0, 1.0, 0.0, 1.0);
    assert!(serde_json::to_value(&legacy)
        .unwrap()
        .get("linkId")
        .is_none());
}

#[test]
fn razor_accepts_one_frame_pieces_but_rejects_sub_frame_pieces() {
    let p = put(project(), "v1", "v", 0.0, 1.0);
    let next = razor(p.clone(), 1.0 / 30.0, Some("v1"), || "right".into());
    assert_eq!(next.tracks[0].clips.len(), 2);
    assert!((next.tracks[0].clips[0].duration - 1.0 / 30.0).abs() < 1e-9);
    assert!((next.tracks[0].clips[1].media().unwrap().offset - 1.0 / 30.0).abs() < 1e-9);
    let selected = razor_clips(p.clone(), 1.0 / 30.0, &["v".into()]);
    assert_eq!(selected.tracks[0].clips.len(), 2);
    assert_eq!(razor_clips(p.clone(), 0.01, &["v".into()]), p);
    assert_eq!(razor(p.clone(), 0.01, None, || "too_short".into()), p);
}

#[test]
fn destructive_edits_preserve_workspace_duration() {
    let mut p = put(project(), "v1", "v", 0.0, 2.0);
    p.duration = Some(46.016);
    let deleted = delete_clips(p.clone(), &["v".into()], false);
    assert_eq!(project_duration(&deleted), 46.016);
    let removed = remove_media(p.clone(), "m", true).unwrap();
    assert_eq!(project_duration(&removed), 46.016);
    let p = add_track(p, TrackKind::Video);
    assert_eq!(project_duration(&delete_track(p, "v1")), 46.016);
}
