use super::{
    place_media::unique_id,
    razor::{minimum_piece, split_clip},
};
use crate::{linked_clip_ids, Project};
use std::collections::{HashMap, HashSet};

pub fn razor_clips(project: Project, time: f64, clip_ids: &[String]) -> Project {
    if !time.is_finite() {
        return project;
    }
    let ids: HashSet<&str> = clip_ids.iter().map(String::as_str).collect();
    let linked = linked_clip_ids(&project, clip_ids);
    if project
        .tracks
        .iter()
        .any(|t| t.locked && t.clips.iter().any(|c| linked.contains(&c.id)))
    {
        return project;
    }
    let mut next = project.clone();
    let minimum = minimum_piece(project.fps);
    let mut links = HashMap::<String, (String, String)>::new();
    for track_index in 0..next.tracks.len() {
        let mut clips = vec![];
        for clip in next.tracks[track_index].clips.clone() {
            if !ids.contains(clip.id.as_str()) {
                clips.push(clip);
                continue;
            }
            let Some((mut left, mut right)) = split_clip(&clip, time, unique_id(&next), minimum)
            else {
                clips.push(clip);
                continue;
            };
            if let Some(link) = &clip.link_id {
                let pair = links
                    .entry(link.clone())
                    .or_insert_with(|| (unique_id(&next), unique_id(&next)));
                left.link_id = Some(pair.0.clone());
                right.link_id = Some(pair.1.clone());
            }
            clips.push(left);
            clips.push(right);
        }
        next.tracks[track_index].clips = clips;
    }
    next
}
