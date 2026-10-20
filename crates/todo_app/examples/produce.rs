//! Run a style recipe straight into a project, headless and without the
//! director: the code pipeline the editor's production panel runs. Uses the
//! real host and AI services (spends money), prints when each item reaches
//! the timeline, saves the project and optionally exports it.
//!
//! ```text
//! cargo run -p todo_app --example produce -- <project> (--topic "<主题>" | --script <file>)
//!     [--style studio.spongebob-koubo] [--seconds 30] [--fresh] [--export out.mp4]
//! ```
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use todo_app::adapters::production::host::{HostReport, ProductionHost};
use todo_application::productions::ProductionOutput;
use todo_application::{
    AppEvent, AppUpdate, Effect, MediaOutput, MediaTask, ProjectOutput, ProjectTask, Workspace,
};
use todo_interaction::ParamValue;
use todo_production::extension::protocol::Canvas;
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

    fn drain(&mut self) {
        while !self.pending.is_empty() {
            let event = self.pending.remove(0);
            let update = self.ws.on_event(event);
            self.answer(update);
        }
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first().filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!("usage: produce <project> (--topic <主题> | --script <file>) [--style id] [--seconds 30] [--fresh] [--export out.mp4]");
        std::process::exit(2);
    };
    let style = arg(&args, "--style").unwrap_or_else(|| "studio.spongebob-koubo".into());
    let script = arg(&args, "--script")
        .map(|path| std::fs::read_to_string(path).expect("read script"))
        .unwrap_or_default();
    let mut values = BTreeMap::new();
    if let Some(topic) = arg(&args, "--topic") {
        values.insert("topic".to_owned(), ParamValue::Text(topic));
    }
    if let Some(seconds) = arg(&args, "--seconds") {
        values.insert("seconds".to_owned(), ParamValue::Text(seconds));
    }
    let export = arg(&args, "--export").map(PathBuf::from);
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
    if args.iter().any(|a| a == "--fresh") {
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
    let canvas = Canvas {
        width: project.width,
        height: project.height,
        fps: project.fps,
    };
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
    // Extensions are self-checked in the background.
    let started = Instant::now();
    let (doc, setup) = loop {
        let _ = run.host.poll();
        match run
            .host
            .create(&name, &style, &values, &script, canvas.clone())
        {
            Ok(created) => break created,
            Err(error) if started.elapsed() > Duration::from_secs(60) => {
                eprintln!("cannot start {style}: {error}");
                std::process::exit(1);
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    };
    let update = run.ws.add_production(doc, setup);
    run.answer(update);
    let started = Instant::now();
    let clock = |started: Instant| format!("[{:>5.1}s]", started.elapsed().as_secs_f64());
    let mut phase = String::new();
    let mut finished: Option<Result<serde_json::Value, String>> = None;
    while finished.is_none() {
        run.drain();
        for report in run.host.poll() {
            let HostReport::Output(output) = report else {
                if let HostReport::Notice(message) = report {
                    eprintln!("notice: {message}");
                }
                continue;
            };
            match &output {
                ProductionOutput::RunProgress { phase: now, .. } if *now != phase => {
                    println!("{} {now}", clock(started));
                    phase = now.clone();
                }
                ProductionOutput::RunItems { items, .. } => {
                    let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
                    println!("{} 入轨 {}", clock(started), keys.join(" "));
                }
                ProductionOutput::RunDone { result, .. } => finished = Some(Ok(result.clone())),
                ProductionOutput::RunFailed { message, .. } => {
                    finished = Some(Err(message.clone()))
                }
                ProductionOutput::Attention { message, .. } => eprintln!("attention: {message}"),
                _ => {}
            }
            run.pending.push(AppEvent::Production(output));
        }
        if started.elapsed() > Duration::from_secs(3600) {
            finished = Some(Err("timed out".into()));
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    // Let the last placements and saves land.
    for _ in 0..40 {
        run.drain();
        for report in run.host.poll() {
            if let HostReport::Output(output) = report {
                run.pending.push(AppEvent::Production(output));
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    run.drain();
    let project = run.ws.editor().unwrap().project().clone();
    run.store.save(&name, &project).expect("save project");
    println!(
        "project {name}: {} tracks, {} clips, {:.2}s",
        project.tracks.len(),
        project.tracks.iter().map(|t| t.clips.len()).sum::<usize>(),
        todo_core::content_duration(&project)
    );
    match finished.unwrap() {
        Ok(result) => println!(
            "RESULT {}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        ),
        Err(message) => {
            println!("FAILED: {message}");
            std::process::exit(1);
        }
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
