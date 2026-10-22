//! A chalk-hand production through the real host, worker pool, Python baker
//! and media adoption, in an isolated app root. Only the AI answers are
//! faked: no network, no cost. Skipped when no Python with numpy/cv2/PIL.
use image::{Rgb, RgbImage};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use todo_app::adapters::production::host::{HostReport, ProductionHost};
use todo_application::{
    AppAction, AppEvent, AppUpdate, EditorCommand, Effect, MediaOutput, MediaTask, Pcm16,
    PreviewCommand, PreviewLayerKind, ProductionOutput, ProductionTask, ProjectOutput, ProjectTask,
    Recipe, Workspace,
};
use todo_core::{create_project, AlphaPacking};
use todo_interaction::ParamValue;
use todo_workflow::workflow::protocol::Canvas;
use todo_production::TaskRequest;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The bundled runtime, else a real interpreter on PATH (never the Store alias).
fn python() -> Option<String> {
    let bundled = repo().join("dependencies/python/python.exe");
    let candidates = std::iter::once(bundled).chain(
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter(|d| !d.to_string_lossy().contains("WindowsApps"))
            .map(|d| d.join("python.exe")),
    );
    candidates
        .filter(|c| c.is_file())
        .find(|c| {
            Command::new(c)
                .args(["-c", "import numpy, cv2, PIL"])
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .map(|c| c.to_string_lossy().into_owned())
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy() == "__pycache__" {
            continue;
        }
        if path.is_dir() {
            copy_dir(&path, &to.join(name));
        } else {
            std::fs::copy(&path, to.join(name)).unwrap();
        }
    }
}

/// White chalk lines on black, like the image model's answers.
fn chalk_image(path: &Path, variant: u32) {
    let mut img = RgbImage::from_pixel(512, 512, Rgb([0, 0, 0]));
    let r = 150.0 + variant as f64 * 20.0;
    for y in 0..512u32 {
        for x in 0..512u32 {
            let (dx, dy) = (x as f64 - 256.0, y as f64 - 256.0);
            let ring = ((dx * dx + dy * dy).sqrt() - r).abs() < 9.0;
            let cross =
                (dx.abs() < 6.0 || dy.abs() < 6.0) && dx.abs() < r * 0.6 && dy.abs() < r * 0.6;
            if ring || cross {
                img.put_pixel(x, y, Rgb([235, 235, 228]));
            }
        }
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    img.save(path).unwrap();
}

fn wav(path: &Path, samples: &[i16], rate: u32) {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// One word per character (0.2 s), 0.5 s after each sentence; a tone while speaking.
fn fake_voice(text: &str) -> (serde_json::Value, Vec<i16>, u32) {
    let rate = 16000u32;
    let mut t = 0.2;
    let (mut words, mut sentences, mut current, mut spans) = (vec![], vec![], vec![], vec![]);
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        let punct = "，。！？、；：".contains(c);
        let d = if punct { 0.1 } else { 0.2 };
        if !punct {
            spans.push((t, t + d));
        }
        words.push(json!({"word": c.to_string(), "start": t, "end": t + d}));
        current.push(json!({"word": c.to_string(), "startTime": t, "endTime": t + d}));
        t += d;
        if "。！？".contains(c) {
            sentences.push(json!({"words": std::mem::take(&mut current)}));
            t += 0.5;
        }
    }
    let mut samples = vec![0i16; ((t + 0.3) * rate as f64) as usize];
    for (a, b) in spans {
        for (i, sample) in samples
            .iter_mut()
            .enumerate()
            .take((b * rate as f64) as usize)
            .skip((a * rate as f64) as usize)
        {
            *sample = ((i as f64 * 0.09).sin() * 8000.0) as i16;
        }
    }
    (
        json!({"words": words, "sentences": sentences}),
        samples,
        rate,
    )
}

struct Run {
    root: PathBuf,
    ws: Workspace,
    host: ProductionHost,
    pending: Vec<AppEvent>,
    images: u32,
}

impl Run {
    fn answer(&mut self, update: AppUpdate) {
        for effect in update.effects {
            match effect {
                Effect::Production(ProductionTask::Submit { workdir, spec }) => {
                    let key = spec.key();
                    let dir = self.root.join(&workdir);
                    let output = match &spec.request {
                        TaskRequest::Audio(speech) => {
                            let (raw, samples, rate) = fake_voice(&speech.text);
                            let file = format!("services/{}.wav", key.0);
                            wav(&dir.join(&file), &samples, rate);
                            ProductionOutput::TaskDone {
                                key,
                                output: Some(raw),
                                file: Some(file),
                                pcm: Some(Pcm16 { samples, rate }),
                            }
                        }
                        TaskRequest::Language(_) => ProductionOutput::TaskDone {
                            key,
                            output: Some(json!({"text": json!({"notes": "", "scenes": [
                                {"start": 0, "elements": [
                                    {"at": 0, "label": "存钱罐", "draw": "a piggy bank", "role": "hero"},
                                    {"at": 1, "label": "时钟", "draw": "a clock", "role": "support"},
                                    {"at": 2, "label": "复利", "draw": "the word 复利", "role": "accent"}]}
                            ]}).to_string()})),
                            file: None,
                            pcm: None,
                        },
                        TaskRequest::Image(_) => {
                            let file = format!("services/{}.png", key.0);
                            chalk_image(&dir.join(&file), self.images);
                            self.images += 1;
                            ProductionOutput::TaskDone {
                                key,
                                output: None,
                                file: Some(file),
                                pcm: None,
                            }
                        }
                        TaskRequest::Video(_) | TaskRequest::Multimodal(_) => {
                            ProductionOutput::TaskDone {
                                key,
                                output: None,
                                file: None,
                                pcm: None,
                            }
                        }
                    };
                    self.pending.push(AppEvent::Production(output));
                }
                Effect::Production(task) => {
                    if std::env::var("E2E_TRACE").is_ok() {
                        eprintln!(
                            "task {}",
                            format!("{task:?}").chars().take(160).collect::<String>()
                        );
                    }
                    if let Some(problem) = self.host.execute(task) {
                        panic!("host refused a task: {problem}");
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
                    task: ProjectTask::Save { project, .. },
                } => self.pending.push(AppEvent::Project {
                    ticket,
                    result: Ok(ProjectOutput::Saved(project)),
                }),
                _ => {}
            }
        }
    }

    fn settle(&mut self, deadline: Duration) {
        let started = Instant::now();
        loop {
            while let Some(event) = (!self.pending.is_empty()).then(|| self.pending.remove(0)) {
                let update = self.ws.on_event(event);
                self.answer(update);
            }
            for report in self.host.poll() {
                match report {
                    HostReport::Output(output) => {
                        if std::env::var("E2E_TRACE").is_ok() {
                            eprintln!(
                                "{:?}",
                                format!("{output:?}").chars().take(300).collect::<String>()
                            );
                        }
                        self.pending.push(AppEvent::Production(output));
                    }
                    HostReport::Notice(message) => eprintln!("notice: {message}"),
                    HostReport::Models { .. }
                    | HostReport::ModelsFailed { .. }
                    | HostReport::Connection { .. }
                    | HostReport::AgentReply { .. } => {}
                }
            }
            let status = &self.ws.production_status()[0];
            let busy = !self.pending.is_empty();
            if !busy && status.total > 0 && status.done == status.total {
                return;
            }
            assert!(
                started.elapsed() < deadline,
                "production did not finish: {:#?}",
                status.nodes
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn chalk_production_runs_offline_through_the_real_host() {
    let Some(python) = python() else {
        eprintln!("skipped: no Python with numpy, cv2 and PIL");
        return;
    };
    let repo = repo();
    let root = repo.join("target/offline-production");
    let _ = std::fs::remove_dir_all(&root);
    copy_dir(
        &repo.join("extensions/builtin/chalk-hand"),
        &root.join("extensions/builtin/chalk-hand"),
    );
    copy_dir(&repo.join("extensions/sdk"), &root.join("extensions/sdk"));
    std::fs::create_dir_all(root.join("data/e2e")).unwrap();
    // Tools and runtime of the development checkout.
    std::env::set_var("TODO_PYTHON", &python);
    for (var, tool) in [("TODO_FFPROBE", "ffprobe"), ("TODO_FFMPEG", "ffmpeg")] {
        let path = repo.join(format!("dependencies/ffmpeg/bin/{tool}.exe"));
        if path.is_file() {
            std::env::set_var(var, path);
        }
    }
    let host = ProductionHost::start(&root).unwrap();
    let mut project = create_project(None, None, None, None, None);
    project.width = 960;
    project.height = 540;
    let ws = Workspace::with_project(project, "e2e");
    let mut run = Run {
        root: root.clone(),
        ws,
        host,
        pending: Vec::new(),
        images: 0,
    };
    let params = BTreeMap::from([
        ("split".to_string(), ParamValue::Text("paragraph".into())),
        ("segmentChars".to_string(), ParamValue::Number(60.0)),
        ("drawSpeed".to_string(), ParamValue::Number(3.0)),
    ]);
    let (doc, setup) = run
        .host
        .create(
            "e2e",
            "builtin.chalk-hand",
            &params,
            "存钱是第一步。复利让时间替你工作，越早开始越好。",
            Canvas {
                width: 960,
                height: 540,
                fps: 30.0,
            },
        )
        .unwrap();
    let update = run.ws.add_production(doc, setup);
    run.answer(update);
    run.settle(Duration::from_secs(
        std::env::var("E2E_DEADLINE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(240),
    ));

    let project = run.ws.editor().unwrap().project().clone();
    let keys: Vec<String> = project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|c| c.origin.as_ref().map(|o| o.key.clone()))
        .collect();
    assert!(keys.iter().any(|k| k == "plate"), "{keys:?}");
    assert!(keys.iter().any(|k| k.ends_with("/voice")), "{keys:?}");
    assert!(keys.iter().any(|k| k.contains("/sub/")), "{keys:?}");
    assert!(
        keys.iter().any(|k| k.ends_with("/dust")),
        "stamp dust item: {keys:?}"
    );
    // The hero is written by hand: a reveal atlas with a chalk hand path.
    let reveal = project
        .media
        .iter()
        .find(|m| m.alpha == Some(AlphaPacking::Reveal))
        .expect("a hand-drawn element is a reveal atlas");
    assert!(reveal.reveal.as_ref().unwrap().hand.is_some());
    let clip = project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.media_id() == Some(reveal.id.as_str()))
        .unwrap()
        .clone();
    // Mid-way through the stroke the preview plays the atlas and shows the hand.
    let mid = clip.start + reveal.duration * 0.5;
    let update = run.ws.dispatch(AppAction::Editor(EditorCommand::Seek(mid)));
    let layers: Vec<_> = update
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Preview(commands) => Some(commands),
            _ => None,
        })
        .flatten()
        .filter_map(|c| match c {
            PreviewCommand::Layers(layers) => Some(layers.clone()),
            _ => None,
        })
        .next_back()
        .expect("seeking sends layers");
    let atlas = layers
        .iter()
        .find(|l| l.kind == PreviewLayerKind::Reveal)
        .expect("reveal layer");
    assert!(
        atlas.media_time > 0.0 && atlas.media_time < 1.0,
        "{}",
        atlas.media_time
    );
    assert!(
        layers.iter().any(|l| l.path.ends_with("hand_chalk.png")),
        "chalk hand layer"
    );
    // Everything the production made lives in its working directory.
    assert!(root.join("data/e2e/productions").is_dir());

    // Export the timeline with the renderer the preview uses.
    let host = std::env::var_os("TODO_PREVIEW_HOST").map(PathBuf::from);
    if !host.as_ref().is_some_and(|h| h.is_file())
        || !root
            .join("../../dependencies/ffmpeg/bin/ffmpeg.exe")
            .is_file()
    {
        eprintln!("export skipped: set TODO_PREVIEW_HOST to a current preview_host.exe");
        return;
    }
    let output = root.join("export/e2e.mp4");
    let mut last = 0;
    let outcome = todo_app::adapters::export::export_timeline(
        &root,
        &project,
        &output,
        todo_media::ExportPreset::Draft,
        &std::sync::atomic::AtomicBool::new(false),
        &mut |done, total| last = done.max(last).min(total),
    )
    .unwrap();
    assert!(output.is_file() && outcome.report.is_file());
    assert_eq!(last, outcome.frames);
    assert!(
        outcome.failed_checks.is_empty(),
        "{:?}",
        outcome.failed_checks
    );
}

#[test]
fn spongebob_workflow_runs_offline_and_emits_editable_multishot_items() {
    let Some(python) = python() else {
        eprintln!("skipped: no Python with numpy, cv2 and PIL");
        return;
    };
    let repo = repo();
    let root = repo.join("target/offline-workflow");
    let _ = std::fs::remove_dir_all(&root);
    copy_dir(
        &repo.join("extensions/builtin/spongebob-koubo"),
        &root.join("extensions/builtin/spongebob-koubo"),
    );
    copy_dir(&repo.join("extensions/sdk"), &root.join("extensions/sdk"));
    std::fs::create_dir_all(root.join("data/e2e")).unwrap();
    std::env::set_var("TODO_PYTHON", &python);
    for (var, tool) in [("TODO_FFPROBE", "ffprobe"), ("TODO_FFMPEG", "ffmpeg")] {
        let path = repo.join(format!("dependencies/ffmpeg/bin/{tool}.exe"));
        if path.is_file() {
            std::env::set_var(var, path);
        }
    }
    let host = ProductionHost::start(&root).unwrap();
    let mut project = create_project(None, None, None, None, None);
    project.width = 960;
    project.height = 540;
    let mut ws = Workspace::with_project(project, "e2e");
    // As in the app: the motion library includes the extension's card motions.
    let (motions, warnings) = todo_app::adapters::motions::load(&root);
    assert!(warnings.is_empty(), "{warnings:?}");
    ws.set_motion_library(motions.presets().cloned().collect());
    let mut run = Run {
        root: root.clone(),
        ws,
        host,
        pending: Vec::new(),
        images: 0,
    };
    // Offline: the worker answers its service calls from tests/fixture.json.
    let (doc, mut setup) = run
        .host
        .create(
            "e2e",
            "studio.spongebob-koubo",
            &BTreeMap::from([("topic".to_string(), ParamValue::Text("沉没成本".into()))]),
            "",
            Canvas {
                width: 960,
                height: 540,
                fps: 30.0,
            },
        )
        .unwrap();
    if let Recipe::Workflow { input, .. } = &mut setup.recipe {
        input["offline"] = serde_json::json!(true);
    }
    let update = run.ws.add_production(doc, setup);
    run.answer(update);
    run.settle(Duration::from_secs(120));
    let project = run.ws.editor().unwrap().project().clone();
    let keyed = |key: &str| {
        project
            .tracks
            .iter()
            .enumerate()
            .find_map(|(index, track)| {
                track
                    .clips
                    .iter()
                    .find(|c| c.origin.as_ref().is_some_and(|o| o.key == key))
                    .map(|clip| (index, track, clip.clone()))
            })
    };
    for expected in [
        "voice",
        "sub/0",
        "title/card",
        "title/text",
        "bg/0",
        "bg/1",
        "shot/0/card",
        "shot/0/el/0",
        "shot/1/card",
        "shot/1/el/0",
        "shot/2/card",
        "shot/5/card",
        "shot/6/line",
        "shot/6/backdrop",
        "bgm/0",
        "bgm/1",
    ] {
        assert!(keyed(expected).is_some(), "missing {expected}");
    }
    // The plate dissolves in from the title card on the same track.
    let (plate_index, plate_track, plate) = keyed("bg/0").unwrap();
    assert_eq!(plate.transition.as_ref().unwrap().kind, "dissolve");
    assert!(todo_core::transition_is_active(
        &project,
        plate_track,
        &plate.id
    ));
    // The extension's motions were copied into the project and play: shot 1
    // drops in over shot 0 (pictures arrived one by one, the crossing still
    // works), elements pop in with their own motions.
    assert!(project.motions.iter().any(|m| m.id == "sb.cardDrop"));
    assert!(project.motions.iter().any(|m| m.id.starts_with("sb.el")));
    let (card_index, track, clip) = keyed("shot/1/card").unwrap();
    assert_eq!(clip.transition.as_ref().unwrap().kind, "sb.cardDrop");
    assert!(todo_core::transition_is_active(&project, track, &clip.id));
    let (element_index, _, coins) = keyed("shot/1/el/0").unwrap();
    assert!(coins.intro.as_ref().unwrap().kind.starts_with("sb.el"));
    assert!(element_index < card_index, "elements draw above the card");
    // Pictures draw above the plate.
    for (index, track) in project.tracks.iter().enumerate() {
        for clip in &track.clips {
            let Some(key) = clip.origin.as_ref().map(|o| o.key.as_str()) else {
                continue;
            };
            if key.starts_with("shot/") && clip.media_id().is_some() {
                assert!(index < plate_index, "{key} is under the plate");
            }
        }
    }
    // The sublime shot: the red line over its blurred picture, no card.
    let (line_index, _, line) = keyed("shot/6/line").unwrap();
    assert_eq!(line.text().unwrap().style_id, "sublime-red-brush");
    let (backdrop_index, _, backdrop) = keyed("shot/6/backdrop").unwrap();
    assert_eq!(backdrop.intro.as_ref().unwrap().kind, "sb.backdropZoom");
    assert!(line_index < backdrop_index && backdrop_index < plate_index);
    assert!(keyed("shot/6/card").is_none());
    let (_, _, sub) = keyed("sub/0").unwrap();
    assert_eq!(sub.text().unwrap().style_id, "subtitle-kuaile");
    assert!(project.media.len() >= 8, "all workflow files were adopted");
    assert!(root.join("data/e2e/productions").is_dir());
}
