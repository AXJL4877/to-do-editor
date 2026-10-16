use crate::{AssetRequest, AudioClockSample};
use std::path::PathBuf;
use todo_core::{EditCommand, MediaItem, Project};

#[derive(Debug, Clone)]
pub enum EditorCommand {
    Edit(EditCommand),
    BeginEdit,
    CommitEdit,
    CancelEdit,
    Undo,
    Redo,
    Copy {
        clip_ids: Vec<String>,
    },
    Paste,
    RemoveMedia {
        media_id: String,
        cascade: bool,
    },
    Seek(f64),
    TogglePlay,
    Frame {
        now: f64,
    },
    Clock {
        epoch: u64,
        now: f64,
        sample: Option<AudioClockSample>,
    },
    ClockFailed {
        epoch: u64,
    },
    RefreshPicture,
    ImportStarted,
    Imported(MediaItem),
    /// Register a probed item in the library without placing it on the timeline.
    RegisterMedia(MediaItem),
    ImportFinished,
    Saved(Project),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

#[derive(Debug, Clone)]
pub enum AppAction {
    RefreshProjects,
    CreateProject(String),
    OpenProject(String),
    DeleteProject(String),
    ChooseProjectFile,
    ImportProjectFile(PathBuf),
    ChooseImport,
    ImportFiles(Vec<PathBuf>),
    Save,
    Back,
    RequestClose,
    ConfirmClose(CloseChoice),
    Editor(EditorCommand),
    RequestAssets {
        now: f64,
        requests: Vec<AssetRequest>,
    },
    Notice(String),
}
