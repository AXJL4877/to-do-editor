use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use todo_production::{Limits, PackRef, Production, SplitMode};
use todo_storage::{ProductionStore, ProjectStore, StorageError};
use uuid::Uuid;

struct Repo {
    root: PathBuf,
}
impl Repo {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/production-store-tests")
            .join(Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        ProjectStore::new(&root).create("剪辑").unwrap();
        Self { root }
    }
    fn store(&self) -> ProductionStore {
        ProductionStore::new(&self.root)
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn document(id: &str) -> Production {
    Production::new(
        id,
        PackRef {
            id: "builtin.chalk-hand".into(),
            version: "4.0.0".into(),
        },
        "你好",
        BTreeMap::new(),
        SplitMode::Semantic,
        Limits {
            target: 120,
            max: 200,
        },
    )
}

#[test]
fn production_round_trip_revision_and_workdir_are_fenced() {
    let repo = Repo::new();
    let store = repo.store();
    store.create("剪辑", &document("p1")).unwrap();
    assert_eq!(
        store.list("剪辑").unwrap()[0],
        todo_storage::ProductionSummary {
            id: "p1".into(),
            title: String::new(),
            rev: 0
        }
    );
    let work = store.workdir("剪辑", "p1").unwrap();
    assert!(work.ends_with("productions/p1/work") || work.ends_with("productions\\p1\\work"));
    let mut changed = store.load("剪辑", "p1").unwrap();
    changed.title = "标题".into();
    store.save("剪辑", &changed).unwrap();
    assert_eq!(store.load("剪辑", "p1").unwrap().rev, 1);
    changed.title = "新标题".into();
    store.save("剪辑", &changed).unwrap();
    assert_eq!(store.load("剪辑", "p1").unwrap().rev, 2);
}

#[test]
fn invalid_ids_and_deleted_documents_cannot_resurrect() {
    let repo = Repo::new();
    let store = repo.store();
    assert!(matches!(
        store.create("剪辑", &document("../escape")),
        Err(StorageError::InvalidName(_))
    ));
    store.create("剪辑", &document("p1")).unwrap();
    store.delete("剪辑", "p1").unwrap();
    assert!(matches!(
        store.load("剪辑", "p1"),
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        store.save("剪辑", &document("p1")),
        Err(StorageError::NotFound(_))
    ));
}
