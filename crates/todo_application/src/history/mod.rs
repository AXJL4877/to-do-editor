//! Bounded project history. Playback and selection do not create edit entries.
use todo_core::Project;

#[derive(Default)]
pub(crate) struct History {
    past: Vec<Project>,
    future: Vec<Project>,
}

pub(crate) fn same_content(a: &Project, b: &Project) -> bool {
    a.version == b.version
        && a.fps == b.fps
        && a.width == b.width
        && a.height == b.height
        && a.duration == b.duration
        && a.media == b.media
        && a.tracks == b.tracks
}

impl History {
    pub fn record(&mut self, before: Project, after: &Project) {
        if same_content(&before, after) {
            return;
        }
        if self.past.len() == 100 {
            self.past.remove(0);
        }
        self.past.push(before);
        self.future.clear();
    }
    pub fn undo(&mut self, current: &Project) -> Option<Project> {
        let previous = self.past.pop()?;
        self.future.push(current.clone());
        Some(previous)
    }
    pub fn redo(&mut self, current: &Project) -> Option<Project> {
        let next = self.future.pop()?;
        self.past.push(current.clone());
        Some(next)
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}
