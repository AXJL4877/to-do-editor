//! Host services for extension workers. Paid services go through the durable
//! `AiService` (one journal entry per content key, so a rerun or a crash
//! retry never pays twice); subtitles, probe and encode run locally, the
//! slow ones on their own thread. Result files are content-addressed inside
//! the run's working directory.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use todo_ai::{AiEvent, AiService};
use todo_extensions::{PoolHandle, RunId};
use todo_media::{AlphaPacking, EncodeQuality, MediaService, MediaTools};
use todo_production::extension::protocol::{ServiceCall, ServiceError, ServiceErrorClass};
use todo_production::extension::services::{speech_result, subtitles_result, task_for};
use todo_production::extension::HostDefaults;
use todo_production::tasks::fingerprint;
use todo_production::voice::{read_wav, Pcm, Timeline};
use todo_production::TaskKey;

#[derive(Debug, Clone)]
struct PendingCall {
    run: RunId,
    call: String,
    service: ServiceCall,
    workdir: PathBuf,
}

/// Speech timelines by file (needed by a later `subtitles` call).
type Timelines = Arc<Mutex<HashMap<PathBuf, Timeline>>>;

pub struct ServiceBridge {
    defaults: HostDefaults,
    runs: HashMap<RunId, PathBuf>,
    pending: HashMap<TaskKey, Vec<PendingCall>>,
    timelines: Timelines,
    tools: MediaTools,
    pool: PoolHandle,
}

fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError {
        class: ServiceErrorClass::Invalid,
        message: message.into(),
    }
}

/// Hard link (same volume) or copy; an existing file with the same content
/// address is already right.
pub fn materialize(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.is_file() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if fs::hard_link(source, destination).is_err() {
        let partial = destination.with_extension("part");
        fs::copy(source, &partial).map_err(|e| e.to_string())?;
        fs::rename(&partial, destination).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `services/<key>.<ext>`: the same request always lands on the same file.
pub fn service_file(key: &TaskKey, ext: &str) -> String {
    format!(
        "services/{}.{}",
        key.0,
        if ext.is_empty() { "bin" } else { ext }
    )
}

impl ServiceBridge {
    pub fn new(defaults: HostDefaults, root: &Path, pool: PoolHandle) -> Self {
        Self {
            defaults,
            runs: HashMap::new(),
            pending: HashMap::new(),
            timelines: Arc::default(),
            tools: MediaTools::discover(root),
            pool,
        }
    }

    pub fn register_run(&mut self, run: RunId, workdir: PathBuf) {
        self.runs.insert(run, workdir);
    }

    /// Forget a run; returns the AI tasks only it was waiting for.
    pub fn remove_run(&mut self, run: RunId) -> Vec<TaskKey> {
        self.runs.remove(&run);
        let mut orphaned = Vec::new();
        self.pending.retain(|key, calls| {
            calls.retain(|c| c.run != run);
            if calls.is_empty() {
                orphaned.push(key.clone());
            }
            !calls.is_empty()
        });
        orphaned
    }

    /// A worker's service call.
    pub fn call(&mut self, ai: &AiService, run: RunId, call: String, service: ServiceCall) {
        let Some(workdir) = self.runs.get(&run).cloned() else {
            self.pool.reply(run, call, Err(invalid("运行已结束")));
            return;
        };
        match &service {
            ServiceCall::Subtitles(request) => {
                let result = (|| {
                    let path = workdir.join(&request.speech);
                    let timeline = self
                        .timelines
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&path)
                        .cloned()
                        .ok_or_else(|| invalid("speech 文件不是本次运行的配音结果"))?;
                    let bytes = fs::read(&path).map_err(|e| invalid(e.to_string()))?;
                    let (samples, rate) =
                        read_wav(&bytes).ok_or_else(|| invalid("配音不是 PCM16 WAV"))?;
                    Ok(subtitles_result(
                        request,
                        &timeline,
                        Pcm {
                            samples: &samples,
                            rate,
                        },
                    ))
                })();
                self.pool.reply(run, call, result);
            }
            ServiceCall::Probe(_) | ServiceCall::Encode(_) => {
                // Encoding takes seconds: never on the UI thread.
                let pool = self.pool.clone();
                let tools = self.tools.clone();
                std::thread::spawn(move || {
                    let result = local_service(&service, &workdir, tools);
                    pool.reply(run, call, result);
                });
            }
            ServiceCall::Chat(_) | ServiceCall::Image(_) | ServiceCall::Speech(_) => {
                let Some(task) = task_for(&service, &self.defaults) else {
                    self.pool.reply(run, call, Err(invalid("宿主不支持该服务")));
                    return;
                };
                let key = ai.submit(task);
                self.pending.entry(key).or_default().push(PendingCall {
                    run,
                    call,
                    service,
                    workdir,
                });
            }
        }
    }

    /// Answer the calls waiting for a finished AI task.
    pub fn on_ai_event(&mut self, ai: &AiService, event: &AiEvent) {
        let key = match event {
            AiEvent::Done { key, .. }
            | AiEvent::Failed { key, .. }
            | AiEvent::Cancelled { key } => key,
            _ => return,
        };
        let Some(calls) = self.pending.remove(key) else {
            return;
        };
        for pending in calls {
            let result = match event {
                AiEvent::Done {
                    artifact, output, ..
                } => self.done(ai, key, &pending, artifact.as_ref(), output.clone()),
                AiEvent::Failed {
                    class,
                    message,
                    possibly_billed,
                    ..
                } => Err(ServiceError {
                    class: match class.as_str() {
                        "content_policy" => ServiceErrorClass::ContentPolicy,
                        "invalid" => ServiceErrorClass::Invalid,
                        _ => ServiceErrorClass::Unavailable,
                    },
                    message: if *possibly_billed {
                        format!("{message}（结果未知，可能已扣费；需用户确认后重试）")
                    } else {
                        message.clone()
                    },
                }),
                _ => Err(ServiceError {
                    class: ServiceErrorClass::Unavailable,
                    message: "已取消".into(),
                }),
            };
            self.pool.reply(pending.run, pending.call, result);
        }
    }

    fn done(
        &mut self,
        ai: &AiService,
        key: &TaskKey,
        pending: &PendingCall,
        artifact: Option<&todo_ai::ArtifactRef>,
        output: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, ServiceError> {
        match &pending.service {
            ServiceCall::Chat(_) => Ok(output.unwrap_or_else(|| serde_json::json!({ "text": "" }))),
            ServiceCall::Image(_) => {
                let artifact = artifact.ok_or_else(|| invalid("生图没有返回图片"))?;
                let relative = service_file(key, &artifact.ext);
                materialize(
                    &ai.artifact_path(artifact),
                    &pending.workdir.join(&relative),
                )
                .map_err(invalid)?;
                let mut result = output
                    .filter(|v| v.is_object())
                    .unwrap_or_else(|| serde_json::json!({}));
                result["file"] = relative.into();
                if let Ok((w, h)) = image_size(
                    &pending
                        .workdir
                        .join(result["file"].as_str().unwrap_or_default()),
                ) {
                    result["width"] = w.into();
                    result["height"] = h.into();
                }
                Ok(result)
            }
            ServiceCall::Speech(call) => {
                let artifact = artifact.ok_or_else(|| invalid("配音没有返回音频"))?;
                let relative = service_file(key, "wav");
                let destination = pending.workdir.join(&relative);
                materialize(&ai.artifact_path(artifact), &destination).map_err(invalid)?;
                let bytes = fs::read(&destination).map_err(|e| invalid(e.to_string()))?;
                let (samples, rate) = read_wav(&bytes).ok_or_else(|| invalid("配音不是 WAV"))?;
                let raw = output.unwrap_or_else(|| serde_json::json!({}));
                let result = speech_result(
                    &call.text,
                    &raw,
                    Pcm {
                        samples: &samples,
                        rate,
                    },
                    &relative,
                );
                if let Some(timeline) = result
                    .get("timeline")
                    .and_then(|t| serde_json::from_value::<Timeline>(t.clone()).ok())
                {
                    self.timelines
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(destination, timeline);
                }
                Ok(result)
            }
            _ => Err(invalid("不是 AI 服务")),
        }
    }
}

fn image_size(path: &Path) -> Result<(u32, u32), String> {
    todo_media::image_size(path).map_err(|e| e.to_string())
}

fn local_service(
    service: &ServiceCall,
    workdir: &Path,
    tools: MediaTools,
) -> Result<serde_json::Value, ServiceError> {
    let inside = |path: &str| todo_production::extension::manifest::valid_relative_path(path);
    let media = MediaService::with_tools(workdir, tools);
    match service {
        ServiceCall::Probe(call) => {
            if !inside(&call.file) {
                return Err(invalid("probe 的 file 必须是工作目录内的相对路径"));
            }
            let metadata = media
                .probe_media(&call.file)
                .map_err(|e| invalid(e.to_string()))?;
            Ok(serde_json::json!({
                "duration": metadata.duration,
                "width": metadata.width,
                "height": metadata.height,
                "hasAudio": metadata.has_audio,
                "alpha": metadata.alpha.map(|alpha| match alpha {
                    AlphaPacking::Stacked => "stacked",
                    AlphaPacking::Reveal => "reveal",
                }),
            }))
        }
        ServiceCall::Encode(call) => {
            if call.audio.as_deref().is_some_and(|a| !inside(a)) {
                return Err(invalid("encode 的 audio 必须是工作目录内的相对路径"));
            }
            let frames = workdir.join(&call.frames);
            // Same frames and settings → same file; a rerun reuses it.
            let id = fingerprint(&serde_json::json!({
                "frames": call.frames, "fps": call.fps, "alpha": call.alpha,
                "audio": call.audio, "quality": format!("{:?}", call.quality),
                "stamp": frames_stamp(&frames),
            }));
            let relative = format!("services/encode-{}.mp4", &id[..32]);
            let output = workdir.join(&relative);
            let quality = match call.quality {
                todo_production::extension::protocol::EncodeQuality::Standard => {
                    EncodeQuality::Standard
                }
                todo_production::extension::protocol::EncodeQuality::High => EncodeQuality::High,
            };
            let info = media
                .encode_frames(
                    &frames,
                    call.fps,
                    call.alpha,
                    quality,
                    call.audio.as_ref().map(|a| workdir.join(a)).as_deref(),
                    &output,
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                )
                .map_err(|e| invalid(e.to_string()))?;
            Ok(serde_json::json!({
                "file": relative,
                "duration": info.duration,
                "width": info.width,
                "height": info.height,
            }))
        }
        _ => Err(invalid("不是本地服务")),
    }
}

/// Names, sizes and modification times of a frame directory: frames written
/// again (new content) give a new encode.
fn frames_stamp(dir: &Path) -> Vec<(String, u64, u128)> {
    let mut out: Vec<(String, u64, u128)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            let modified = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos();
            Some((
                entry.file_name().to_string_lossy().into_owned(),
                meta.len(),
                modified,
            ))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageFormat, Rgba, RgbaImage};
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use todo_ai::config::Limits;
    use todo_ai::net::{BreakerPolicy, RetryPolicy};
    use todo_ai::{ArtifactStore, ProxyMode, Secret, ServiceSettings};
    use todo_extensions::{PoolOptions, RuntimePaths, WorkerEvent, WorkerPool};
    use todo_production::extension::protocol::{ChatCall, ImageCall};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn bridge_fixture(name: &str) -> (ServiceBridge, todo_ai::AiService, WorkerPool, PathBuf) {
        bridge_fixture_at(name, "http://127.0.0.1:1")
    }

    fn bridge_fixture_at(
        name: &str,
        base_url: &str,
    ) -> (ServiceBridge, todo_ai::AiService, WorkerPool, PathBuf) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/bridge-tests")
            .join(format!(
                "{}-{}-{}",
                name,
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let (events, _received) = std::sync::mpsc::channel::<WorkerEvent>();
        let workers = WorkerPool::start(
            PoolOptions {
                runtime: RuntimePaths::new(root.clone()),
                ..PoolOptions::default()
            },
            events,
        );
        let (ai, _) = todo_ai::AiService::start(ServiceSettings {
            huniu_base_url: base_url.into(),
            volc_base_url: base_url.into(),
            huniu_strategy: None,
            keys: HashMap::from([
                (
                    todo_production::Provider::Huniu,
                    Secret::new("test-key").unwrap(),
                ),
                (
                    todo_production::Provider::Volc,
                    Secret::new("test-volc").unwrap(),
                ),
            ]),
            journal_dir: root.join("tasks"),
            artifacts_root: root.join("blobs"),
            proxy: ProxyMode::Direct,
            limits: Limits {
                chat_inflight: 1,
                image_inflight: 1,
                image_rate_per_second: 100.0,
                speech_inflight: 1,
                image_paid_retries: 1,
            },
            retry: RetryPolicy {
                base: Duration::from_millis(10),
                cap: Duration::from_millis(40),
                max_retry_after: Duration::from_millis(200),
                paid_retries: 1,
            },
            breaker: BreakerPolicy::default(),
        })
        .unwrap();
        let bridge = ServiceBridge::new(
            HostDefaults {
                chat_model: "chat-test".into(),
                image_model: "image-test".into(),
                voice_resource: "voice-test".into(),
                sample_rate: 24_000,
            },
            &root,
            workers.handle(),
        );
        (bridge, ai, workers, root)
    }

    fn image_call() -> ServiceCall {
        ServiceCall::Image(ImageCall {
            prompt: "a red square".into(),
            size: "128x96".into(),
            quality: None,
            transparent: false,
            variant: 0,
            model: None,
        })
    }

    fn fake_image_server() -> (String, Arc<AtomicU64>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let generation_calls = Arc::new(AtomicU64::new(0));
        let calls = generation_calls.clone();
        let thread_base = base.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve_fake_image(stream, &thread_base, &calls);
            }
        });
        (base, generation_calls)
    }

    fn serve_fake_image(stream: TcpStream, base: &str, calls: &AtomicU64) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        if reader.read_line(&mut request).is_err() {
            return;
        }
        let path = request.split_whitespace().nth(1).unwrap_or_default();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; content_length];
        let _ = reader.read_exact(&mut body);
        let (status, content_type, response) = if path == "/v1/images/generations" {
            let attempt = calls.fetch_add(1, Ordering::Relaxed);
            if attempt == 0 {
                (503, "text/plain", b"busy".to_vec())
            } else {
                (
                    200,
                    "application/json",
                    format!(r#"{{"data":[{{"url":"{base}/files/image.png"}}]}}"#).into_bytes(),
                )
            }
        } else if path == "/files/image.png" {
            let mut png = Vec::new();
            RgbaImage::from_pixel(128, 96, Rgba([220, 30, 40, 255]))
                .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
                .unwrap();
            (200, "image/png", png)
        } else {
            (404, "text/plain", b"missing".to_vec())
        };
        let head = format!(
            "HTTP/1.1 {status} X\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
            response.len()
        );
        let mut stream = stream;
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&response);
    }

    #[test]
    fn service_files_are_content_addressed() {
        let key = TaskKey("ab12".into());
        assert_eq!(service_file(&key, "png"), "services/ab12.png");
        assert_eq!(service_file(&key, ""), "services/ab12.bin");
    }

    #[test]
    fn materialize_links_once_and_keeps_existing_files() {
        // Some Windows runners deny creating child directories in the system
        // temp folder. Keep this tiny fixture under the workspace target,
        // where the test process already has write access.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/bridge-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("a.bin");
        fs::write(&source, b"one").unwrap();
        let target = dir.join("w/services/a.bin");
        materialize(&source, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"one");
        materialize(&source, &target).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bridge_returns_chat_results_and_materializes_image_artifacts() {
        let (mut bridge, ai, workers, root) = bridge_fixture("roundtrip");
        let workdir = root.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let chat = PendingCall {
            run: 1,
            call: "chat-call".into(),
            service: ServiceCall::Chat(ChatCall {
                messages: Vec::new(),
                json: false,
                temperature: None,
                model: None,
            }),
            workdir: workdir.clone(),
        };
        let chat_result = bridge
            .done(
                &ai,
                &TaskKey("chat-key".into()),
                &chat,
                None,
                Some(serde_json::json!({"text": "好的"})),
            )
            .unwrap();
        assert_eq!(chat_result["text"], "好的");

        let mut png = Vec::new();
        RgbaImage::from_pixel(128, 96, Rgba([220, 30, 40, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let store = ArtifactStore::new(root.join("blobs"));
        let artifact = store
            .put("image-key", &png, "png", |path| {
                image::load_from_memory(&fs::read(path).map_err(|error| error.to_string())?)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
            .unwrap();
        let image_pending = PendingCall {
            run: 1,
            call: "image-call".into(),
            service: image_call(),
            workdir: workdir.clone(),
        };
        let image_result = bridge
            .done(
                &ai,
                &TaskKey("image-key".into()),
                &image_pending,
                Some(&artifact),
                Some(serde_json::json!({"model": "fake"})),
            )
            .unwrap();
        let relative = image_result["file"].as_str().unwrap();
        assert_eq!(image_result["width"], 128);
        assert_eq!(image_result["height"], 96);
        assert_eq!(fs::read(workdir.join(relative)).unwrap(), png);

        // A rerun with the same task key is idempotent: an existing user file
        // is retained rather than overwritten by the cached artifact.
        fs::write(workdir.join(relative), b"user-kept").unwrap();
        bridge
            .done(
                &ai,
                &TaskKey("image-key".into()),
                &image_pending,
                Some(&artifact),
                None,
            )
            .unwrap();
        assert_eq!(fs::read(workdir.join(relative)).unwrap(), b"user-kept");
        drop(bridge);
        drop(ai);
        drop(workers);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bridge_maps_unavailable_ai_failures_to_worker_errors() {
        let (mut bridge, ai, workers, root) = bridge_fixture("failure");
        let workdir = root.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let pending = PendingCall {
            run: 2,
            call: "chat-call".into(),
            service: ServiceCall::Chat(ChatCall {
                messages: Vec::new(),
                json: false,
                temperature: None,
                model: None,
            }),
            workdir,
        };
        let key = TaskKey("failed-key".into());
        bridge.pending.insert(key.clone(), vec![pending]);
        // The pool manager is deliberately not running a worker; this only
        // verifies classification and that the bridge consumes the pending
        // entry without panicking. The real 503/retry sequence is covered by
        // todo_ai's FakeServer tests.
        bridge.on_ai_event(
            &ai,
            &AiEvent::Failed {
                key,
                class: "unavailable".into(),
                message: "503 busy".into(),
                possibly_billed: false,
            },
        );
        assert!(bridge.pending.is_empty());
        drop(bridge);
        drop(ai);
        drop(workers);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bridge_roundtrip_recovers_from_image_503_and_persists_the_artifact() {
        let (base, generation_calls) = fake_image_server();
        let (mut bridge, ai, workers, root) = bridge_fixture_at("fake-server", &base);
        let workdir = root.join("work");
        fs::create_dir_all(&workdir).unwrap();
        bridge.register_run(7, workdir.clone());
        bridge.call(&ai, 7, "image-call".into(), image_call());

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut finished = false;
        while std::time::Instant::now() < deadline {
            if let Some(event) = ai.recv_timeout(Duration::from_millis(50)) {
                let terminal = matches!(
                    event,
                    AiEvent::Done { .. } | AiEvent::Failed { .. } | AiEvent::Cancelled { .. }
                );
                bridge.on_ai_event(&ai, &event);
                if terminal {
                    finished = true;
                    break;
                }
            }
        }
        assert!(finished, "AI image request did not finish");
        assert_eq!(generation_calls.load(Ordering::Relaxed), 2);
        let services = workdir.join("services");
        let artifact = fs::read_dir(&services)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|ext| ext == "png"))
            .expect("bridge should materialize image artifact");
        assert_eq!(image::open(&artifact).unwrap().dimensions(), (128, 96));
        assert!(bridge.pending.is_empty());
        drop(bridge);
        drop(ai);
        drop(workers);
        let _ = fs::remove_dir_all(root);
    }
}
