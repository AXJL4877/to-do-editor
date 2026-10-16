use crate::Project;
use std::collections::HashSet;

/// Existing selected IDs first, then their linked partners in stable track order.
pub fn linked_clip_ids(project: &Project, clip_ids: &[String]) -> Vec<String> {
    let all: Vec<_> = project.tracks.iter().flat_map(|t| &t.clips).collect();
    let selected: HashSet<&str> = clip_ids.iter().map(String::as_str).collect();
    let links: HashSet<&str> = all
        .iter()
        .filter(|c| selected.contains(c.id.as_str()))
        .filter_map(|c| c.link_id.as_deref())
        .collect();
    let mut ids = vec![];
    for id in clip_ids {
        if all.iter().any(|c| c.id == *id) && !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    for clip in all {
        if clip
            .link_id
            .as_deref()
            .is_some_and(|link| links.contains(link))
            && !ids.contains(&clip.id)
        {
            ids.push(clip.id.clone());
        }
    }
    ids
}
