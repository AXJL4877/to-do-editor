use std::path::PathBuf;
use todo_ai::{Journal, TaskState};
use todo_production::*;

fn scratch(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ai-journal-tests")
        .join(format!("{name}-{unique}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn image(prompt: &str) -> TaskSpec {
    TaskSpec {
        request: TaskRequest::Image(ImageRequest {
            model: "gpt-image-2".into(),
            prompt: prompt.into(),
            size: "1024x1024".into(),
            quality: None,
            transparent: false,
            variant: 0,
            references: Vec::new(),
            reference_digests: Vec::new(),
        }),
    }
}

fn chat(prompt: &str) -> TaskSpec {
    TaskSpec {
        request: TaskRequest::Language(LanguageRequest {
            model: "gpt-6-astra".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: prompt.into(),
            }],
            temperature: 0.3,
            json: true,
        }),
    }
}

#[test]
fn one_record_per_request_survives_reopening() {
    let dir = scratch("reopen");
    let spec = image("chalk house");
    let key = spec.key();
    {
        let (mut journal, recovered) = Journal::open(&dir, 2, 1).unwrap();
        assert!(recovered.is_empty());
        assert_eq!(journal.ensure(&spec, 1).unwrap().state, TaskState::Pending);
        journal
            .update(&key, 2, |r| {
                r.state = TaskState::Downloading;
                r.remote_url = Some("https://cdn.example/x.png".into());
            })
            .unwrap();
        // Ensuring again never resets an existing record.
        assert_eq!(
            journal.ensure(&spec, 3).unwrap().state,
            TaskState::Downloading
        );
    }
    let (journal, recovered) = Journal::open(&dir, 2, 4).unwrap();
    assert!(recovered.is_empty(), "downloads resume as they are");
    let record = journal.get(&key).unwrap();
    assert_eq!(record.state, TaskState::Downloading);
    assert_eq!(
        record.remote_url.as_deref(),
        Some("https://cdn.example/x.png")
    );
    assert_eq!(record.created_at, 1);
}

#[test]
fn interrupted_sends_follow_the_billing_rules() {
    let dir = scratch("recover");
    let free = chat("plan");
    let paid = image("fresh");
    let spent = image("spent");
    {
        let (mut journal, _) = Journal::open(&dir, 2, 1).unwrap();
        for spec in [&free, &paid, &spent] {
            journal.ensure(spec, 1).unwrap();
            journal
                .update(&spec.key(), 1, |r| r.state = TaskState::Submitting)
                .unwrap();
        }
        journal
            .update(&spent.key(), 1, |r| r.paid_retries = 2)
            .unwrap();
    }
    let (journal, mut recovered) = Journal::open(&dir, 2, 5).unwrap();
    recovered.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(recovered.len(), 3);
    assert_eq!(journal.get(&free.key()).unwrap().state, TaskState::Pending);
    let paid = journal.get(&paid.key()).unwrap();
    assert_eq!((paid.state, paid.paid_retries), (TaskState::Pending, 1));
    let spent = journal.get(&spent.key()).unwrap();
    assert_eq!(spent.state, TaskState::Ambiguous);
    assert_eq!(spent.updated_at, 5);
}

#[test]
fn manual_retry_restarts_failed_tasks_and_keeps_a_known_result_url() {
    let dir = scratch("retry");
    let (mut journal, _) = Journal::open(&dir, 2, 1).unwrap();
    let a = image("a");
    let b = image("b");
    for spec in [&a, &b] {
        journal.ensure(spec, 1).unwrap();
    }
    journal
        .update(&a.key(), 2, |r| {
            r.state = TaskState::Ambiguous;
            r.paid_retries = 2;
            r.failures = 3;
        })
        .unwrap();
    journal
        .update(&b.key(), 2, |r| {
            r.state = TaskState::Failed;
            r.remote_url = Some("https://cdn.example/b.png".into());
        })
        .unwrap();
    let a = journal.retry(&a.key(), 3).unwrap().clone();
    assert_eq!(
        (a.state, a.failures, a.paid_retries),
        (TaskState::Pending, 0, 0)
    );
    assert_eq!(
        journal.retry(&b.key(), 3).unwrap().state,
        TaskState::Downloading
    );
}

#[test]
fn foreign_or_corrupt_files_are_skipped_not_trusted() {
    let dir = scratch("corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("deadbeef.json"), b"{ not json").unwrap();
    // A record whose file name does not match its own key.
    let spec = image("x");
    {
        let (mut journal, _) = Journal::open(&dir, 2, 1).unwrap();
        journal.ensure(&spec, 1).unwrap();
    }
    std::fs::rename(
        dir.join(format!("{}.json", spec.key())),
        dir.join("0000.json"),
    )
    .unwrap();
    let (journal, _) = Journal::open(&dir, 2, 2).unwrap();
    assert!(journal.get(&spec.key()).is_none());
    assert_eq!(journal.skipped.len(), 2);
}

fn with_reference(path: &str, digest: &str) -> TaskSpec {
    let mut spec = image("crab at the counter");
    if let TaskRequest::Image(value) = &mut spec.request {
        value.references = vec![path.into()];
        value.reference_digests = vec![digest.into()];
    }
    spec
}

#[test]
fn the_same_reference_picture_elsewhere_is_the_same_request() {
    let here = with_reference("data/a/work/out/lib/krabs.png", "d1");
    let there = with_reference("data/b/work/out/lib/krabs.png", "d1");
    let changed = with_reference("data/a/work/out/lib/krabs.png", "d2");
    assert_eq!(here.key(), there.key());
    assert_ne!(here.key(), changed.key());

    // The record keeps the newest paths, so a rerun sends files that exist.
    let dir = scratch("reference-paths");
    let (mut journal, _) = Journal::open(&dir, 2, 1).unwrap();
    journal.ensure(&here, 1).unwrap();
    let record = journal.ensure(&there, 2).unwrap();
    let TaskRequest::Image(value) = &record.spec.request else {
        panic!()
    };
    assert_eq!(value.references, vec!["data/b/work/out/lib/krabs.png"]);
}

#[test]
fn records_written_under_the_old_key_are_moved_not_lost() {
    let dir = scratch("legacy-key");
    let spec = with_reference("data/a/work/out/lib/krabs.png", "d1");
    let (mut journal, _) = Journal::open(&dir, 2, 1).unwrap();
    journal.ensure(&spec, 1).unwrap();
    journal
        .update(&spec.key(), 2, |r| {
            r.state = TaskState::Done;
            r.remote_url = Some("https://cdn.example/old.png".into());
        })
        .unwrap();
    drop(journal);
    // Rewrite the file the way journals stored it before (path-based key).
    let new_file = dir.join(format!("{}.json", spec.key()));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&new_file).unwrap()).unwrap();
    value["key"] = serde_json::json!(spec.legacy_key().as_str());
    std::fs::remove_file(&new_file).unwrap();
    std::fs::write(
        dir.join(format!("{}.json", spec.legacy_key())),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();

    let (journal, _) = Journal::open(&dir, 2, 3).unwrap();
    let record = journal.get(&spec.key()).expect("moved to the new key");
    assert_eq!(record.state, TaskState::Done);
    assert!(journal.skipped.is_empty());
    assert!(new_file.is_file());
    assert!(!dir.join(format!("{}.json", spec.legacy_key())).exists());
}
