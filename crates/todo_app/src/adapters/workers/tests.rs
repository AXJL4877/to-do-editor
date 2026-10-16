use super::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
struct TestDirectory {
    path: PathBuf,
    boundary: PathBuf,
}
impl TestDirectory {
    fn new() -> Self {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let boundary = repo.join("target/todo-app-import-tests");
        fs::create_dir_all(&boundary).unwrap();
        let boundary = boundary.canonicalize().unwrap();
        let path = boundary.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let tools = todo_media::MediaTools::discover(&repo);
        let packaged = path.join("dependencies/ffmpeg/bin");
        fs::create_dir_all(&packaged).unwrap();
        for (name, tool) in [("ffmpeg.exe", tools.ffmpeg), ("ffprobe.exe", tools.ffprobe)] {
            fs::hard_link(&tool, packaged.join(name)).unwrap();
        }
        Self { path, boundary }
    }
    fn audio(&self, name: &str) -> PathBuf {
        let path = self.path.join(name);
        let mut file = fs::File::create(&path).unwrap();
        let frames = 4800_u32;
        let bytes = frames * 2;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + bytes).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16_u32.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&48000_u32.to_le_bytes()).unwrap();
        file.write_all(&96000_u32.to_le_bytes()).unwrap();
        file.write_all(&2_u16.to_le_bytes()).unwrap();
        file.write_all(&16_u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&bytes.to_le_bytes()).unwrap();
        file.write_all(&vec![0; bytes as usize]).unwrap();
        path
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Ok(path) = self.path.canonicalize() {
            if path.parent() == Some(self.boundary.as_path()) {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}
fn next_event(workers: &Workers) -> AppEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = workers.poll() {
            return event;
        }
        assert!(Instant::now() < deadline, "worker event timed out");
        thread::sleep(Duration::from_millis(5));
    }
}
fn import_result(workers: &Workers, expected: Ticket) -> Result<MediaOutput, String> {
    match next_event(workers) {
        AppEvent::Media { ticket, result } => {
            assert_eq!(ticket, expected);
            result
        }
        event => panic!("expected media event: {event:?}"),
    }
}

#[test]
fn batch_failures_preserve_local_imports_completion_and_future_batches() {
    let directory = TestDirectory::new();
    let store = ProjectStore::new(&directory.path);
    store.create("reel").unwrap();
    store.create("future").unwrap();
    let missing = directory.path.join("missing.wav");
    let corrupt = directory.path.join("corrupt.mp4");
    fs::write(&corrupt, b"invalid media").unwrap();
    let good = directory.audio("声音 一.wav");
    let bytes = fs::read(&good).unwrap();
    let future = directory.audio("future.wav");
    let workers = Workers::start(directory.path.clone());
    let first = Ticket { id: 12, session: 7 };
    let second = Ticket { id: 13, session: 8 };
    let empty = Ticket { id: 14, session: 8 };
    workers
        .media(
            first,
            MediaTask::Import {
                project: "reel".into(),
                paths: vec![missing.clone(), corrupt.clone(), good],
            },
        )
        .unwrap();
    workers
        .media(
            second,
            MediaTask::Import {
                project: "future".into(),
                paths: vec![future],
            },
        )
        .unwrap();
    workers
        .media(
            empty,
            MediaTask::Import {
                project: "future".into(),
                paths: vec![],
            },
        )
        .unwrap();
    assert!(import_result(&workers, first)
        .unwrap_err()
        .contains(&missing.display().to_string()));
    assert!(import_result(&workers, first)
        .unwrap_err()
        .contains(&corrupt.display().to_string()));
    match import_result(&workers, first).unwrap() {
        MediaOutput::Imported(item) => {
            assert_eq!(item.name, "声音 一.wav");
            assert_eq!(item.has_audio, Some(true));
            let stored = item.path.unwrap();
            assert!(stored.starts_with("data/reel/media/"));
            assert_eq!(fs::read(directory.path.join(stored)).unwrap(), bytes);
        }
        event => panic!("lost successful import: {event:?}"),
    }
    assert!(matches!(
        import_result(&workers, first),
        Ok(MediaOutput::ImportFinished)
    ));
    assert!(
        matches!(import_result(&workers, second), Ok(MediaOutput::Imported(item)) if item.path.as_deref().unwrap().starts_with("data/future/media/"))
    );
    assert!(matches!(
        import_result(&workers, second),
        Ok(MediaOutput::ImportFinished)
    ));
    assert!(matches!(
        import_result(&workers, empty),
        Ok(MediaOutput::ImportFinished)
    ));
    assert_eq!(
        fs::read_dir(directory.path.join("data/reel/media"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn stopped_import_queue_returns_a_terminal_event_that_releases_the_workspace() {
    use todo_application::{control::*, Workspace};
    let (projects, _project_rx) = mpsc::channel();
    let (imports, import_rx) = mpsc::channel();
    drop(import_rx);
    let (assets, _asset_rx) = mpsc::channel();
    let (text, _text_rx) = mpsc::channel();
    let (_events_tx, events) = mpsc::channel();
    let workers = Workers {
        projects,
        imports,
        assets,
        text,
        events,
    };
    let project = todo_application::Project {
        version: 1,
        fps: 30.0,
        width: 1920,
        height: 1080,
        playhead: 0.0,
        duration: Some(60.0),
        media_scale: Default::default(),
        media: Vec::new(),
        tracks: Vec::new(),
        text_styles: Vec::new(),
        folders: Vec::new(),
};
    let mut workspace = Workspace::with_project(project, "queue-test");
    let context = workspace.control_context().unwrap();
    let outcome = workspace.control(ControlRequest {
        request_id: "import".into(),
        session_id: context.session_id,
        expected_revision: Some(context.revision),
        command: ControlCommand::ImportMedia {
            paths: vec!["A.mp4".into()],
                    folder_id: None,
        },
    });
    let (ticket, task) = outcome
        .update
        .effects
        .into_iter()
        .find_map(|effect| match effect {
            todo_application::Effect::Media { ticket, task } => Some((ticket, task)),
            _ => None,
        })
        .unwrap();
    workspace.on_event(*workers.media(ticket, task).unwrap_err());
    assert!(!workspace.editor().unwrap().importing());
    let job = workspace.control_context().unwrap().imports.pop().unwrap();
    assert_eq!(job.status, ImportStatus::Failed);
    assert_eq!(job.errors.len(), 1);
}

#[test]
fn local_save_acknowledges_and_persists_the_submitted_snapshot() {
    let directory = TestDirectory::new();
    let store = ProjectStore::new(&directory.path);
    let mut project = store.create("reel").unwrap();
    project.fps = 24.0;
    project.playhead = 7.25;
    project.duration = Some(20.0);
    let submitted = project.clone();
    let workers = Workers::start(directory.path.clone());
    let ticket = Ticket { id: 41, session: 9 };
    workers
        .project(
            ticket,
            ProjectTask::Save {
                name: "reel.json".into(),
                project: submitted.clone(),
            },
        )
        .unwrap();
    project.playhead = 15.0;
    project.fps = 60.0;
    match next_event(&workers) {
        AppEvent::Project {
            ticket: actual,
            result: Ok(ProjectOutput::Saved(saved)),
        } => {
            assert_eq!(actual, ticket);
            assert_eq!(saved, submitted);
            assert_ne!(saved, project);
            assert_eq!(store.load("reel").unwrap(), saved);
        }
        event => panic!("save did not acknowledge its snapshot: {event:?}"),
    }
}
