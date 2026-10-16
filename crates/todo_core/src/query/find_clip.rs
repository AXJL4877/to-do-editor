use crate::{Clip, Project, Track};

pub struct FoundClip<'a> {
    pub track: &'a Track,
    pub track_id: String,
    pub clip: Clip,
    pub index: usize,
}

pub fn find_clip<'a>(project: &'a Project, clip_id: &str) -> Option<FoundClip<'a>> {
    for track in &project.tracks {
        if let Some(index) = track.clips.iter().position(|c| c.id == clip_id) {
            return Some(FoundClip {
                track,
                track_id: track.id.clone(),
                clip: track.clips[index].clone(),
                index,
            });
        }
    }
    None
}
