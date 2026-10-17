use crate::productions::ProductionOutput;
use crate::Ticket;
use todo_core::{MediaItem, Project, TransformBounds};

#[derive(Debug, Clone)]
pub enum ProjectOutput {
    Listed(Vec<String>),
    Opened { name: String, project: Project },
    Saved(Project),
    Deleted,
}

#[derive(Debug, Clone)]
pub enum MediaOutput {
    Imported(MediaItem),
    ImportFinished,
    ImportAborted {
        message: String,
    },
    Peaks {
        path: String,
        duration: f64,
        peaks: Vec<f64>,
    },
    Thumbnail {
        path: String,
        bytes: Vec<u8>,
    },
    TextRaster {
        path: String,
        /// Laid-out text block as canvas fractions, before the clip transform.
        block: TransformBounds,
    },
    /// Not rendered because a newer frame of the same text was queued.
    TextRasterSkipped,
    Adopted(Vec<MediaItem>),
    Exported {
        output: std::path::PathBuf,
        report: std::path::PathBuf,
        frames: usize,
        failed_checks: Vec<String>,
    },
    ExportProgress {
        done: usize,
        total: usize,
    },
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Project {
        ticket: Ticket,
        result: Result<ProjectOutput, String>,
    },
    Media {
        ticket: Ticket,
        result: Result<MediaOutput, String>,
    },
    Production(ProductionOutput),
}
