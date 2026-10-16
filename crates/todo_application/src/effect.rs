use crate::{PreviewCommand, TextRaster};
use std::path::PathBuf;
use todo_core::Project;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket {
    pub id: u64,
    pub session: u64,
}

#[derive(Debug, Clone)]
pub enum ProjectTask {
    List,
    Create { name: String },
    Load { name: String },
    Save { name: String, project: Project },
    Delete { name: String },
    Import { name: String, path: PathBuf },
}

#[derive(Debug, Clone)]
pub enum MediaTask {
    Import {
        project: String,
        paths: Vec<PathBuf>,
    },
    Peaks {
        path: String,
        bins: u32,
    },
    Thumbnail {
        path: String,
        width: u32,
    },
    /// Render a text image; the result is [`crate::MediaOutput::TextRaster`].
    TextRaster(Box<TextRaster>),
}

#[derive(Debug, Clone)]
pub enum Effect {
    Project { ticket: Ticket, task: ProjectTask },
    Media { ticket: Ticket, task: MediaTask },
    Preview(Vec<PreviewCommand>),
    ChooseMediaFiles,
    ChooseProjectFile,
    UploadThumbnail { path: String, bytes: Vec<u8> },
    ResetPresentation,
    CloseWindow,
}
