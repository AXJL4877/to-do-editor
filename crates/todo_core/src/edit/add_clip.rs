use crate::model::clip::{normalize_clip, Clip};
use crate::model::track::{Track, TrackKind};
use crate::model::Project;
use crate::{add_to_library, MediaItem};

pub fn add_clip(project: Project, track_id: &str, clip: Clip) -> Project {
    if clip
        .media_id()
        .is_some_and(|id| !project.media.iter().any(|m| m.id == id))
        || !project.tracks.iter().any(|t| t.id == track_id && !t.locked)
        || project
            .tracks
            .iter()
            .any(|t| t.clips.iter().any(|c| c.id == clip.id))
    {
        return project;
    }
    let target = project.tracks.iter().find(|t| t.id == track_id).unwrap();
    if crate::edit::validate_placement(&project, target, &clip, &[]).is_err() {
        return project;
    }
    let next = clip;
    Project {
        tracks: project
            .tracks
            .into_iter()
            .map(|track| {
                if track.id != track_id {
                    return track;
                }
                if track.clips.iter().any(|c| c.id == next.id) {
                    return track;
                }
                let mut clips = track.clips;
                clips.push(next.clone());
                Track { clips, ..track }
            })
            .collect(),
        ..project
    }
}

pub fn add_imported_video(
    project: Project,
    media: MediaItem,
    video_clip_id: String,
    audio_clip_id: String,
) -> Project {
    let mut next = add_to_library(project, media.clone());
    let needs_video = !next.tracks.iter().any(|t| t.kind == TrackKind::Video);
    if needs_video {
        let mut tracks = vec![Track {
            id: "v1".into(),
            name: "视频1".into(),
            kind: TrackKind::Video,
            muted: false,
            locked: false,
            clips: vec![],
        }];
        tracks.extend(next.tracks);
        next.tracks = tracks;
    }
    let video_track_id = next
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .map(|t| t.id.clone())
        .unwrap_or_else(|| "v1".into());
    let start = next
        .tracks
        .iter()
        .find(|t| t.id == video_track_id)
        .map(|t| {
            t.clips
                .iter()
                .fold(0.0_f64, |end, c| end.max(c.start + c.duration))
        })
        .unwrap_or(0.0);
    let mut video_clip = normalize_clip(
        video_clip_id,
        media.id.clone(),
        start,
        media.duration,
        0.0,
        1.0,
    );
    if let (Some(w), Some(h)) = (media.width, media.height) {
        video_clip.transform =
            crate::Transform::for_media_placement((w, h), next.output_size(), next.media_scale);
    }
    next = add_clip(next, &video_track_id, video_clip.clone());
    if media.has_audio.unwrap_or(false) {
        if !next.tracks.iter().any(|t| t.id == "a1") {
            next.tracks.push(Track {
                id: "a1".into(),
                name: "对白".into(),
                kind: TrackKind::Audio,
                muted: false,
                locked: false,
                clips: vec![],
            });
        }
        let audio_clip = Clip {
            id: audio_clip_id,
            ..video_clip.clone()
        };
        next = add_clip(next, "a1", audio_clip);
    }
    next.playhead = start.min(crate::project_duration(&next));
    next
}
