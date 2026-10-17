use crate::model::track::TrackKind;
use crate::model::Project;

/// Remove a timeline track and its clips. Media library entries are kept.
///
/// No-op when the track is missing, locked, or would leave the project with
/// no video or no audio tracks.
pub fn delete_track(project: Project, track_id: &str) -> Project {
    if project.tracks.len() <= 1 {
        return project;
    }
    let Some(track) = project.tracks.iter().find(|track| track.id == track_id) else {
        return project;
    };
    if track.locked
        || (matches!(track.kind, TrackKind::Video | TrackKind::Audio)
            && project
                .tracks
                .iter()
                .filter(|other| other.kind == track.kind)
                .count()
                <= 1)
    {
        return project;
    }
    let tracks = project
        .tracks
        .into_iter()
        .filter(|track| track.id != track_id)
        .collect();
    Project { tracks, ..project }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{add_clip, add_track, create_project, normalize_clip, MediaItem, TrackKind};

    #[test]
    fn removes_track_keeps_media() {
        let mut project = add_track(
            create_project(None, None, None, None, None),
            TrackKind::Video,
        );
        project.media.push(MediaItem {
            id: "m1".into(),
            name: "a".into(),
            path: Some("a.mp4".into()),
            duration: 5.0,
            has_audio: Some(true),
            width: None,
            height: None,
            kind: Some(crate::MediaKind::Video),
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        project = add_clip(
            project,
            "v1",
            normalize_clip("c1".into(), "m1".into(), 0.0, 2.0, 0.0, 1.0),
        );
        assert_eq!(project.tracks[0].clips.len(), 1);

        let next = delete_track(project, "v1");
        assert_eq!(next.tracks.len(), 3);
        assert!(next.tracks.iter().all(|track| track.id != "v1"));
        assert_eq!(next.media.len(), 1);
        assert_eq!(next.media[0].id, "m1");
    }

    #[test]
    fn refuses_last_track_and_unknown_id() {
        let mut project = create_project(None, None, None, None, None);
        project.tracks.truncate(1);
        let only = project.tracks[0].id.clone();
        let same = delete_track(project.clone(), &only);
        assert_eq!(same.tracks.len(), 1);

        let untouched = delete_track(project, "nope");
        assert_eq!(untouched.tracks.len(), 1);
    }
}
