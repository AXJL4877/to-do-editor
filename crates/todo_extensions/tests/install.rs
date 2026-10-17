use std::io::Write;
use std::path::{Path, PathBuf};
use todo_extensions::{install, pack, uninstall, InstallError, Origin, Registry};

fn temp(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/extension-install-tests")
        .join(format!("{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn package(dir: &Path, version: &str) {
    let manifest = serde_json::json!({
        "spec": "todo-extension/1",
        "id": "studio.title-burst",
        "name": "片头爆字",
        "version": version,
        "kind": "workflow",
        "requires": {"services": ["encode"], "runtime": {"type": "python"}},
        "worker": {"entry": "worker/main.py", "ops": ["run"], "timeoutSec": 60}
    });
    std::fs::create_dir_all(dir.join("worker/__pycache__")).unwrap();
    std::fs::write(
        dir.join("extension.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("worker/main.py"), "print('hi')\n").unwrap();
    std::fs::write(dir.join("worker/__pycache__/main.pyc"), b"junk").unwrap();
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        out.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn packed_extensions_install_upgrade_and_uninstall() {
    let source = temp("src");
    let user = temp("user");
    package(&source, "1.0.0");
    let first = install(&pack(&source).unwrap(), &user).unwrap();
    assert_eq!(first.build_key(), "studio.title-burst@1.0.0");
    assert!(!user.join("studio.title-burst/worker/__pycache__").exists());

    package(&source, "1.1.0");
    install(&pack(&source).unwrap(), &user).unwrap();
    let registry = Registry::scan(&temp("builtin"), &user);
    let found = registry.get("studio.title-burst").unwrap();
    assert_eq!(
        (found.manifest.version.as_str(), found.origin),
        ("1.1.0", Origin::User)
    );
    // No staging or retired directories are left behind.
    assert_eq!(std::fs::read_dir(&user).unwrap().count(), 1);

    assert!(uninstall(&user, "studio.title-burst").unwrap());
    assert!(!uninstall(&user, "../escape").unwrap());
    assert!(Registry::scan(&temp("builtin"), &user)
        .get("studio.title-burst")
        .is_none());
}

#[test]
fn unsafe_or_invalid_archives_write_nothing() {
    let user = temp("bad");
    for entries in [
        vec![("../evil.txt", b"x".as_slice())],
        vec![("C:/evil.txt", b"x".as_slice())],
        vec![("worker/main.py", b"x".as_slice())],
    ] {
        let result = install(&zip_of(&entries), &user);
        assert!(matches!(result, Err(InstallError::Unsafe(_))), "{result:?}");
    }
    let manifest = br#"{"spec":"todo-extension/1","id":"a.b","name":"x","version":"1.0.0","kind":"workflow","worker":{"entry":"worker/main.py","ops":["run"],"timeoutSec":60}}"#;
    let result = install(&zip_of(&[("extension.json", manifest)]), &user);
    assert!(
        matches!(result, Err(InstallError::Invalid(_))),
        "{result:?}"
    );
    assert!(matches!(
        install(b"not a zip", &user),
        Err(InstallError::Archive(_))
    ));
    assert_eq!(std::fs::read_dir(&user).unwrap().count(), 0);
}

#[test]
fn broken_packages_are_listed_but_never_runnable() {
    let builtin = temp("builtin-broken");
    let dir = builtin.join("broken");
    package(&dir, "1.0.0");
    std::fs::remove_file(dir.join("worker/main.py")).unwrap();
    let registry = Registry::scan(&builtin, &temp("none"));
    assert!(registry.get("studio.title-burst").is_none());
    assert_eq!(registry.rejected.len(), 1);
    assert!(registry.rejected[0]
        .problems
        .iter()
        .any(|p| p.path == "worker.entry"));
}
