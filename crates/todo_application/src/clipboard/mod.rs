use todo_core::{Clip, CopiedClip, Project};

#[derive(Default)]
pub(crate) struct Clipboard {
    clips: Vec<CopiedClip>,
}
impl Clipboard {
    pub fn capture(&mut self, project: &Project, ids: &[String]) {
        self.clips = project
            .tracks
            .iter()
            .flat_map(|track| {
                track
                    .clips
                    .iter()
                    .filter(|clip| ids.contains(&clip.id))
                    .map(|clip| CopiedClip {
                        track_id: track.id.clone(),
                        kind: track.kind,
                        clip: Clip::clone(clip),
                    })
            })
            .collect();
    }
    pub fn available(&self) -> bool {
        !self.clips.is_empty()
    }
    pub fn clips(&self) -> Vec<CopiedClip> {
        self.clips.clone()
    }
}
