//! Pure project data, validated edits, and read-only domain queries.
pub mod edit;
pub mod error;
pub mod model;
pub mod query;
pub mod transform;
pub use edit::library::{
    add_to_library, create_folder, delete_folder, move_folder, move_media, register_media,
    remove_media, rename_folder,
};
pub use edit::project::{
    set_media_scale_mode, set_project_duration, set_project_fps, set_project_output_size, MAX_FPS,
    MAX_OUTPUT_EDGE, MIN_FPS,
};
pub use edit::tracks::{
    add_track, can_move_track_down, can_move_track_up, cleanup_empty_tracks, compact_tracks,
    ensure_av_tracks, ensure_track_sections, insert_track_below, move_track, set_track_locked,
    set_track_muted,
};
pub use edit::{
    add_clip, add_imported_video, add_text, apply_generated, default_clip_id, delete_clips,
    delete_track, execute_edit, is_split_piece, move_clip, move_clips, paste_clips, place_media,
    place_media_range, razor, razor_clips, seek, set_text, stretch, stretch_clips, trim,
    trim_clips, validate_project, Anchor, CopiedClip, EditCommand, GeneratedClip, GeneratedEdit,
    GeneratedSource, LaneHint, TrimEdge,
};
pub use error::DomainError;
pub use model::clip::{
    clip_end, normalize_clip, source_span, text_clip, MAX_CLIP_RATE, MIN_CLIP_DURATION,
    MIN_CLIP_RATE,
};
pub use model::effects::{intro_at, picture_transform};
pub use model::media::{
    is_image_media, is_video_media, is_visual_media, media_kind, place_duration, RevealHand,
    RevealInfo, DEFAULT_STILL_DURATION_SECS,
};
pub use model::project::{
    create_project, DEFAULT_OUTPUT_HEIGHT, DEFAULT_OUTPUT_WIDTH, DEFAULT_WORKSPACE_SECS,
};
pub use model::text::{
    builtin_text_styles, text_raster_digest, PerCharReveal, DEFAULT_FONT_SIZE,
    DEFAULT_TEXT_DURATION_SECS, DEFAULT_TEXT_STYLE_ID, MAX_TEXT_CHARS,
};
pub use model::track::create_default_tracks;
pub use model::{
    AlphaPacking, Clip, ClipAudio, ClipOrigin, ClipSource, ClipTransform, FontRef, Intro,
    IntroKind, IntroSample, MediaFolder, MediaItem, MediaKind, MediaScaleMode, MediaSource,
    Project, TextAlign, TextAnimation, TextColor, TextContent, TextErode, TextFrame, TextIntro,
    TextLayer, TextLayerKind, TextLayout, TextPaint, TextStyle, Track, TrackKind,
};
pub use query::project::{content_duration, project_duration, timeline_extent, workspace_duration};
pub use query::{
    can_move_clips, can_place_media, clip_display_name, export_layers, find_clip, linked_clip_ids,
    resolve_audio_clips, resolve_layers, resolve_preview, reveal_frame, reveal_hand, FoundClip,
    HandLayer, HitSource, PreviewHit, ResolvedAudioClip, RevealFrame, MAX_DECODED_LAYERS,
    MAX_PREVIEW_LAYERS, MAX_TEXT_LAYERS,
};
pub use transform::{BaseSize, Transform, TransformBounds};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_default_project_roundtrip_json() {
        let p = create_project(None, None, None, None, None);
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.tracks.len(), 3);
    }

    #[test]
    fn move_and_seek() {
        let mut p = create_project(None, None, None, None, None);
        p.media.push(MediaItem {
            id: "m1".into(),
            name: "a".into(),
            path: Some("x.mp4".into()),
            duration: 10.0,
            has_audio: Some(true),
            width: Some(1920),
            height: Some(1080),
            kind: None,
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        p = add_clip(
            p,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 0.0, 2.0, 0.0, 1.0),
        );
        p = move_clip(p, "c1", 1.5, None);
        let clip = find_clip(&p, "c1").unwrap().clip;
        assert!((clip.start - 1.5).abs() < 1e-9);
        p = seek(p, 3.0);
        assert!((p.playhead - 3.0).abs() < 1e-9);
    }

    #[test]
    fn trim_out_keeps_offset_and_extends_duration() {
        let mut p = create_project(None, None, None, None, None);
        p.media.push(MediaItem {
            id: "m1".into(),
            name: "a".into(),
            path: Some("x.mp4".into()),
            duration: 10.0,
            has_audio: Some(true),
            width: Some(1920),
            height: Some(1080),
            kind: None,
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        p = add_clip(
            p,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 1.0, 2.0, 0.5, 1.0),
        );
        p = trim(p, "c1", TrimEdge::Out, 4.0, Some(10.0));
        let clip = find_clip(&p, "c1").unwrap().clip;
        assert!((clip.duration - 3.0).abs() < 1e-9);
        assert!((clip.media().unwrap().offset - 0.5).abs() < 1e-9);
        assert!((clip.start - 1.0).abs() < 1e-9);
    }

    #[test]
    fn resolve_preview_skips_clip_with_missing_media() {
        let mut p = create_project(None, None, None, None, None);
        p.media.push(MediaItem {
            id: "m2".into(),
            name: "b".into(),
            path: Some("b.mp4".into()),
            duration: 5.0,
            has_audio: None,
            width: None,
            height: None,
            kind: None,
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        p = add_clip(
            p,
            "v1",
            normalize_clip("gone".into(), "missing".into(), 0.0, 2.0, 0.0, 1.0),
        );
        assert!(p.tracks.iter().all(|track| track.clips.is_empty()));
        // add_clip refuses unknown media, so place the orphan directly.
        p.tracks[0].clips.insert(
            0,
            normalize_clip("gone".into(), "missing".into(), 0.0, 2.0, 0.0, 1.0),
        );
        // Query robustness is tested against a malformed imported document;
        // validated edits correctly reject overlaps with even orphaned clips.
        p.tracks[0].clips.push(normalize_clip(
            "c2".into(),
            "m2".into(),
            0.0,
            2.0,
            0.25,
            2.0,
        ));
        let hit = resolve_preview(&p, 0.5, None, Some(&[TrackKind::Video])).unwrap();
        assert_eq!(hit.clip.id, "c2");
        assert!((hit.media_time().unwrap() - 1.25).abs() < 1e-9);
    }

    #[test]
    fn resolve_layers_bottom_to_top_and_keeps_topmost_when_capped() {
        let mut p = create_project(None, None, None, None, None);
        for (id, name) in [("m1", "a.mp4"), ("m2", "b.mp4"), ("m3", "c.mp4")] {
            p.media.push(MediaItem {
                id: id.into(),
                name: name.into(),
                path: Some(name.into()),
                duration: 10.0,
                has_audio: Some(false),
                width: Some(1280),
                height: Some(720),
                kind: Some(MediaKind::Video),
                folder_id: None,
                alpha: None,
                reveal: None,
            });
        }
        // tracks[0]=v1 is visually highest; insert_track_below pushes later tracks lower.
        p = insert_track_below(p, "v1").unwrap();
        p = insert_track_below(p, "v1").unwrap();
        let video_ids: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .map(|t| t.id.clone())
            .collect();
        assert!(video_ids.len() >= 3);
        p = add_clip(
            p,
            &video_ids[0],
            normalize_clip("top".into(), "m1".into(), 0.0, 4.0, 0.0, 1.0),
        );
        p = add_clip(
            p,
            &video_ids[1],
            normalize_clip("mid".into(), "m2".into(), 0.0, 4.0, 0.0, 1.0),
        );
        p = add_clip(
            p,
            &video_ids[2],
            normalize_clip("bot".into(), "m3".into(), 0.0, 4.0, 0.0, 1.0),
        );
        let layers = resolve_layers(&p, 1.0);
        assert_eq!(
            layers
                .iter()
                .map(|h| h.clip.id.as_str())
                .collect::<Vec<_>>(),
            vec!["bot", "mid", "top"]
        );
        assert_eq!(
            resolve_preview(&p, 1.0, None, Some(&[TrackKind::Video]))
                .unwrap()
                .clip
                .id,
            "top"
        );

        // Over budget: keep topmost MAX_DECODED_LAYERS (drop deepest videos).
        while p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .count()
            < MAX_DECODED_LAYERS + 2
        {
            p = insert_track_below(p, "v1").unwrap();
        }
        let video_ids: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .map(|t| t.id.clone())
            .collect();
        for (i, track_id) in video_ids.iter().enumerate() {
            if find_clip(&p, "top").is_some() && track_id == &video_ids[0] {
                continue;
            }
            if p.tracks
                .iter()
                .find(|t| t.id == *track_id)
                .is_some_and(|t| !t.clips.is_empty())
            {
                continue;
            }
            let media = format!("pad{i}");
            p.media.push(MediaItem {
                id: media.clone(),
                name: format!("{media}.mp4"),
                path: Some(format!("{media}.mp4")),
                duration: 10.0,
                has_audio: Some(false),
                width: Some(64),
                height: Some(64),
                kind: Some(MediaKind::Video),
                folder_id: None,
                alpha: None,
                reveal: None,
            });
            p = add_clip(
                p,
                track_id,
                normalize_clip(format!("pad{i}"), media, 0.0, 4.0, 0.0, 1.0),
            );
        }
        let capped = resolve_layers(&p, 1.0);
        assert_eq!(capped.len(), MAX_DECODED_LAYERS);
        assert_eq!(capped.last().unwrap().clip.id, "top");
    }

    #[test]
    fn existing_project_json_camel_case() {
        // A stable legacy project: tests must not depend on the user's editable data/ project.
        let text = r#"{
          "version": 1, "fps": 30, "playhead": 0,
          "media": [{ "id": "m1", "name": "sample.mp4", "path": "sample.mp4",
            "duration": 10, "hasAudio": true, "width": 1920, "height": 1080 }],
          "tracks": [
            { "id": "v1", "name": "\u89c6\u98911", "kind": "video", "muted": false,
              "clips": [{ "id": "c1", "mediaId": "m1", "start": 0, "duration": 5, "offset": 0, "rate": 1 }] },
            { "id": "a1", "name": "audio", "kind": "audio", "muted": false, "clips": [] }
          ]
        }"#;
        let project: Project = serde_json::from_str(text).expect("deserialize project.json");
        assert_eq!(project.version, 1);
        assert_eq!(project.width, DEFAULT_OUTPUT_WIDTH);
        assert_eq!(project.height, DEFAULT_OUTPUT_HEIGHT);
        assert_eq!(project.tracks[0].kind, TrackKind::Video);
        assert_eq!(project.tracks[0].name, "视频1");
        assert_eq!(project.tracks[1].kind, TrackKind::Audio);
        if let Some(sfx) = project
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Sfx)
        {
            assert_eq!(sfx.name, "音效");
        }
        assert_eq!(project.media[0].has_audio, Some(true));
        assert!(project.media[0].duration.is_finite() && project.media[0].duration > 0.0);
        let value = serde_json::to_value(&project).unwrap();
        assert_eq!(value["tracks"][0]["kind"], "video");
        if project
            .tracks
            .iter()
            .any(|track| track.kind == TrackKind::Sfx)
        {
            assert!(value["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|track| track["kind"] == "sfx"));
        }
        assert_eq!(value["media"][0]["hasAudio"], true);
        assert!(value["media"][0].get("has_audio").is_none());
        assert!(value["tracks"][0]["clips"][0].get("mediaId").is_some());
        assert!(value["tracks"][0]["clips"][0].get("media_id").is_none());
        assert!(value.get("playhead").is_some());
        let again: Project = serde_json::from_value(value).unwrap();
        assert_eq!(
            again.tracks[0].clips[0].media_id().unwrap(),
            project.media[0].id
        );
    }

    #[test]
    fn stretch_razor_move_and_trim_in() {
        let mut p = sample_project();
        p = add_clip(
            p,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 0.0, 4.0, 0.0, 1.0),
        );
        p = add_clip(
            p,
            "a1",
            normalize_clip("c2".into(), "m1".into(), 1.0, 2.0, 0.0, 1.0),
        );
        let audio = resolve_audio_clips(&p);
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].id, "c2");
        assert_eq!(audio[0].path, "x.mp4");
        assert!((audio[0].gain - 1.0).abs() < 1e-9);
        let mut muted = p.clone();
        muted
            .tracks
            .iter_mut()
            .find(|track| track.id == "a1")
            .unwrap()
            .muted = true;
        assert!(resolve_audio_clips(&muted).is_empty());

        p = trim(p, "c2", TrimEdge::In, 0.0, Some(10.0));
        let held = find_clip(&p, "c2").unwrap().clip;
        assert!((held.start - 1.0).abs() < 1e-9);
        assert!((held.media().unwrap().offset).abs() < 1e-9);
        assert!((held.duration - 2.0).abs() < 1e-9);

        p = trim(p, "c2", TrimEdge::In, 1.5, Some(10.0));
        let trimmed = find_clip(&p, "c2").unwrap().clip;
        assert!((trimmed.start - 1.5).abs() < 1e-9);
        assert!((trimmed.media().unwrap().offset - 0.5).abs() < 1e-9);
        assert!((trimmed.duration - 1.5).abs() < 1e-9);
        assert!((trimmed.media().unwrap().rate - 1.0).abs() < 1e-9);

        let slowed = stretch(p.clone(), "c1", TrimEdge::In, 2.0);
        let slowed_clip = find_clip(&slowed, "c1").unwrap().clip;
        assert!((slowed_clip.start - 2.0).abs() < 1e-9);
        assert!((slowed_clip.duration - 2.0).abs() < 1e-9);
        assert!((slowed_clip.media().unwrap().rate - 2.0).abs() < 1e-9);
        assert!(slowed_clip.media().unwrap().offset.abs() < 1e-9);

        p = stretch(p, "c1", TrimEdge::Out, 8.0);
        let stretched = find_clip(&p, "c1").unwrap().clip;
        assert!((stretched.duration - 8.0).abs() < 1e-9);
        assert!((stretched.media().unwrap().rate - 0.5).abs() < 1e-9);
        assert!((stretched.media().unwrap().offset).abs() < 1e-9);

        let skipped = razor(p.clone(), 0.01, Some("v1"), || "nope".into());
        assert!(find_clip(&skipped, "nope").is_none());

        p = razor(p, 2.0, Some("v1"), || "c1b".into());
        let left = find_clip(&p, "c1").unwrap().clip;
        let right = find_clip(&p, "c1b").unwrap();
        assert_eq!(right.track_id, "v1");
        assert!((left.duration - 2.0).abs() < 1e-9);
        assert!((left.media().unwrap().rate - 0.5).abs() < 1e-9);
        assert!((right.clip.start - 2.0).abs() < 1e-9);
        assert!((right.clip.duration - 6.0).abs() < 1e-9);
        assert!((right.clip.media().unwrap().offset - 1.0).abs() < 1e-9);
        assert!((find_clip(&p, "c2").unwrap().clip.duration - 1.5).abs() < 1e-9);

        assert_ne!(default_clip_id(), default_clip_id());
        let mut seq = 0u32;
        let split = razor(p.clone(), 1.6, None, || {
            seq += 1;
            format!("n{seq}")
        });
        let mut ids = Vec::new();
        for track in &split.tracks {
            for clip in &track.clips {
                assert!(ids.iter().all(|id| id != &clip.id));
                ids.push(clip.id.clone());
            }
        }
        assert!(ids.len() >= 4);
        assert!(ids.iter().any(|id| id == "n1"));
        assert!(ids.iter().any(|id| id == "n2"));

        let rejected = move_clip(p.clone(), "c1", -3.0, Some("sfx1"));
        assert_eq!(rejected, p);
        p = add_track(p, TrackKind::Video);
        p = move_clip(p, "c1", -3.0, Some("v2"));
        let moved = find_clip(&p, "c1").unwrap();
        assert_eq!(moved.track_id, "v2");
        assert!(moved.clip.start.abs() < 1e-9);
        assert!(p
            .tracks
            .iter()
            .find(|track| track.id == "v1")
            .unwrap()
            .clips
            .iter()
            .all(|clip| clip.id != "c1"));

        let end = project_duration(&p);
        p = seek(p, end + 50.0);
        assert!((p.playhead - end).abs() < 1e-9);
        p = seek(p, -1.0);
        assert!(p.playhead.abs() < 1e-9);
    }

    #[test]
    fn add_imported_video_places_picture_and_dialogue() {
        let p = add_imported_video(
            create_project(None, None, None, None, None),
            MediaItem {
                id: "m9".into(),
                name: "v".into(),
                path: Some("v.mp4".into()),
                duration: 3.0,
                has_audio: Some(true),
                width: Some(1280),
                height: Some(720),
                kind: None,
                folder_id: None,
                alpha: None,
                reveal: None,
            },
            "vc".into(),
            "ac".into(),
        );
        assert_eq!(find_clip(&p, "vc").unwrap().track_id, "v1");
        assert_eq!(find_clip(&p, "ac").unwrap().track_id, "a1");
        assert!((find_clip(&p, "vc").unwrap().clip.duration - 3.0).abs() < 1e-9);
        assert!(p.playhead.abs() < 1e-9);
        // Short clips keep the default workspace; they do not shrink it.
        assert_eq!(p.duration, Some(DEFAULT_WORKSPACE_SECS));

        let p = add_imported_video(
            p,
            MediaItem {
                id: "m10".into(),
                name: "silent".into(),
                path: Some("s.mp4".into()),
                duration: 2.0,
                has_audio: Some(false),
                width: None,
                height: None,
                kind: None,
                folder_id: None,
                alpha: None,
                reveal: None,
            },
            "vc2".into(),
            "ac2".into(),
        );
        let second = find_clip(&p, "vc2").unwrap();
        assert_eq!(second.track_id, "v1");
        assert!((second.clip.start - 3.0).abs() < 1e-9);
        assert!(find_clip(&p, "ac2").is_none());
    }

    fn sample_project() -> Project {
        let mut p = create_project(None, None, None, None, None);
        p.media.push(MediaItem {
            id: "m1".into(),
            name: "a".into(),
            path: Some("x.mp4".into()),
            duration: 10.0,
            has_audio: Some(true),
            width: Some(1920),
            height: Some(1080),
            kind: None,
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        p
    }
}
