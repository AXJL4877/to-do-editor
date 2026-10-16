//! Media panel state; importing registers assets independently of sequence editing.
use super::{LibraryItemView, LibraryMediaView, LibraryViewModel};
use todo_core::Project;

#[derive(Default)]
pub(crate) struct LibraryState {
    pub selected: Option<String>,
    pub query: String,
}

impl LibraryState {
    pub fn view(&self, project: &Project) -> LibraryViewModel {
        let query = self.query.trim().to_lowercase();
        LibraryViewModel {
            selected_id: self.selected.clone(),
            query: self.query.clone(),
            items: project
                .media
                .iter()
                .filter(|item| {
                    item.name.to_lowercase().contains(&query)
                        || item
                            .path
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&query)
                })
                .map(|item| {
                    let uses = project
                        .tracks
                        .iter()
                        .flat_map(|t| &t.clips)
                        .filter(|c| c.media_id() == Some(item.id.as_str()))
                        .count();
                    LibraryItemView {
                        item: LibraryMediaView {
                            id: item.id.clone(),
                            name: item.name.clone(),
                            path: item.path.clone(),
                            duration: item.duration,
                            width: item.width,
                            height: item.height,
                            kind: todo_core::media_kind(item),
                            visual: todo_core::is_visual_media(item),
                            has_audio: item.has_audio == Some(true)
                                && !todo_core::is_image_media(item),
                            place_duration: todo_core::place_duration(item),
                        },
                        uses,
                        selected: self.selected.as_ref() == Some(&item.id),
                        locked_references: project.tracks.iter().any(|t| {
                            t.locked
                                && t.clips
                                    .iter()
                                    .any(|c| c.media_id() == Some(item.id.as_str()))
                        }),
                    }
                })
                .collect(),
        }
    }
}
