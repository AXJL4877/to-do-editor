mod control;
mod dispatch;
mod events;
mod lifecycle;
mod productions;

use crate::media_jobs::MediaJobs;
use crate::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Projects,
    Editor,
}

#[derive(Debug, Clone, Default)]
pub struct AppUpdate {
    pub effects: Vec<Effect>,
    pub new_clips: Vec<String>,
}
impl AppUpdate {
    fn effects(effects: Vec<Effect>) -> Self {
        Self {
            effects,
            new_clips: Vec::new(),
        }
    }
    fn editor(update: EditorUpdate) -> Self {
        let effects = if update.commands.is_empty() {
            Vec::new()
        } else {
            vec![Effect::Preview(update.commands)]
        };
        Self {
            effects,
            new_clips: update.new_clips,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AfterSave {
    Stay,
    Back,
    Close,
}

pub struct Workspace {
    route: Route,
    projects: Vec<String>,
    editor: Option<EditorSession>,
    error: Option<Status>,
    generation: u64,
    next_id: u64,
    pending_open: Option<Ticket>,
    pending_list: Option<Ticket>,
    pending_delete: Option<Ticket>,
    saves: HashMap<Ticket, AfterSave>,
    jobs: MediaJobs,
    peaks: PeaksCache,
    close_prompt: bool,
    now: f64,
    control: crate::control::state::ControlState,
    /// In-flight text raster jobs by ticket → raster digest.
    raster_jobs: HashMap<Ticket, String>,
    /// Virtual folder that receives the in-flight import (`None` = library root).
    import_folder_id: Option<String>,
    productions: productions::Productions,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            route: Route::Projects,
            projects: Vec::new(),
            editor: None,
            error: None,
            generation: 1,
            next_id: 0,
            pending_open: None,
            pending_list: None,
            pending_delete: None,
            saves: HashMap::new(),
            jobs: MediaJobs::default(),
            peaks: PeaksCache::default(),
            close_prompt: false,
            now: 0.0,
            control: Default::default(),
            raster_jobs: HashMap::new(),
            import_folder_id: None,
            productions: Default::default(),
        }
    }
}

impl Workspace {
    /// Starts an in-memory workspace, useful for embedding and pure interaction tests.
    pub fn with_project(project: Project, name: impl Into<String>) -> Self {
        Self {
            route: Route::Editor,
            editor: Some(EditorSession::new(project, name)),
            ..Self::default()
        }
    }
    pub fn route(&self) -> Route {
        self.route
    }
    pub fn projects(&self) -> &[String] {
        &self.projects
    }
    pub fn editor(&self) -> Option<&EditorSession> {
        self.editor.as_ref()
    }
    pub fn editor_mut(&mut self) -> Option<&mut EditorSession> {
        self.editor.as_mut()
    }
    pub fn error(&self) -> Option<&Status> {
        self.error.as_ref()
    }
    pub fn close_prompt(&self) -> bool {
        self.close_prompt
    }
    pub fn busy(&self) -> bool {
        self.jobs.busy()
            || !self.saves.is_empty()
            || self.pending_open.is_some()
            || !self.raster_jobs.is_empty()
    }
    pub fn saving(&self) -> bool {
        !self.saves.is_empty()
    }
    pub fn peaks(&self) -> &PeaksCache {
        &self.peaks
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Allocate a ticket for an adapter-created media task.
    pub fn media_ticket(&mut self) -> Ticket {
        self.ticket()
    }
    /// Target folder for the next / current media import (`None` = root).
    pub fn set_import_folder(&mut self, folder_id: Option<String>) {
        self.import_folder_id = folder_id;
    }
    fn ticket(&mut self) -> Ticket {
        self.next_id = self.next_id.wrapping_add(1);
        Ticket {
            id: self.next_id,
            session: self.generation,
        }
    }
    /// Re-send the whole preview state, e.g. after the preview host restarted.
    pub fn resync_preview(&mut self) -> AppUpdate {
        let Some(editor) = self.editor.as_mut() else {
            return AppUpdate::default();
        };
        let commands = editor.sync();
        self.editor_update(EditorUpdate {
            commands,
            ..EditorUpdate::default()
        })
    }
    /// Editor output plus worker jobs for any text rasters the editor asked for.
    fn editor_update(&mut self, update: EditorUpdate) -> AppUpdate {
        let mut app = AppUpdate::editor(update);
        let rasters = self
            .editor
            .as_mut()
            .map(EditorSession::take_text_rasters)
            .unwrap_or_default();
        for raster in rasters {
            let ticket = self.ticket();
            self.raster_jobs.insert(ticket, raster.digest.clone());
            app.effects.push(Effect::Media {
                ticket,
                task: MediaTask::TextRaster(Box::new(raster)),
            });
        }
        app
    }
    fn problem(&mut self, operation: &'static str, message: String) {
        let status = Status::External { operation, message };
        if let Some(editor) = &mut self.editor {
            editor.notice(status.clone());
        }
        self.error = Some(status);
    }
}
