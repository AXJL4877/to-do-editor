//! Media panel state; importing registers assets independently of sequence editing.
use super::{
    ExplorerNode, FolderNodeView, LibraryEntryView, LibraryFolderView, LibraryItemView,
    LibraryMediaView, LibraryViewModel,
};
use std::collections::HashSet;
use todo_core::{MediaFolder, Project};

#[derive(Default)]
pub(crate) struct LibraryState {
    pub selected: Option<String>,
    /// `None` = library root.
    pub selected_folder: Option<String>,
    pub query: String,
    pub expanded: HashSet<String>,
}

impl LibraryState {
    pub fn view(&self, project: &Project) -> LibraryViewModel {
        let query = self.query.trim().to_lowercase();
        let searching = !query.is_empty();
        let selected_folder = self.resolved_folder(project);
        let tree = self.build_folder_tree(project, None, 0);
        let explorer = if searching {
            self.search_explorer(project, &query)
        } else {
            self.build_explorer(project, None, 0)
        };
        let entries = if searching {
            self.search_entries(project, &query)
        } else {
            self.folder_entries(project, selected_folder.as_deref())
        };
        let items = if searching {
            entries
                .iter()
                .filter_map(|entry| match entry {
                    LibraryEntryView::Media(item) => Some(item.clone()),
                    LibraryEntryView::Folder(_) => None,
                })
                .collect()
        } else {
            flatten_explorer_media(&explorer)
        };
        LibraryViewModel {
            explorer,
            tree,
            entries,
            items,
            selected_id: self.selected.clone(),
            selected_folder_id: selected_folder,
            query: self.query.clone(),
            searching,
        }
    }

    fn resolved_folder(&self, project: &Project) -> Option<String> {
        self.selected_folder.as_ref().and_then(|id| {
            project
                .folders
                .iter()
                .any(|folder| folder.id == *id)
                .then(|| id.clone())
        })
    }

    fn build_explorer(
        &self,
        project: &Project,
        parent_id: Option<&str>,
        depth: usize,
    ) -> Vec<ExplorerNode> {
        let mut folders: Vec<_> = project
            .folders
            .iter()
            .filter(|folder| folder.parent_id.as_deref() == parent_id)
            .collect();
        folders.sort_by_key(|folder| folder.name.to_lowercase());
        let mut nodes = Vec::new();
        for folder in folders {
            let child_folders = project
                .folders
                .iter()
                .any(|child| child.parent_id.as_deref() == Some(folder.id.as_str()));
            let child_media = project
                .media
                .iter()
                .any(|item| item.folder_id.as_deref() == Some(folder.id.as_str()));
            let expanded = self.expanded.contains(&folder.id);
            let mut children = Vec::new();
            if expanded && (child_folders || child_media) {
                children = self.build_explorer(project, Some(folder.id.as_str()), depth + 1);
            }
            nodes.push(ExplorerNode::Folder {
                id: folder.id.clone(),
                name: folder.name.clone(),
                depth,
                expanded,
                selected: self.selected.is_none()
                    && self.selected_folder.as_deref() == Some(folder.id.as_str()),
                children,
            });
        }
        let mut media: Vec<_> = project
            .media
            .iter()
            .filter(|item| item.folder_id.as_deref() == parent_id)
            .collect();
        media.sort_by_key(|item| item.name.to_lowercase());
        nodes.extend(media.into_iter().map(|item| ExplorerNode::Media {
            depth,
            item: media_item_view(project, item, self.selected.as_deref()),
        }));
        nodes
    }

    fn search_explorer(&self, project: &Project, query: &str) -> Vec<ExplorerNode> {
        self.search_entries(project, query)
            .into_iter()
            .filter_map(|entry| match entry {
                LibraryEntryView::Media(item) => Some(ExplorerNode::Media { depth: 0, item }),
                LibraryEntryView::Folder(_) => None,
            })
            .collect()
    }

    fn build_folder_tree(
        &self,
        project: &Project,
        parent_id: Option<&str>,
        depth: usize,
    ) -> Vec<FolderNodeView> {
        let mut children: Vec<_> = project
            .folders
            .iter()
            .filter(|folder| folder.parent_id.as_deref() == parent_id)
            .collect();
        children.sort_by_key(|folder| folder.name.to_lowercase());
        let mut nodes = Vec::new();
        for folder in children {
            let has_children = project
                .folders
                .iter()
                .any(|child| child.parent_id.as_deref() == Some(folder.id.as_str()))
                || project
                    .media
                    .iter()
                    .any(|item| item.folder_id.as_deref() == Some(folder.id.as_str()));
            let expanded = self.expanded.contains(&folder.id);
            let mut node = FolderNodeView {
                id: folder.id.clone(),
                name: folder.name.clone(),
                depth,
                expanded,
                has_children,
                selected: self.selected.is_none()
                    && self.selected_folder.as_deref() == Some(folder.id.as_str()),
                children: Vec::new(),
            };
            if expanded {
                node.children =
                    self.build_folder_tree(project, Some(folder.id.as_str()), depth + 1);
            }
            nodes.push(node);
        }
        nodes
    }

    fn folder_entries(&self, project: &Project, folder_id: Option<&str>) -> Vec<LibraryEntryView> {
        let mut folders: Vec<_> = project
            .folders
            .iter()
            .filter(|folder| folder.parent_id.as_deref() == folder_id)
            .collect();
        folders.sort_by_key(|folder| folder.name.to_lowercase());
        let mut entries: Vec<_> = folders
            .into_iter()
            .map(|folder| LibraryEntryView::Folder(folder_view(project, folder)))
            .collect();
        let mut media: Vec<_> = project
            .media
            .iter()
            .filter(|item| item.folder_id.as_deref() == folder_id)
            .collect();
        media.sort_by_key(|item| item.name.to_lowercase());
        entries.extend(media.into_iter().map(|item| {
            LibraryEntryView::Media(media_item_view(project, item, self.selected.as_deref()))
        }));
        entries
    }

    fn search_entries(&self, project: &Project, query: &str) -> Vec<LibraryEntryView> {
        let mut media: Vec<_> = project
            .media
            .iter()
            .filter(|item| {
                item.name.to_lowercase().contains(query)
                    || item
                        .path
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(query)
            })
            .collect();
        media.sort_by_key(|item| item.name.to_lowercase());
        media
            .into_iter()
            .map(|item| {
                let mut view = media_item_view(project, item, self.selected.as_deref());
                view.folder_path = folder_path(project, item.folder_id.as_deref());
                LibraryEntryView::Media(view)
            })
            .collect()
    }
}

fn flatten_explorer_media(nodes: &[ExplorerNode]) -> Vec<LibraryItemView> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            ExplorerNode::Folder { children, .. } => out.extend(flatten_explorer_media(children)),
            ExplorerNode::Media { item, .. } => out.push(item.clone()),
        }
    }
    out
}

fn folder_view(project: &Project, folder: &MediaFolder) -> LibraryFolderView {
    let child_folders = project
        .folders
        .iter()
        .filter(|child| child.parent_id.as_deref() == Some(folder.id.as_str()))
        .count();
    let child_media = project
        .media
        .iter()
        .filter(|item| item.folder_id.as_deref() == Some(folder.id.as_str()))
        .count();
    LibraryFolderView {
        id: folder.id.clone(),
        name: folder.name.clone(),
        child_count: child_folders + child_media,
    }
}

fn media_item_view(
    project: &Project,
    item: &todo_core::MediaItem,
    selected: Option<&str>,
) -> LibraryItemView {
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
            has_audio: item.has_audio == Some(true) && !todo_core::is_image_media(item),
            place_duration: todo_core::place_duration(item),
        },
        uses,
        selected: selected == Some(item.id.as_str()),
        locked_references: project.tracks.iter().any(|t| {
            t.locked
                && t.clips
                    .iter()
                    .any(|c| c.media_id() == Some(item.id.as_str()))
        }),
        folder_path: None,
    }
}

fn folder_path(project: &Project, folder_id: Option<&str>) -> Option<String> {
    let mut id = folder_id?;
    let mut parts = Vec::new();
    let mut guard = 0;
    while guard <= project.folders.len() {
        let folder = project.folders.iter().find(|folder| folder.id == id)?;
        parts.push(folder.name.clone());
        match folder.parent_id.as_deref() {
            Some(parent) => id = parent,
            None => break,
        }
        guard += 1;
    }
    parts.reverse();
    Some(parts.join(" / "))
}
