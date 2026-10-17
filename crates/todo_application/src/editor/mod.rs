mod commands;
mod imports;
mod preview;
pub use preview::picture_layers;

use crate::clipboard::Clipboard;
use crate::history::{same_content, History};
use crate::text_rasters::TextRasters;
use todo_core::{
    ensure_av_tracks, project_duration, ClipTransform, DomainError, Project, ResolvedAudioClip,
    DEFAULT_WORKSPACE_SECS,
};

pub const CLOCK_POLL_SECS: f64 = 0.05;
pub const CLOCK_FRESH_SECS: f64 = 1.0;
pub const CLOCK_MISSING_SECS: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioClockSample {
    pub time: f64,
    pub playing: bool,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Domain(DomainError),
    External {
        operation: &'static str,
        message: String,
    },
    Copied,
    WaitingImport,
    Saving,
}

/// One composited preview layer after domain resolve (bottom → top in a stack).
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewLayer {
    pub path: String,
    pub media_time: f64,
    pub aspect: Option<(u32, u32)>,
    pub kind: PreviewLayerKind,
    pub transform: ClipTransform,
    /// Reveal layers: `media_time` is the progress 0..1 and this the soft
    /// edge as a fraction of the duration.
    pub soft: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewLayerKind {
    #[default]
    Video,
    Image,
    Packed,
    Reveal,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreviewCommand {
    /// Clear or legacy single-source open. Prefer [`PreviewCommand::Layers`].
    Path(Option<String>),
    Seek(f64),
    Aspect {
        width: u32,
        height: u32,
    },
    Transform(ClipTransform),
    /// Full visual stack at the playhead, bottom → top. Empty clears the picture.
    Layers(Vec<PreviewLayer>),
    Audio(Vec<ResolvedAudioClip>),
    Transport {
        time: f64,
        playing: bool,
    },
    Quit,
}

#[derive(Debug, Clone, Default)]
pub struct EditorUpdate {
    pub commands: Vec<PreviewCommand>,
    pub new_clips: Vec<String>,
    pub content_changed: bool,
    /// Failure from this command only; presentation status may outlive a command.
    pub error: Option<DomainError>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NativeClock {
    pub time: f64,
    pub playing: bool,
    pub received_at: f64,
}

pub(crate) struct Want {
    pub audio: bool,
    pub transport: bool,
    pub picture: bool,
}

#[derive(Debug, Clone)]
pub struct Picture {
    pub active: bool,
    pub clip_id: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
    pub aspect: Option<(u32, u32)>,
    pub media_time: f64,
    pub transform: ClipTransform,
}

/// Which monitor pane the preview host is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonitorPane {
    #[default]
    Program,
    Source,
}

/// Library media loaded into the source monitor (independent of the playhead).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceMonitor {
    pub media_id: String,
    pub time: f64,
}

pub struct EditorSession {
    pub(crate) history: History,
    pub(crate) gesture_before: Option<Project>,
    pub(crate) saved_project: Project,
    pub(crate) clipboard: Clipboard,
    pub(crate) status: Option<Status>,
    pub(crate) project: Project,
    pub(crate) revision: u64,
    pub(crate) project_name: String,
    pub(crate) playing: bool,
    pub(crate) playhead: f64,
    pub(crate) importing: bool,
    pub(crate) monitor_pane: MonitorPane,
    pub(crate) source_monitor: Option<SourceMonitor>,
    pub(crate) last_tick: Option<f64>,
    pub(crate) clock: Option<NativeClock>,
    pub(crate) audio_unavailable: bool,
    pub(crate) clock_missing_at: Option<f64>,
    pub(crate) clock_epoch: u64,
    pub(crate) synced_playhead: f64,
    pub(crate) last_audio: Option<Vec<ResolvedAudioClip>>,
    pub(crate) last_path: Option<Option<String>>,
    pub(crate) last_seek_ms: Option<i64>,
    pub(crate) last_aspect: Option<(u32, u32)>,
    pub(crate) last_transform: Option<ClipTransform>,
    pub(crate) last_layers: Option<Vec<PreviewLayer>>,
    pub(crate) last_transport: Option<(i64, bool)>,
    pub(crate) text_rasters: TextRasters,
}

impl EditorSession {
    pub fn new(project: Project, project_name: impl Into<String>) -> Self {
        let mut project = ensure_av_tracks(project);
        if project
            .duration
            .filter(|d| d.is_finite() && *d > 0.0)
            .is_none()
        {
            project.duration = Some(DEFAULT_WORKSPACE_SECS);
        }
        let playhead = project.playhead.max(0.0).min(project_duration(&project));
        project.playhead = playhead;
        Self {
            history: History::default(),
            gesture_before: None,
            saved_project: project.clone(),
            clipboard: Clipboard::default(),
            status: None,
            project,
            revision: 0,
            project_name: project_name.into(),
            playing: false,
            playhead,
            importing: false,
            monitor_pane: MonitorPane::Program,
            source_monitor: None,
            last_tick: None,
            clock: None,
            audio_unavailable: false,
            clock_missing_at: None,
            clock_epoch: 1,
            synced_playhead: playhead,
            last_audio: None,
            last_path: None,
            last_seek_ms: None,
            last_aspect: None,
            last_transform: None,
            last_layers: None,
            last_transport: None,
            text_rasters: TextRasters::default(),
        }
    }
    pub fn project(&self) -> &Project {
        &self.project
    }
    /// Monotonic content revision for this editor session, independent of document version.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn project_name(&self) -> &str {
        &self.project_name
    }
    pub fn playhead(&self) -> f64 {
        self.playhead
    }
    pub fn playing(&self) -> bool {
        self.playing
    }
    pub fn importing(&self) -> bool {
        self.importing
    }
    pub fn clock_epoch(&self) -> u64 {
        self.clock_epoch
    }
    pub fn dirty(&self) -> bool {
        !same_content(&self.project, &self.saved_project)
    }
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    pub fn can_paste(&self) -> bool {
        self.clipboard.available()
    }
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
    pub fn transaction_active(&self) -> bool {
        self.gesture_before.is_some()
    }
    pub fn edit_base(&self) -> &Project {
        self.gesture_before.as_ref().unwrap_or(&self.project)
    }
    pub fn notice(&mut self, status: Status) {
        self.status = Some(status);
    }
    pub fn snapshot(&mut self) -> Project {
        self.project.playhead = self.playhead;
        self.project.clone()
    }
    pub fn can_play(&self) -> bool {
        self.can_play_at(self.picture().active) && !self.importing
    }
    pub fn monitor_pane(&self) -> MonitorPane {
        self.monitor_pane
    }
    pub fn source_monitor(&self) -> Option<&SourceMonitor> {
        self.source_monitor.as_ref()
    }
    pub(crate) fn showing_source(&self) -> bool {
        self.monitor_pane == MonitorPane::Source && self.source_monitor.is_some()
    }
}

pub(crate) fn playhead_changed(next: f64, previous: f64) -> bool {
    (next - previous).abs() > 1e-4
}
