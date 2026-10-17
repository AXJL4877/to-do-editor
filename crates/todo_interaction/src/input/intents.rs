use todo_application::CloseChoice;
use todo_core::{ClipTransform, MediaScaleMode, TextContent, TrackKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorFocus {
    #[default]
    Timeline,
    Library,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimelineTool {
    #[default]
    Select,
    Razor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Escape,
    C,
    V,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPointerKind {
    Down,
    Move,
    Up,
}

/// Toolkit-neutral user input. Host clocks and I/O results are application events.
#[derive(Debug, Clone)]
pub enum Intent {
    Focus(EditorFocus),
    SelectClip {
        clip_id: String,
        additive: bool,
    },
    SelectAll,
    ClearSelection,
    DeleteSelection {
        ripple: bool,
    },
    AddTrack {
        kind: TrackKind,
    },
    InsertTrackBelow {
        track_id: String,
    },
    /// Swap with the adjacent same-kind neighbor (`up` = toward timeline top).
    MoveTrack {
        track_id: String,
        up: bool,
    },
    /// Pack non-overlapping clips into fewer compatible tracks.
    CompactTracks,
    ToggleLock {
        track_id: String,
    },
    ToggleLinkedSelection,
    ToggleSnapping,
    MoveSelectionToTrack {
        track_id: String,
    },
    LibrarySelect(String),
    LibraryQuery(String),
    /// `None` selects the library root.
    LibrarySelectFolder(Option<String>),
    LibraryToggleFolder(String),
    LibraryCreateFolder {
        name: String,
    },
    LibraryRenameFolder {
        id: String,
        name: String,
    },
    LibraryDeleteFolder {
        id: String,
    },
    LibraryMoveFolder {
        id: String,
        parent_id: Option<String>,
    },
    LibraryMoveMedia {
        media_id: String,
        folder_id: Option<String>,
    },
    /// Open library media in the source monitor (double-click).
    OpenSourceMonitor {
        media_id: String,
    },
    ShowProgramMonitor,
    ShowSourceMonitor,
    PlaceMedia {
        media_id: String,
        track_id: Option<String>,
        time: Option<f64>,
    },
    RemoveMedia {
        media_id: String,
        cascade: bool,
    },
    MediaDragHover {
        media_id: String,
        track_id: String,
        time: f64,
    },
    MediaDragEnd,
    Undo,
    Redo,
    Copy,
    Paste,
    BoardPointerDown {
        view_x: f64,
        scroll_left: f64,
    },
    LanePointerDown {
        track_id: String,
        view_x: f64,
        scroll_left: f64,
    },
    MarqueePointerDown {
        view_x: f64,
        track_index: usize,
        scroll_left: f64,
        additive: bool,
    },
    ClipPointerDown {
        clip_id: String,
        view_x: f64,
        scroll_left: f64,
        local_x: f64,
        width_px: f64,
        alt: bool,
    },
    PointerMove {
        view_x: f64,
        scroll_left: f64,
        track_index: Option<usize>,
    },
    PointerUp,
    ToolClicked(TimelineTool),
    RazorAtPlayhead,
    ToggleMute {
        track_id: String,
    },
    DeleteTrack {
        track_id: String,
    },
    Key {
        code: KeyCode,
        typing: bool,
    },
    Wheel {
        delta_y: f64,
        view_x: f64,
        scroll_left: f64,
        alt: bool,
    },
    Viewport {
        scroll_left: f64,
        width: f64,
    },
    TogglePlay,
    Seek(f64),
    PreviewViewport {
        width: f64,
        height: f64,
    },
    PreviewZoom(f64),
    PreviewWheel {
        delta: f64,
        x: f64,
        y: f64,
    },
    PreviewPan {
        kind: PreviewPointerKind,
        x: f64,
        y: f64,
    },
    PreviewFit,
    PreviewOutputOnly(bool),
    PreviewSourceAspect {
        path: String,
        aspect: f64,
    },
    PreviewPointer {
        kind: PreviewPointerKind,
        x: f64,
        y: f64,
    },
    BeginEdit,
    CommitEdit,
    SetClipDuration {
        clip_id: String,
        seconds: f64,
    },
    SetClipTransform {
        clip_id: String,
        transform: ClipTransform,
    },
    ResetTransform {
        clip_id: String,
    },
    /// Add default styled text at the playhead on a free text lane.
    AddText,
    SetClipText {
        clip_id: String,
        content: TextContent,
    },
    SetProjectDuration {
        seconds: f64,
    },
    SetProjectOutput {
        width: u32,
        height: u32,
    },
    SetProjectFps {
        fps: f64,
    },
    SetMediaScaleMode {
        mode: MediaScaleMode,
    },
    ImportPressed,
    Save,
    Back,
    RefreshProjects,
    CreateProject {
        name: String,
    },
    OpenProject {
        name: String,
    },
    DeleteProject {
        name: String,
    },
    ChooseProjectFile,
    ConfirmClose(CloseChoice),
    RequestClose,
}
