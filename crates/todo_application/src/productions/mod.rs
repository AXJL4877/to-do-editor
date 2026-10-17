//! AI productions: the use case that turns a script (or any workflow input)
//! into timeline clips, progressively and resumably. Pure state machines; the
//! adapter executes [`ProductionTask`]s and reports [`ProductionOutput`]s.
//!
//! - A narration recipe is orchestrated here: outline → voice → storyboard →
//!   images → bake (extension worker) → items. A workflow extension
//!   orchestrates itself and only streams items.
//! - Requests are content-addressed (`TaskKey`); a failed request is never
//!   resubmitted automatically (it may have been billed) until the user asks.
//! - Items reach the timeline through `todo_production::placement` as one
//!   batch edit per pass.
mod session;
#[cfg(test)]
mod tests;

pub use session::{ProductionSession, ProductionStatus, StatusNode, StatusState};

use serde_json::Value;
use todo_production::extension::items::Item;
use todo_production::extension::protocol::Canvas;
use todo_production::work::ProductionSpec;
use todo_production::{Production, TaskKey, TaskSpec};

pub type LocalRunId = u64;

/// What the extension package resolves to (the app reads the package).
#[derive(Debug, Clone, PartialEq)]
pub enum Recipe {
    Narration {
        spec: Box<ProductionSpec>,
        /// Worker operation that bakes one board; `None` places stills.
        bake: Option<String>,
        /// Worker operation that makes the background plate.
        plate: Option<String>,
    },
    Workflow {
        op: String,
        /// The run input (`script`, `media`), as the manifest declares.
        input: Value,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionSetup {
    pub extension: String,
    pub recipe: Recipe,
    pub canvas: Canvas,
    /// Repository-relative working directory of the production.
    pub workdir: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProductionTask {
    Submit {
        workdir: String,
        spec: TaskSpec,
    },
    Cancel(TaskKey),
    Retry(TaskKey),
    Run {
        run: LocalRunId,
        extension: String,
        op: String,
        input: Value,
        params: Value,
        canvas: Canvas,
        workdir: String,
        seed: u64,
    },
    CancelRun(LocalRunId),
    Save {
        project: String,
        doc: Box<Production>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pcm16 {
    pub samples: Vec<i16>,
    pub rate: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Generating,
    Downloading,
    Waiting(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProductionOutput {
    TaskState {
        key: TaskKey,
        state: TaskState,
    },
    /// `file` is relative to the submitting production's workdir; `pcm` is
    /// set for speech.
    TaskDone {
        key: TaskKey,
        output: Option<Value>,
        file: Option<String>,
        pcm: Option<Pcm16>,
    },
    TaskFailed {
        key: TaskKey,
        class: String,
        message: String,
        possibly_billed: bool,
    },
    RunProgress {
        run: LocalRunId,
        phase: String,
        fraction: f64,
    },
    RunItems {
        run: LocalRunId,
        items: Vec<Item>,
    },
    RunDone {
        run: LocalRunId,
        result: Value,
    },
    RunFailed {
        run: LocalRunId,
        class: String,
        message: String,
    },
    /// A provider needs the user (key or balance); shown once.
    Attention {
        provider: String,
        message: String,
    },
}
