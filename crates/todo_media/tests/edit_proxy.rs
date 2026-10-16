use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use todo_media::{MediaService, MediaTools};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    tools: MediaTools,
    service: MediaService,
}
impl Fixture {
    fn new() -> Self {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let root = repo.join("target/edit-proxy-tests").join(format!(
            "{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let tools = MediaTools::discover(&repo);
        let service = MediaService::with_tools(&root, tools.clone());
        Self {
            root,
            tools,
            service,
        }
    }
    fn generate(&self, name: &str, args: &[&str]) -> PathBuf {
        let path = self.root.join(name);
        let result = Command::new(&self.tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
            .args(args)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        path
    }
    fn video(&self, color: &str) -> PathBuf {
        self.generate(
            "视频 一.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={color}:s=1920x1080:r=10:d=0.5"),
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ],
        )
    }
    fn inspect(&self, path: &Path) -> Value {
        let result = Command::new(&self.tools.ffprobe)
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-show_frames",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    }
    fn missing_encoder(&self) -> MediaService {
        MediaService::with_tools(
            &self.root,
            MediaTools {
                ffmpeg: self.root.join("missing-encoder.exe"),
                ffprobe: self.tools.ffprobe.clone(),
            },
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let boundary = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/edit-proxy-tests")
            .canonicalize()
            .unwrap();
        if let Ok(path) = self.root.canonicalize() {
            if path.parent() == Some(boundary.as_path()) {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}
fn number(value: &Value) -> f64 {
    value.as_str().unwrap().parse().unwrap()
}
fn dar(stream: &Value) -> f64 {
    let sar: Vec<f64> = stream["sample_aspect_ratio"]
        .as_str()
        .unwrap()
        .split(':')
        .map(|v| v.parse().unwrap())
        .collect();
    stream["width"].as_f64().unwrap() / stream["height"].as_f64().unwrap() * sar[0] / sar[1]
}

#[test]
fn proxy_is_bounded_all_intra_video_and_does_not_modify_source() {
    let fixture = Fixture::new();
    let source = fixture.video("red");
    let original = fs::read(&source).unwrap();
    let proxy = fixture
        .service
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    assert_eq!((proxy.source_width, proxy.source_height), (1920, 1080));
    assert_eq!((proxy.width, proxy.height), (1280, 720));
    assert_ne!(proxy.path, source);
    let inspected = fixture.inspect(&proxy.path);
    assert_eq!(inspected["streams"].as_array().unwrap().len(), 1);
    assert_eq!(inspected["streams"][0]["codec_name"], "h264");
    let frames = inspected["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 5);
    assert!(frames
        .iter()
        .all(|f| f["key_frame"] == 1 && f["pict_type"] == "I"));
    assert_eq!(fs::read(source).unwrap(), original);
}

#[test]
fn cached_proxy_is_reused_and_source_replacement_gets_a_new_identity() {
    let fixture = Fixture::new();
    let source = fixture.video("red");
    let first = fixture
        .service
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    let stamp = fs::metadata(&first.path).unwrap().modified().unwrap();
    let cached = fixture
        .missing_encoder()
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    assert_eq!(cached.path, first.path);
    assert_eq!(
        fs::metadata(&cached.path).unwrap().modified().unwrap(),
        stamp
    );
    fixture.video("blue");
    assert!(fixture
        .missing_encoder()
        .get_edit_proxy(source.to_str().unwrap())
        .is_err());
    let replaced = fixture
        .service
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    assert_ne!(replaced.path, first.path);
}

#[test]
fn proxy_preserves_vfr_intervals_anamorphic_display_aspect_and_normalized_pts() {
    let fixture = Fixture::new();
    let source = fixture.generate(
        "vfr-offset.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1440x1080:r=10:d=1",
            "-vf",
            "select='not(eq(n,2)+eq(n,5))',setpts=PTS+5/TB,setsar=4/3",
            "-fps_mode",
            "passthrough",
            "-enc_time_base",
            "1:1000",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-avoid_negative_ts",
            "disabled",
        ],
    );
    let before = fixture.inspect(&source);
    let proxy = fixture
        .service
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    let after = fixture.inspect(&proxy.path);
    assert!((dar(&before["streams"][0]) - dar(&after["streams"][0])).abs() < 0.0001);
    let origin = number(&before["format"]["start_time"]);
    assert!((origin - 5.0).abs() < 0.001);
    let input_frames = before["frames"].as_array().unwrap();
    let output_frames = after["frames"].as_array().unwrap();
    assert_eq!(input_frames.len(), 8);
    assert_eq!(output_frames.len(), input_frames.len());
    for (input, output) in input_frames.iter().zip(output_frames) {
        assert!(
            (number(&input["best_effort_timestamp_time"])
                - origin
                - number(&output["best_effort_timestamp_time"]))
            .abs()
                < 0.001
        );
    }
}

#[test]
fn corrupt_cache_and_failed_generation_never_reuse_or_leave_partial_outputs() {
    let fixture = Fixture::new();
    let source = fixture.video("red");
    let original = fs::read(&source).unwrap();
    let first = fixture
        .service
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    fs::write(&first.path, b"broken mp4").unwrap();
    assert!(fixture
        .missing_encoder()
        .get_edit_proxy(source.to_str().unwrap())
        .is_err());
    let regenerated = fixture
        .service
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    assert_eq!(regenerated.path, first.path);
    assert_eq!(
        fixture.inspect(&regenerated.path)["streams"][0]["codec_name"],
        "h264"
    );
    fixture.video("blue");
    assert!(fixture
        .service
        .clone()
        .with_tool_timeout(std::time::Duration::ZERO)
        .get_edit_proxy(source.to_str().unwrap())
        .is_err());
    assert!(fs::read_dir(first.path.parent().unwrap())
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp.")));
    assert_ne!(fs::read(&source).unwrap(), original);
    assert!(fixture
        .service
        .probe_media(source.to_str().unwrap())
        .is_ok());
    let invalid = fixture.root.join("bad.mp4");
    fs::write(&invalid, b"invalid video").unwrap();
    assert!(fixture
        .service
        .get_edit_proxy(invalid.to_str().unwrap())
        .is_err());
}

#[test]
fn concurrent_requests_share_one_atomic_proxy() {
    let fixture = Fixture::new();
    let source = fixture.video("red");
    let service = Arc::new(fixture.service.clone());
    let workers: Vec<_> = (0..3)
        .map(|_| {
            let service = service.clone();
            let source = source.clone();
            std::thread::spawn(move || service.get_edit_proxy(source.to_str().unwrap()).unwrap())
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(results.windows(2).all(|pair| pair[0].path == pair[1].path));
    assert_eq!(
        fs::read_dir(results[0].path.parent().unwrap())
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "mp4"))
            .count(),
        1
    );
}

#[test]
fn video_offset_relative_to_original_audio_is_preserved() {
    let fixture = Fixture::new();
    let source = fixture.generate(
        "av-offset.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=96x64:r=10:d=0.6",
            "-f",
            "lavfi",
            "-i",
            "sine=sample_rate=48000:duration=1",
            "-filter:v",
            "setpts=PTS+0.3/TB",
            "-fps_mode",
            "passthrough",
            "-enc_time_base",
            "1:1000",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-c:a",
            "aac",
        ],
    );
    let before = fixture.inspect(&source);
    let proxy = fixture
        .service
        .get_edit_proxy(source.to_str().unwrap())
        .unwrap();
    assert_eq!((proxy.width, proxy.height), (96, 64));
    let after = fixture.inspect(&proxy.path);
    let input: Vec<_> = before["frames"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|frame| frame["media_type"] == "video")
        .collect();
    let output = after["frames"].as_array().unwrap();
    assert_eq!(input.len(), output.len());
    let origin = number(&before["format"]["start_time"]);
    for (input, output) in input.iter().zip(output) {
        assert!(
            (number(&input["best_effort_timestamp_time"])
                - origin
                - number(&output["best_effort_timestamp_time"]))
            .abs()
                < 0.001
        );
    }
    assert!(number(&output[0]["best_effort_timestamp_time"]) > 0.1);
    assert_eq!(after["streams"].as_array().unwrap().len(), 1);
}

#[test]
fn cancelled_proxy_job_keeps_source_and_publishes_nothing() {
    let fixture = Fixture::new();
    let source = fixture.video("red");
    let before = fs::read(&source).unwrap();
    let cancelled = Arc::new(AtomicBool::new(true));
    assert!(matches!(
        fixture
            .service
            .get_edit_proxy_cancellable(source.to_str().unwrap(), cancelled),
        Err(todo_media::MediaError::Cancelled)
    ));
    assert_eq!(fs::read(source).unwrap(), before);
    assert!(!fixture.root.join("cache/media/editproxies").exists());
}

#[test]
fn still_images_and_audio_are_left_on_the_original_media_path() {
    let fixture = Fixture::new();
    let still = fixture.generate(
        "still.png",
        &["-f", "lavfi", "-i", "color=c=red:s=96x64", "-frames:v", "1"],
    );
    let audio = fixture.generate("audio.wav", &["-f", "lavfi", "-i", "sine=duration=0.1"]);
    for source in [still, audio] {
        let original = fs::read(&source).unwrap();
        assert!(fixture
            .service
            .get_edit_proxy(source.to_str().unwrap())
            .is_err());
        assert_eq!(fs::read(&source).unwrap(), original);
    }
}

#[test]
fn hdr_transfer_functions_fall_back_to_original_without_losing_color_metadata() {
    let fixture = Fixture::new();
    for transfer in ["smpte2084", "arib-std-b67"] {
        let source = fixture.generate(
            &format!("hdr-{transfer}.mp4"),
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=96x64:r=10:d=0.2",
                "-c:v",
                "libx264",
                "-x264-params",
                &format!("colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc"),
            ],
        );
        assert_eq!(
            fixture.inspect(&source)["streams"][0]["color_transfer"],
            transfer
        );
        let original = fs::read(&source).unwrap();
        let error = fixture
            .service
            .get_edit_proxy(source.to_str().unwrap())
            .expect_err("HDR must use the original until tone mapping is implemented");
        assert!(error.to_string().contains("HDR"));
        assert_eq!(fs::read(&source).unwrap(), original);
    }
}

#[test]
fn cancellation_after_creating_pending_output_removes_the_partial_file() {
    let fixture = Fixture::new();
    let source = fixture.generate(
        "cancel.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=30:d=2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ],
    );
    let original = fs::read(&source).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_service = fixture.service.clone();
    let worker_source = source.clone();
    let worker_flag = cancelled.clone();
    let worker = std::thread::spawn(move || {
        worker_service.get_edit_proxy_cancellable(worker_source.to_str().unwrap(), worker_flag)
    });
    let directory = fixture.root.join("cache/media/editproxies");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut saw_pending = false;
    while std::time::Instant::now() < deadline {
        if fs::read_dir(&directory).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp.mp4"))
        }) {
            saw_pending = true;
            break;
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    cancelled.store(true, Ordering::Release);
    let result = worker.join().unwrap();
    assert!(
        saw_pending,
        "the test must exercise a proxy job with a pending output"
    );
    assert!(matches!(result, Err(todo_media::MediaError::Cancelled)));
    assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), original);
}
