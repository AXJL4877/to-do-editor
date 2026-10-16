use crate::model::clip::Clip;
use crate::model::track::Track;
use crate::model::Project;

use crate::query::find_clip;

pub(crate) fn map_tracks<F>(project: Project, map_track: F) -> Project
where
    F: FnMut(Track) -> Track,
{
    let mut next = project.clone();
    next.tracks = next.tracks.into_iter().map(map_track).collect();
    if crate::validate_project(&next).is_err() {
        return project;
    }
    next
}

pub(crate) fn patch_clip<F>(project: Project, clip_id: &str, update: F) -> Project
where
    F: FnOnce(Clip, String) -> Option<(Clip, String)>,
{
    let Some(found) = find_clip(&project, clip_id) else {
        return project;
    };
    if found.track.locked {
        return project;
    }
    let from_track = found.track_id.clone();
    let link_id = found.clip.link_id.clone();
    let Some((mut next_clip, next_track_id)) = update(found.clip, found.track_id) else {
        return map_tracks(project, |track| {
            if track.id == from_track {
                Track {
                    clips: track
                        .clips
                        .into_iter()
                        .filter(|c| c.id != clip_id)
                        .collect(),
                    ..track
                }
            } else {
                track
            }
        });
    };
    let Some(target) = project.tracks.iter().find(|t| t.id == next_track_id) else {
        return project;
    };
    if target.locked
        || project
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .any(|c| c.id == next_clip.id && c.id != clip_id)
    {
        return project;
    }
    if next_clip.link_id.is_none() {
        next_clip.link_id = link_id;
    }
    let source = project.tracks.iter().find(|t| t.id == from_track).unwrap();
    if !source.kind.same_family(target.kind)
        || crate::edit::validate_placement(&project, target, &next_clip, &[clip_id]).is_err()
    {
        return project;
    }

    if next_track_id == from_track {
        return map_tracks(project, |track| {
            if track.id != from_track {
                return track;
            }
            Track {
                clips: track
                    .clips
                    .into_iter()
                    .map(|c| {
                        if c.id == clip_id {
                            next_clip.clone()
                        } else {
                            c
                        }
                    })
                    .collect(),
                ..track
            }
        });
    }

    map_tracks(project, |track| {
        if track.id == from_track {
            return Track {
                clips: track
                    .clips
                    .into_iter()
                    .filter(|c| c.id != clip_id)
                    .collect(),
                ..track
            };
        }
        if track.id == next_track_id {
            if track.clips.iter().any(|c| c.id == next_clip.id) {
                return track;
            }
            let mut clips = track.clips;
            clips.push(next_clip.clone());
            return Track { clips, ..track };
        }
        track
    })
}
