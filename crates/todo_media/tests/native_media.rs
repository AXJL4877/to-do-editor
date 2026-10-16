use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use todo_media::{MediaService, MediaTools};
use todo_storage::ProjectStore;

static SEQ: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    service: MediaService,
    tools: MediaTools,
}
impl Fixture {
    fn new() -> Self {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let root = repo.join("target/native-media-tests").join(format!(
            "{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        ProjectStore::new(&root).create("工程 一").unwrap();
        let tools = MediaTools::discover(&repo);
        let service = MediaService::with_tools(&root, tools.clone());
        Self {
            root,
            service,
            tools,
        }
    }
    fn generate(&self, name: &str, arguments: &[&str]) -> PathBuf {
        let path = self.root.join(name);
        let result = Command::new(&self.tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
            .args(arguments)
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
    fn image(&self) -> PathBuf {
        self.generate(
            "封面 一.png",
            &["-f", "lavfi", "-i", "color=c=red:s=96x64", "-frames:v", "1"],
        )
    }
    fn audio(&self) -> PathBuf {
        self.generate(
            "声音 一.wav",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
            ],
        )
    }
    fn files(&self) -> Vec<PathBuf> {
        fs::read_dir(self.root.join("data/工程 一/media"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let boundary = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/native-media-tests")
            .canonicalize()
            .unwrap();
        if let Ok(path) = self.root.canonicalize() {
            if path.parent() == Some(boundary.as_path()) {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}

#[test]
fn local_import_preserves_unicode_metadata_bytes_and_relative_project_paths() {
    let fixture = Fixture::new();
    let source = fixture.image();
    let bytes = fs::read(&source).unwrap();
    let item = fixture
        .service
        .import_file("工程 一.json", &source)
        .unwrap();
    assert_eq!(item.name, "封面 一.png");
    assert_eq!(item.kind, Some(todo_core::MediaKind::Image));
    assert_eq!(
        (item.duration, item.has_audio, item.width, item.height),
        (5.0, Some(false), Some(96), Some(64))
    );
    let stored = item.path.unwrap();
    assert!(stored.starts_with("data/工程 一/media/"));
    assert!(!stored.contains('\\'));
    assert_eq!(fs::read(fixture.root.join(stored)).unwrap(), bytes);
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fixture.files().len(), 1);
    let second = fixture.service.import_file("工程 一", &source).unwrap();
    assert_ne!(second.id, item.id);
    assert_eq!(fixture.files().len(), 2);
}

#[test]
fn invalid_empty_missing_and_directory_sources_publish_no_media() {
    let fixture = Fixture::new();
    let empty = fixture.root.join("empty.mp4");
    fs::write(&empty, b"").unwrap();
    let corrupt = fixture.root.join("corrupt.mp4");
    fs::write(&corrupt, b"not a media file").unwrap();
    for path in [
        &empty,
        &corrupt,
        &fixture.root.join("missing.mp4"),
        &fixture.root,
    ] {
        assert!(fixture.service.import_file("工程 一", path).is_err());
        assert!(fixture.files().is_empty());
    }
}

#[test]
fn project_traversal_deleted_projects_and_relative_media_escapes_are_rejected() {
    let fixture = Fixture::new();
    let source = fixture.image();
    for project in ["../escape", "..", ".trash", "CON", "bad/name"] {
        assert!(fixture.service.import_file(project, &source).is_err());
    }
    ProjectStore::new(&fixture.root).delete("工程 一").unwrap();
    assert!(fixture.service.import_file("工程 一", &source).is_err());
    assert!(!fixture.root.join("data/工程 一").exists());
    assert!(fixture.service.probe_media("../escape.mp4").is_err());
    assert!(fixture.service.probe_media(".").is_err());
}

#[test]
fn video_probe_does_not_misclassify_mjpeg_as_a_still_image() {
    let fixture = Fixture::new();
    let video = fixture.generate(
        "mjpeg.avi",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=96x64:r=10:d=1",
            "-c:v",
            "mjpeg",
            "-an",
        ],
    );
    let probe = fixture
        .service
        .probe_media(video.to_str().unwrap())
        .unwrap();
    assert_eq!(probe.kind, todo_core::MediaKind::Video);
    assert!((probe.duration - 1.0).abs() < 0.01);
    assert!(!probe.has_audio);
    let item = fixture.service.import_file("工程 一", &video).unwrap();
    assert_eq!(item.kind, Some(todo_core::MediaKind::Video));
}

#[test]
fn thumbnail_cache_depends_on_dimensions_and_source_replacement() {
    let fixture = Fixture::new();
    let source = fixture.image();
    let path = source.to_str().unwrap();
    let first = fixture.service.get_thumb(path, 64).unwrap();
    let different_width = fixture.service.get_thumb(path, 128).unwrap();
    assert_eq!(image::load_from_memory(&first).unwrap().width(), 64);
    assert_eq!(
        image::load_from_memory(&different_width).unwrap().width(),
        128
    );
    let missing_tools = MediaTools {
        ffmpeg: fixture.root.join("missing.exe"),
        ffprobe: fixture.root.join("missing-probe.exe"),
    };
    let cached = MediaService::with_tools(&fixture.root, missing_tools);
    assert_eq!(cached.get_thumb(path, 64).unwrap(), first);
    fixture.generate(
        "封面 一.png",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=128x96",
            "-frames:v",
            "1",
        ],
    );
    assert!(
        cached.get_thumb(path, 64).is_err(),
        "Replacing a source must invalidate the old cached thumbnail"
    );
    assert_ne!(fixture.service.get_thumb(path, 64).unwrap(), first);
}

#[test]
fn stereo_waveforms_use_finite_bounded_buckets_and_support_cache_reads_without_tools() {
    let fixture = Fixture::new();
    let source = fixture.audio();
    let path = source.to_str().unwrap();
    let waveform = fixture.service.get_peaks(path, 32).unwrap();
    assert!(waveform.has_audio);
    assert_eq!(waveform.peaks.len(), 32);
    assert!(waveform
        .peaks
        .iter()
        .all(|peak| peak.is_finite() && *peak > 0.05 && *peak < 0.2));
    assert!((waveform.duration - 1.0).abs() < 0.01);
    let missing = MediaTools {
        ffmpeg: fixture.root.join("missing.exe"),
        ffprobe: fixture.root.join("missing-probe.exe"),
    };
    assert_eq!(
        MediaService::with_tools(&fixture.root, missing)
            .get_peaks(path, 32)
            .unwrap(),
        waveform
    );
    assert_eq!(fixture.service.get_peaks(path, 0).unwrap().peaks.len(), 1);
    assert_eq!(
        fixture.service.get_peaks(path, 99_999).unwrap().peaks.len(),
        16_384
    );
    let still = fixture.image();
    let no_audio = fixture
        .service
        .get_peaks(still.to_str().unwrap(), 32)
        .unwrap();
    assert!(!no_audio.has_audio);
    assert!(no_audio.peaks.is_empty());
}

#[test]
fn cache_unavailable_and_concurrent_requests_preserve_usable_results() {
    let fixture = Fixture::new();
    let source = fixture.image();
    let service = Arc::new(fixture.service.clone());
    let threads: Vec<_> = (0..3)
        .map(|_| {
            let service = service.clone();
            let source = source.clone();
            std::thread::spawn(move || service.get_thumb(source.to_str().unwrap(), 80).unwrap())
        })
        .collect();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
    let blocked = fixture.root.join("blocked-cache");
    fs::write(&blocked, b"a file").unwrap();
    let uncached = fixture.service.clone().with_cache_directory(blocked);
    assert!(!uncached
        .get_thumb(source.to_str().unwrap(), 96)
        .unwrap()
        .is_empty());
}

#[test]
fn corrupt_jpeg_cache_is_regenerated_even_when_outer_markers_are_present() {
    let fixture = Fixture::new();
    let source = fixture.image();
    let path = source.to_str().unwrap();
    fixture.service.get_thumb(path, 64).unwrap();
    let cached = fs::read_dir(fixture.root.join("cache/media/thumbnails"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&cached, [0xff, 0xd8, 0x00, 0xff, 0xd9]).unwrap();
    let regenerated = fixture.service.get_thumb(path, 64).unwrap();
    assert_eq!(image::load_from_memory(&regenerated).unwrap().width(), 64);
    assert_eq!(fs::read(&cached).unwrap(), regenerated);
}
