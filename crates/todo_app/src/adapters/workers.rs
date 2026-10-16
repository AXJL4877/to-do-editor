//! Blocking infrastructure runs outside the UI thread. Saves and text rasters
//! have their own queues so neither waits behind media tool jobs.
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use todo_application::{
    AppEvent, MediaOutput, MediaTask, ProjectOutput, ProjectTask, TextRaster, Ticket,
};
use todo_media::{MediaService, RasterRequest, TextRasterizer};
use todo_storage::ProjectStore;

pub struct Workers {
    projects: Sender<(Ticket, ProjectTask)>,
    imports: Sender<(Ticket, MediaTask)>,
    assets: Sender<(Ticket, MediaTask)>,
    text: Sender<(Ticket, TextRaster)>,
    events: Receiver<AppEvent>,
}
impl Workers {
    pub fn start(repo: PathBuf) -> Self {
        let (projects, project_rx) = mpsc::channel();
        let (imports, import_rx) = mpsc::channel();
        let (assets, asset_rx) = mpsc::channel();
        let (text, text_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let tx = events_tx.clone();
        let project_root = repo.clone();
        thread::spawn(move || {
            let store = ProjectStore::new(project_root);
            while let Ok((ticket, task)) = project_rx.recv() {
                let result = run_project(&store, task);
                if tx.send(AppEvent::Project { ticket, result }).is_err() {
                    break;
                }
            }
        });
        let rasterizer = TextRasterizer::new(&repo);
        let tx = events_tx.clone();
        thread::spawn(move || text_worker(rasterizer, text_rx, tx));
        let media = MediaService::new(repo);
        for rx in [import_rx, asset_rx] {
            let tx = events_tx.clone();
            let service = media.clone();
            thread::spawn(move || media_worker(service, rx, tx));
        }
        Self {
            projects,
            imports,
            assets,
            text,
            events,
        }
    }
    pub fn project(&self, ticket: Ticket, task: ProjectTask) -> Result<(), Box<AppEvent>> {
        self.projects.send((ticket, task)).map_err(|_| {
            Box::new(AppEvent::Project {
                ticket,
                result: Err("工程任务线程已停止".into()),
            })
        })
    }
    pub fn media(&self, ticket: Ticket, task: MediaTask) -> Result<(), Box<AppEvent>> {
        if let MediaTask::TextRaster(raster) = task {
            return self.text.send((ticket, *raster)).map_err(|_| {
                Box::new(AppEvent::Media {
                    ticket,
                    result: Err("文字渲染线程已停止".into()),
                })
            });
        }
        let importing = matches!(&task, MediaTask::Import { .. });
        let queue = if importing {
            &self.imports
        } else {
            &self.assets
        };
        queue.send((ticket, task)).map_err(|_| {
            Box::new(AppEvent::Media {
                ticket,
                result: if importing {
                    Ok(MediaOutput::ImportAborted {
                        message: "媒体任务线程已停止".into(),
                    })
                } else {
                    Err("媒体任务线程已停止".into())
                },
            })
        })
    }
    pub fn poll(&self) -> Option<AppEvent> {
        self.events.try_recv().ok()
    }
}

fn run_project(store: &ProjectStore, task: ProjectTask) -> Result<ProjectOutput, String> {
    match task {
        ProjectTask::List => store
            .list()
            .map(ProjectOutput::Listed)
            .map_err(|e| e.to_string()),
        ProjectTask::Create { name } => store
            .create(&name)
            .map(|project| ProjectOutput::Opened { name, project })
            .map_err(|e| e.to_string()),
        ProjectTask::Load { name } => store
            .load(&name)
            .map(|project| ProjectOutput::Opened { name, project })
            .map_err(|e| e.to_string()),
        ProjectTask::Save { name, project } => store
            .save(&name, &project)
            .map(|()| ProjectOutput::Saved(project))
            .map_err(|e| e.to_string()),
        ProjectTask::Delete { name } => store
            .delete(&name)
            .map(|()| ProjectOutput::Deleted)
            .map_err(|e| e.to_string()),
        ProjectTask::Import { name, path } => std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| store.import_json(&name, &bytes).map_err(|e| e.to_string()))
            .map(|project| ProjectOutput::Opened { name, project }),
    }
}

fn media_worker(
    service: MediaService,
    jobs: Receiver<(Ticket, MediaTask)>,
    events: Sender<AppEvent>,
) {
    while let Ok((ticket, task)) = jobs.recv() {
        if let MediaTask::Import { project, paths } = task {
            for path in paths {
                let result = service
                    .import_file(&project, &path)
                    .map(MediaOutput::Imported)
                    .map_err(|e| format!("{}：{e}", path.display()));
                if events.send(AppEvent::Media { ticket, result }).is_err() {
                    return;
                }
            }
            if events
                .send(AppEvent::Media {
                    ticket,
                    result: Ok(MediaOutput::ImportFinished),
                })
                .is_err()
            {
                return;
            }
            continue;
        }
        let result = match task {
            MediaTask::Peaks { path, bins } => match service.get_peaks(&path, bins) {
                Ok(todo_media::Waveform {
                    has_audio: true,
                    duration,
                    peaks,
                }) if !peaks.is_empty() && duration.is_finite() && duration > 0.0 => {
                    Ok(MediaOutput::Peaks {
                        path,
                        duration,
                        peaks,
                    })
                }
                Ok(_) => Err("音频波形不可用".into()),
                Err(e) => Err(e.to_string()),
            },
            MediaTask::Thumbnail { path, width } => service
                .get_thumb(&path, width)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    if bytes.is_empty() {
                        Err("缩略图为空".into())
                    } else {
                        Ok(MediaOutput::Thumbnail { path, bytes })
                    }
                }),
            MediaTask::Import { .. } | MediaTask::TextRaster(_) => unreachable!(),
        };
        if events.send(AppEvent::Media { ticket, result }).is_err() {
            return;
        }
    }
}

/// While scrubbing through a per-character intro only the newest frame of a text
/// matters: a queued frame is skipped when a later job renders the same text.
fn text_worker(
    mut rasterizer: TextRasterizer,
    jobs: Receiver<(Ticket, TextRaster)>,
    events: Sender<AppEvent>,
) {
    let mut queue = std::collections::VecDeque::new();
    loop {
        if queue.is_empty() {
            match jobs.recv() {
                Ok(job) => queue.push_back(job),
                Err(_) => return,
            }
        }
        queue.extend(jobs.try_iter());
        let (ticket, raster) = queue.pop_front().expect("queue is not empty");
        let superseded = queue.iter().any(|(_, later): &(Ticket, TextRaster)| {
            later.content == raster.content
                && later.style == raster.style
                && later.canvas == raster.canvas
        });
        if superseded {
            let result = Ok(MediaOutput::TextRasterSkipped);
            if events.send(AppEvent::Media { ticket, result }).is_err() {
                return;
            }
            continue;
        }
        let request = RasterRequest {
            content: &raster.content,
            style: &raster.style,
            canvas: raster.canvas,
            reveal: raster.reveal,
        };
        let result = rasterizer
            .raster(&raster.digest, request)
            .map(|(path, block)| MediaOutput::TextRaster {
                path: path.to_string_lossy().into_owned(),
                block,
            })
            .map_err(|e| e.to_string());
        if events.send(AppEvent::Media { ticket, result }).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
