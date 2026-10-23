//! The built-in director, headless: one request in, a reviewed cut out.
//! Uses the real host, AI services, renderer and workspace (spends money),
//! saves the project like the editor would and optionally exports it.
//!
//! ```text
//! cargo run -p todo_app --example direct -- <project> "<request>" [--fresh] [--export out.mp4]
//! ```
use std::path::PathBuf;
use std::time::{Duration, Instant};
use todo_app::adapters::director::DirectorDriver;
use todo_app::adapters::production::host::{HostReport, ProductionHost};
use todo_application::director::Phase;
use todo_application::{
    AppEvent, AppUpdate, Effect, MediaOutput, MediaTask, ProjectOutput, ProjectTask, Workspace,
};
use todo_storage::ProjectStore;

struct Headless {
    root: PathBuf,
    ws: Workspace,
    host: ProductionHost,
    store: ProjectStore,
    pending: Vec<AppEvent>,
}

impl Headless {
    fn answer(&mut self, update: AppUpdate) {
        for effect in update.effects {
            match effect {
                Effect::Production(task) => {
                    if let Some(problem) = self.host.execute(task) {
                        eprintln!("host refused a task: {problem}");
                    }
                }
                Effect::Media {
                    ticket,
                    task: MediaTask::Import { project, paths },
                } => {
                    let service = todo_media::MediaService::new(&self.root);
                    for path in paths {
                        let result = service
                            .import_file(&project, &path)
                            .map(MediaOutput::Imported)
                            .map_err(|e| format!("{}：{e}", path.display()));
                        self.pending.push(AppEvent::Media { ticket, result });
                    }
                    self.pending.push(AppEvent::Media {
                        ticket,
                        result: Ok(MediaOutput::ImportFinished),
                    });
                }
                Effect::Media {
                    ticket,
                    task: MediaTask::Adopt { files, .. },
                } => {
                    let result = todo_media::MediaService::new(&self.root)
                        .adopt_files(&files)
                        .map(MediaOutput::Adopted)
                        .map_err(|e| e.to_string());
                    self.pending.push(AppEvent::Media { ticket, result });
                }
                Effect::Project {
                    ticket,
                    task: ProjectTask::Save { name, project },
                } => {
                    let result = self
                        .store
                        .save(&name, &project)
                        .map(|()| ProjectOutput::Saved(project))
                        .map_err(|e| e.to_string());
                    self.pending.push(AppEvent::Project { ticket, result });
                }
                _ => {}
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: direct <project> \"<request>\" [--fresh] [--export out.mp4]");
        std::process::exit(2);
    }
    let (name, request) = (args[0].clone(), args[1].clone());
    let fresh = args.iter().any(|a| a == "--fresh");
    let export = args
        .iter()
        .position(|a| a == "--export")
        .map(|i| PathBuf::from(&args[i + 1]));
    let root = std::env::var_os("TODO_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    for (var, tool) in [("TODO_FFPROBE", "ffprobe"), ("TODO_FFMPEG", "ffmpeg")] {
        let path = root.join(format!("dependencies/ffmpeg/bin/{tool}.exe"));
        if std::env::var_os(var).is_none() && path.is_file() {
            std::env::set_var(var, path);
        }
    }
    let store = ProjectStore::new(root.clone());
    // Never write a project the editor (or another run) has open.
    let refuse = |error: todo_storage::StorageError| -> ! {
        eprintln!("{error}");
        std::process::exit(3);
    };
    if fresh {
        match store.delete(&name) {
            Ok(()) | Err(todo_storage::StorageError::NotFound(_)) => {}
            Err(error) => refuse(error),
        }
    }
    let project = store
        .load(&name)
        .or_else(|_| store.create(&name))
        .expect("project");
    let _held = store.lock(&name).unwrap_or_else(|error| refuse(error));
    let mut ws = Workspace::with_project(project, &name);
    let (library, warnings) = todo_app::adapters::motions::load(&root);
    for warning in warnings {
        eprintln!("motion library: {warning}");
    }
    ws.set_motion_library(library.presets().cloned().collect());
    let mut run = Headless {
        root: root.clone(),
        ws,
        host: ProductionHost::start(&root).expect("production host"),
        store,
        pending: Vec::new(),
    };
    // Extensions are self-checked in the background; styles appear when ready.
    let started = Instant::now();
    while run.host.styles().is_empty() && started.elapsed() < Duration::from_secs(60) {
        let _ = run.host.poll(None, false, None);
        std::thread::sleep(Duration::from_millis(200));
    }
    println!(
        "styles: {:?}",
        run.host.styles().iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    let (mut driver, update) =
        DirectorDriver::start(root.clone(), request, &mut run.ws, &mut run.host).expect("start");
    run.answer(update);
    let mut shown = 0;
    let mut phase = String::new();
    let started = Instant::now();
    while driver.director().busy() {
        while !run.pending.is_empty() {
            let event = run.pending.remove(0);
            let update = run.ws.on_event(event);
            run.answer(update);
        }
        for report in run.host.poll(None, false, None) {
            match report {
                HostReport::Output(output) => run.pending.push(AppEvent::Production(output)),
                HostReport::AgentReply { request_id, result } => {
                    let update = driver.on_reply(request_id, result, &mut run.ws, &mut run.host);
                    run.answer(update);
                }
                HostReport::Notice(message) => eprintln!("notice: {message}"),
                _ => {}
            }
        }
        let update = driver.tick(&mut run.ws, &mut run.host);
        run.answer(update);
        let now = format!("{:?}", driver.director().phase());
        if now != phase {
            println!("[{:>5.0}s] phase {now}", started.elapsed().as_secs_f64());
            phase = now;
        }
        for entry in &driver.director().log()[shown..] {
            println!(
                "[{:>5.0}s] {:?}: {}",
                started.elapsed().as_secs_f64(),
                entry.kind,
                entry.text
            );
        }
        shown = driver.director().log().len();
        if started.elapsed() > Duration::from_secs(3600) {
            println!("timed out");
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let project = run.ws.editor().unwrap().project().clone();
    run.store.save(&name, &project).expect("save project");
    println!(
        "project {name}: {} tracks, {} clips, {:.2}s",
        project.tracks.len(),
        project.tracks.iter().map(|t| t.clips.len()).sum::<usize>(),
        todo_core::content_duration(&project)
    );
    if let Some(approval) = driver.director().approval() {
        println!("APPROVAL: {}", approval.summary);
        for item in &approval.remaining {
            println!("  - {item}");
        }
        for (t, path) in &approval.frames {
            println!("  frame {t:.2}s {path}");
        }
    }
    if let Phase::Failed { reason } = driver.director().phase() {
        println!("FAILED: {reason}");
        std::process::exit(1);
    }
    if let Some(output) = export {
        let outcome = todo_app::adapters::export::export_timeline(
            &root,
            &project,
            &output,
            todo_media::ExportPreset::HighQuality,
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_, _| {},
        )
        .expect("export");
        println!("exported {} ({} frames)", output.display(), outcome.frames);
        if !outcome.failed_checks.is_empty() {
            println!("EXPORT CHECKS FAILED: {:?}", outcome.failed_checks);
            std::process::exit(1);
        }
    }
}
