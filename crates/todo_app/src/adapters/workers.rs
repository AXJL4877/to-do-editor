//! Blocking infrastructure runs outside the UI thread. Saves and text rasters
//! have their own queues so neither waits behind media tool jobs.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc};
use std::thread;
use todo_application::{
    AppEvent, MediaOutput, MediaTask, ProjectOutput, ProjectTask, TextRaster, Ticket,
};
use todo_media::{MediaService, RasterRequest, TextRasterizer};
use todo_storage::{ProjectLock, ProjectStore};

pub struct Workers {
    projects: Sender<(Ticket, ProjectTask)>,
    imports: Sender<(Ticket, MediaTask)>,
    assets: Sender<(Ticket, MediaTask)>,
    export: Sender<(Ticket, MediaTask)>,
    export_cancel: Arc<AtomicBool>,
    text: Sender<(Ticket, TextRaster)>,
    events: Receiver<AppEvent>,
}
impl Workers {
    pub fn start(repo: PathBuf) -> Self {
        let (projects, project_rx) = mpsc::channel();
        let (imports, import_rx) = mpsc::channel();
        let (assets, asset_rx) = mpsc::channel();
        let (export, export_rx) = mpsc::channel();
        let (text, text_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let tx = events_tx.clone();
        let project_root = repo.clone();
        thread::spawn(move || {
            let store = ProjectStore::new(project_root);
            // The open project, held against other processes until another
            // one opens or the editor exits.
            let mut held = None;
            while let Ok((ticket, task)) = project_rx.recv() {
                let result = run_project(&store, &mut held, task);
                if tx.send(AppEvent::Project { ticket, result }).is_err() {
                    break;
                }
            }
        });
        let rasterizer = TextRasterizer::new(&repo);
        let tx = events_tx.clone();
        thread::spawn(move || text_worker(rasterizer, text_rx, tx));
        let media = MediaService::new(repo.clone());
        for rx in [import_rx, asset_rx] {
            let tx = events_tx.clone();
            let service = media.clone();
            thread::spawn(move || media_worker(service, rx, tx));
        }
        let export_cancel = Arc::new(AtomicBool::new(false));
        let export_cancel_thread = export_cancel.clone();
        let export_root = repo.clone();
        let tx = events_tx.clone();
        thread::spawn(move || export_worker(export_root, export_rx, tx, export_cancel_thread));
        Self {
            projects,
            imports,
            assets,
            export,
            export_cancel,
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
        if matches!(&task, MediaTask::ExportTimeline { .. }) {
            return self.export.send((ticket, task)).map_err(|_| {
                Box::new(AppEvent::Media {
                    ticket,
                    result: Err("导出线程已停止".into()),
                })
            });
        }
        let importing = matches!(&task, MediaTask::Import { .. });
        // Generated files unblock timeline placement; never queue their probes
        // behind thumbnail decoding or full-file waveform extraction.
        let queue = if importing || matches!(&task, MediaTask::Adopt { .. }) {
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
    pub fn cancel_export(&self) {
        self.export_cancel.store(true, Ordering::Release);
    }
    pub fn poll(&self) -> Option<AppEvent> {
        self.events.try_recv().ok()
    }
}

fn run_project(
    store: &ProjectStore,
    held: &mut Option<ProjectLock>,
    task: ProjectTask,
) -> Result<ProjectOutput, String> {
    match task {
        ProjectTask::List => store
            .list()
            .map(ProjectOutput::Listed)
            .map_err(|e| e.to_string()),
        ProjectTask::Create { name } => store
            .create(&name)
            .map_err(|e| e.to_string())
            .and_then(|project| {
                hold(store, held, &name)?;
                Ok(ProjectOutput::Opened { name, project })
            }),
        ProjectTask::Load { name } => {
            // Another program (a command-line production) may be writing it.
            hold(store, held, &name)?;
            store
                .load(&name)
                .map(|project| ProjectOutput::Opened { name, project })
                .map_err(|e| e.to_string())
        }
        ProjectTask::Save { name, project } => store
            .save(&name, &project)
            .map(|()| ProjectOutput::Saved(project))
            .map_err(|e| e.to_string()),
        ProjectTask::Delete { name } => {
            if held.as_ref().is_some_and(|lock| lock.name() == name) {
                *held = None;
            }
            store
                .delete(&name)
                .map(|()| ProjectOutput::Deleted)
                .map_err(|e| e.to_string())
        }
        ProjectTask::Import { name, path } => std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| store.import_json(&name, &bytes).map_err(|e| e.to_string()))
            .and_then(|project| {
                hold(store, held, &name)?;
                Ok(ProjectOutput::Opened { name, project })
            }),
    }
}

/// Hold `name` instead of the project held before. A project another
/// process holds is refused; the previous one stays held then.
fn hold(store: &ProjectStore, held: &mut Option<ProjectLock>, name: &str) -> Result<(), String> {
    if held.as_ref().is_some_and(|lock| lock.name() == name) {
        return Ok(());
    }
    *held = Some(store.lock(name).map_err(|e| e.to_string())?);
    Ok(())
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
        if let MediaTask::Adopt { files, .. } = &task {
            let result = service
                .adopt_files(files)
                .map(MediaOutput::Adopted)
                .map_err(|e| e.to_string());
            if events.send(AppEvent::Media { ticket, result }).is_err() {
                return;
            }
            continue;
        }
        if let MediaTask::Export {
            frames,
            fps,
            audio,
            output,
            preset,
        } = &task
        {
            let preset = match preset.as_str() {
                "high" => todo_media::ExportPreset::HighQuality,
                "vertical" => todo_media::ExportPreset::Vertical1080p,
                "draft" => todo_media::ExportPreset::Draft,
                _ => todo_media::ExportPreset::Standard1080p30,
            };
            let result = todo_media::export_frames(
                service.tools(),
                &todo_media::ExportRequest {
                    frames: frames.clone(),
                    fps: *fps,
                    audio: audio.clone(),
                    output: output.clone(),
                    preset,
                },
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .map(|result| MediaOutput::Exported {
                output: result.output,
                report: result.report_path,
                frames: result.frames,
                failed_checks: result
                    .report
                    .checks
                    .iter()
                    .filter(|check| !check.passed)
                    .map(|check| format!("{}：{}", check.name, check.message))
                    .collect(),
            })
            .map_err(|error| error.to_string());
            if events.send(AppEvent::Media { ticket, result }).is_err() {
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
            MediaTask::Import { .. }
            | MediaTask::TextRaster(_)
            | MediaTask::Adopt { .. }
            | MediaTask::ExportTimeline { .. }
            | MediaTask::Export { .. } => {
                unreachable!()
            }
        };
        if events.send(AppEvent::Media { ticket, result }).is_err() {
            return;
        }
    }
}

fn export_worker(
    root: PathBuf,
    jobs: Receiver<(Ticket, MediaTask)>,
    events: Sender<AppEvent>,
    cancel: Arc<AtomicBool>,
) {
    while let Ok((ticket, task)) = jobs.recv() {
        let MediaTask::ExportTimeline {
            project,
            output,
            preset,
        } = task
        else {
            let _ = events.send(AppEvent::Media {
                ticket,
                result: Err("导出线程收到未知任务".into()),
            });
            continue;
        };
        cancel.store(false, Ordering::Release);
        let export_preset = match preset.as_str() {
            "high" => todo_media::ExportPreset::HighQuality,
            "vertical" => todo_media::ExportPreset::Vertical1080p,
            "draft" => todo_media::ExportPreset::Draft,
            _ => todo_media::ExportPreset::Standard1080p30,
        };
        let mut reported = 0usize;
        let result = crate::adapters::export::export_timeline(
            &root,
            &project,
            &output,
            export_preset,
            &cancel,
            &mut |done, total| {
                if done == total || done >= reported.saturating_add(10) {
                    reported = done;
                    let _ = events.send(AppEvent::Media {
                        ticket,
                        result: Ok(MediaOutput::ExportProgress { done, total }),
                    });
                }
            },
        )
        .map(|outcome| MediaOutput::Exported {
            output: outcome.output,
            report: outcome.report,
            frames: outcome.frames,
            failed_checks: outcome.failed_checks,
        })
        .map_err(|error| error.to_string());
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
