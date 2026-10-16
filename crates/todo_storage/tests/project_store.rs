use std::fs;
use std::path::{Path, PathBuf};

use todo_core::create_project;
use todo_storage::{ProjectStore, StorageError};
use uuid::Uuid;

const LEGACY: &[u8] = include_bytes!("fixtures/legacy.json");

struct TestRepo {
    root: PathBuf,
    boundary: PathBuf,
}

impl TestRepo {
    fn new() -> Self {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let boundary = repo.join("target/todo-storage-tests");
        fs::create_dir_all(&boundary).unwrap();
        let boundary = boundary.canonicalize().unwrap();
        assert!(boundary.starts_with(&repo));
        let root = boundary.join(Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        Self { root, boundary }
    }

    fn store(&self) -> ProjectStore {
        ProjectStore::new(&self.root)
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        if let Ok(root) = self.root.canonicalize() {
            assert!(root.starts_with(&self.boundary));
            assert_eq!(root.parent(), Some(self.boundary.as_path()));
            let _ = fs::remove_dir_all(root);
        }
    }
}

#[test]
fn listing_absent_data_is_empty_without_creating_directories() {
    let repo = TestRepo::new();
    assert!(repo.store().list().unwrap().is_empty());
    assert!(!repo.data().exists());
}

#[test]
fn creates_chinese_names_and_normalizes_json_suffix() {
    let repo = TestRepo::new();
    let project = repo.store().create("剪辑 01.JSON").unwrap();
    assert_eq!(project.version, 1);
    assert_eq!(project.fps, 30.0);
    assert_eq!(project.width, 1920);
    assert_eq!(project.height, 1080);
    assert!(repo.data().join("剪辑 01/project.json").is_file());
    assert!(repo.data().join("剪辑 01/media").is_dir());
    assert_eq!(repo.store().list().unwrap(), vec!["剪辑 01"]);
    assert_eq!(repo.store().load("剪辑 01.json").unwrap(), project);
}

#[test]
fn invalid_names_never_create_or_access_a_project() {
    let repo = TestRepo::new();
    for name in [
        "",
        " ",
        ".",
        "..",
        ".json",
        "../outside",
        "a/b",
        "a\\b",
        "C:foo",
        "bad?",
        "bad*",
        "bad\"",
        "bad<",
        "bad>",
        "bad|",
        "bad\0",
        "bad\n",
        "bad.",
        "bad ",
        "CON",
        "nul.txt",
        "com1",
        "LPT9.ext",
        "COM¹",
        "LPT²",
        ".trash",
        ".TRASH.json",
    ] {
        assert!(
            matches!(repo.store().create(name), Err(StorageError::InvalidName(_))),
            "{name:?}"
        );
    }
    let project = create_project(None, None, None, None, None);
    assert!(matches!(
        repo.store().load("../outside"),
        Err(StorageError::InvalidName(_))
    ));
    assert!(matches!(
        repo.store().save("../outside", &project),
        Err(StorageError::InvalidName(_))
    ));
    assert!(matches!(
        repo.store().delete("../outside"),
        Err(StorageError::InvalidName(_))
    ));
    assert!(matches!(
        repo.store().import_json("../outside", LEGACY),
        Err(StorageError::InvalidName(_))
    ));
    assert!(!repo.data().exists());
}

#[test]
fn duplicate_creation_keeps_original_project() {
    let repo = TestRepo::new();
    let store = repo.store();
    let mut first = store.create("project").unwrap();
    first.playhead = 2.5;
    store.save("project", &first).unwrap();
    assert!(matches!(
        store.create("project.json"),
        Err(StorageError::AlreadyExists(_))
    ));
    assert!(matches!(
        store.import_json("project", LEGACY),
        Err(StorageError::AlreadyExists(_))
    ));
    assert_eq!(store.load("project").unwrap().playhead, 2.5);
}

#[test]
fn legacy_import_and_replacement_preserve_paths_and_optional_fields() {
    let repo = TestRepo::new();
    let store = repo.store();
    let mut project = store.import_json("旧工程", LEGACY).unwrap();
    assert_eq!(project.duration, None);
    assert_eq!(project.width, 1920);
    assert_eq!(project.height, 1080);
    assert_eq!(
        project.media[0].path.as_deref(),
        Some("data/旧工程/media/电影.mp4")
    );
    assert_eq!(project.media[0].has_audio, Some(true));
    assert_eq!(project.media[1].path, None);
    assert_eq!(project.media[1].has_audio, None);
    assert_eq!(project.tracks[0].clips[0].link_id, None);
    project.fps = 60.0;
    project.playhead = 3.0;
    store.save("旧工程", &project).unwrap();
    assert_eq!(store.load("旧工程").unwrap(), project);
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.data().join("旧工程/project.json")).unwrap())
            .unwrap();
    assert!(json.get("duration").is_none());
    assert!(json["media"][1].get("path").is_none());
    assert!(json["media"][1].get("hasAudio").is_none());
    assert_eq!(json["tracks"][0]["clips"][0]["mediaId"], "m1");
    let mut entries: Vec<_> = fs::read_dir(repo.data().join("旧工程"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["media", "project.json"]);
}

#[test]
fn unsupported_versions_are_rejected_before_any_write() {
    let repo = TestRepo::new();
    let store = repo.store();
    for version in [0_u64, 2, u64::from(u32::MAX) + 1] {
        let mut value: serde_json::Value = serde_json::from_slice(LEGACY).unwrap();
        value["version"] = version.into();
        assert!(
            matches!(store.import_json("bad", &serde_json::to_vec(&value).unwrap()), Err(StorageError::UnsupportedVersion(found)) if found == version)
        );
    }
    assert!(!repo.data().exists());
    let original = store.create("project").unwrap();
    let mut invalid = original.clone();
    invalid.version = 2;
    assert!(matches!(
        store.save("project", &invalid),
        Err(StorageError::UnsupportedVersion(2))
    ));
    assert_eq!(store.load("project").unwrap(), original);
}

#[test]
fn invalid_documents_cannot_overwrite_valid_state() {
    let repo = TestRepo::new();
    let store = repo.store();
    assert!(matches!(
        store.import_json("bad", b"{"),
        Err(StorageError::Json(_))
    ));
    let mut value: serde_json::Value = serde_json::from_slice(LEGACY).unwrap();
    value["fps"] = 0.into();
    assert!(matches!(
        store.import_json("bad", &serde_json::to_vec(&value).unwrap()),
        Err(StorageError::InvalidProject(_))
    ));
    assert!(!repo.data().exists());
    let original = store.create("project").unwrap();
    let mut invalid = original.clone();
    invalid.fps = f64::NAN;
    assert!(matches!(
        store.save("project", &invalid),
        Err(StorageError::InvalidProject(_))
    ));
    assert_eq!(store.load("project").unwrap(), original);
}

#[test]
fn listing_excludes_trash_and_incomplete_directories_and_sorts_names() {
    let repo = TestRepo::new();
    let store = repo.store();
    store.create("zeta").unwrap();
    store.create("alpha").unwrap();
    fs::create_dir(repo.data().join("incomplete")).unwrap();
    fs::create_dir(repo.data().join(".trash")).unwrap();
    fs::write(repo.data().join(".trash/project.json"), LEGACY).unwrap();
    fs::write(repo.data().join("stray.json"), LEGACY).unwrap();
    assert_eq!(store.list().unwrap(), vec!["alpha", "zeta"]);
}

#[test]
fn deletion_moves_document_and_media_to_unique_recoverable_trash() {
    let repo = TestRepo::new();
    let store = repo.store();
    store.import_json("删除我", LEGACY).unwrap();
    fs::write(repo.data().join("删除我/media/keep.mp4"), b"keep media").unwrap();
    store.delete("删除我").unwrap();
    assert!(!repo.data().join("删除我").exists());
    assert!(store.list().unwrap().is_empty());
    let first: Vec<_> = fs::read_dir(repo.data().join(".trash"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(first.len(), 1);
    assert_eq!(
        fs::read(first[0].join("media/keep.mp4")).unwrap(),
        b"keep media"
    );
    let recovered: todo_core::Project =
        serde_json::from_slice(&fs::read(first[0].join("project.json")).unwrap()).unwrap();
    assert_eq!(
        recovered.media[0].path.as_deref(),
        Some("data/旧工程/media/电影.mp4")
    );
    store.create("删除我").unwrap();
    store.delete("删除我").unwrap();
    assert_eq!(fs::read_dir(repo.data().join(".trash")).unwrap().count(), 2);
}

#[test]
fn missing_projects_report_not_found_without_recreating_them() {
    let repo = TestRepo::new();
    let store = repo.store();
    assert!(matches!(
        store.load("missing"),
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        store.save("missing", &create_project(None, None, None, None, None)),
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        store.delete("missing"),
        Err(StorageError::NotFound(_))
    ));
    assert!(!repo.data().exists());
}

#[test]
fn maximum_length_project_name_remains_deletable() {
    let repo = TestRepo::new();
    let name = "a".repeat(255);
    repo.store().create(&name).unwrap();
    repo.store().delete(&name).unwrap();
    assert!(repo.store().list().unwrap().is_empty());
    assert_eq!(fs::read_dir(repo.data().join(".trash")).unwrap().count(), 1);
}

#[test]
fn explicit_transform_and_link_survive_import_and_save() {
    let repo = TestRepo::new();
    let mut value: serde_json::Value = serde_json::from_slice(LEGACY).unwrap();
    value["tracks"][0]["clips"][0]["transform"] = serde_json::json!({ "x": 0.25, "y": -0.5, "scale": 1.75,"scaleX":0.5,"scaleY":2.0,"opacity":0.4 });
    value["tracks"][0]["clips"][0]["linkId"] = "linked".into();
    let project = repo
        .store()
        .import_json("transform", &serde_json::to_vec(&value).unwrap())
        .unwrap();
    repo.store().save("transform", &project).unwrap();
    let loaded = repo.store().load("transform").unwrap();
    let clip = &loaded.tracks[0].clips[0];
    assert_eq!(
        clip.transform,
        todo_core::ClipTransform {
            x: 0.25,
            y: -0.5,
            scale: 1.75,
            scale_x: 0.5,
            scale_y: 2.0,
            opacity: 0.4,
        }
    );
    assert_eq!(clip.link_id.as_deref(), Some("linked"));
}

#[test]
fn load_rejects_unsupported_or_corrupt_on_disk_documents() {
    let repo = TestRepo::new();
    repo.store().create("project").unwrap();
    let path = repo.data().join("project/project.json");
    fs::write(&path, br#"{"version": 2}"#).unwrap();
    assert!(matches!(
        repo.store().load("project"),
        Err(StorageError::UnsupportedVersion(2))
    ));
    fs::write(&path, b"{").unwrap();
    assert!(matches!(
        repo.store().load("project"),
        Err(StorageError::Json(_))
    ));
    // Corrupt projects must still be removable and recoverable.
    repo.store().delete("project").unwrap();
    let archived = fs::read_dir(repo.data().join(".trash"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read(archived.join("project.json")).unwrap(), b"{");
}

#[cfg(windows)]
#[test]
fn failed_replacement_retains_previous_document_and_cleans_staging() {
    use std::os::windows::fs::OpenOptionsExt;

    let repo = TestRepo::new();
    let mut project = repo.store().create("project").unwrap();
    let path = repo.data().join("project/project.json");
    let before = fs::read(&path).unwrap();
    // Hold a real Windows handle denying deletion/replacement of this file.
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    project.playhead = 7.0;
    assert!(matches!(
        repo.store().save("project", &project),
        Err(StorageError::Io { .. })
    ));
    drop(locked);
    assert_eq!(fs::read(&path).unwrap(), before);
    let files: Vec<_> = fs::read_dir(repo.data().join("project"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|name| name == "project.json"));
    assert!(files.iter().any(|name| name == "media"));
}

#[cfg(any(unix, windows))]
struct DirectoryLink(PathBuf);

#[cfg(any(unix, windows))]
impl DirectoryLink {
    fn create(link: &Path, target: &Path) -> Self {
        #[cfg(windows)]
        {
            // Windows PowerShell's New-Item cannot handle canonical verbatim
            // path prefixes, although Rust filesystem operations support them.
            let link = link.to_str().unwrap();
            let target = target.to_str().unwrap();
            let output = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:TODO_STORAGE_LINK -Target $env:TODO_STORAGE_TARGET | Out-Null"])
                .env("TODO_STORAGE_LINK", link.strip_prefix(r"\\?\").unwrap_or(link))
                .env("TODO_STORAGE_TARGET", target.strip_prefix(r"\\?\").unwrap_or(target))
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        Self(link.to_owned())
    }
}

#[cfg(any(unix, windows))]
impl Drop for DirectoryLink {
    fn drop(&mut self) {
        #[cfg(windows)]
        fs::remove_dir(&self.0).unwrap();
        #[cfg(unix)]
        fs::remove_file(&self.0).unwrap();
    }
}

#[cfg(any(unix, windows))]
#[test]
fn linked_data_directory_cannot_escape_repository() {
    let repo = TestRepo::new();
    let outside = TestRepo::new();
    let original = outside.store().create("project").unwrap();
    let _link = DirectoryLink::create(&repo.data(), &outside.data());
    assert!(matches!(
        repo.store().list(),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(matches!(
        repo.store().create("outside"),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(matches!(
        repo.store().load("project"),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(matches!(
        repo.store().save("project", &original),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(matches!(
        repo.store().delete("project"),
        Err(StorageError::UnsafePath(_))
    ));
    assert_eq!(outside.store().load("project").unwrap(), original);
    assert!(!outside.data().join("outside").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn linked_project_and_trash_directories_cannot_escape_data() {
    let repo = TestRepo::new();
    let outside = TestRepo::new();
    let original = outside.store().create("project").unwrap();
    repo.store().create("local").unwrap();
    let _project_link = DirectoryLink::create(
        &repo.data().join("project"),
        &outside.data().join("project"),
    );
    let _trash_link = DirectoryLink::create(&repo.data().join(".trash"), &outside.data());
    assert_eq!(repo.store().list().unwrap(), vec!["local"]);
    assert!(matches!(
        repo.store().load("project"),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(matches!(
        repo.store().save("project", &original),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(matches!(
        repo.store().delete("project"),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(matches!(
        repo.store().delete("local"),
        Err(StorageError::UnsafePath(_))
    ));
    assert!(repo.data().join("local/project.json").is_file());
    assert_eq!(outside.store().load("project").unwrap(), original);
}
