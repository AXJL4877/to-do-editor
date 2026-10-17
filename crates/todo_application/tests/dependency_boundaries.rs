//! Guard the production dependency graph, including renamed/target-specific dependencies.
use std::{collections::BTreeSet, process::Command};

#[test]
fn workspace_production_dependencies_respect_layer_boundaries() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rules: &[(&str, &[&str])] = &[
        ("todo_core", &["serde", "serde_json"]),
        (
            "todo_application",
            &["todo_core", "todo_production", "serde", "serde_json"],
        ),
        (
            "todo_interaction",
            &["todo_core", "todo_application", "todo_production", "uuid"],
        ),
        ("todo_ui", &["todo_interaction", "egui", "image"]),
        (
            "todo_storage",
            &[
                "todo_core",
                "todo_production",
                "serde_json",
                "thiserror",
                "uuid",
            ],
        ),
        (
            "todo_media",
            &[
                "todo_core",
                "todo_storage",
                "todo_text",
                "todo_process",
                "serde",
                "serde_json",
                "serde_json",
                "thiserror",
                "uuid",
                "image",
                "png",
            ],
        ),
        (
            "todo_text",
            &["todo_core", "png", "thiserror", "tiny-skia", "ttf-parser"],
        ),
        ("todo_preview", &["thiserror"]),
        ("todo_process", &["thiserror", "windows-sys"]),
        // Extension packages and worker processes; services are answered by the app.
        (
            "todo_extensions",
            &[
                "todo_production",
                "todo_process",
                "serde",
                "serde_json",
                "thiserror",
                "uuid",
                "zip",
            ],
        ),
        // Pure production domain: requests, plans and rules as data.
        (
            "todo_production",
            &["todo_core", "serde", "serde_json", "sha2"],
        ),
        // External AI infrastructure; never a dependency of application or UI.
        (
            "todo_ai",
            &[
                "todo_production",
                "serde",
                "serde_json",
                "thiserror",
                "sha2",
                "image",
                "windows-sys",
                "tokio",
                "reqwest",
                "base64",
            ],
        ),
        (
            "todo_app",
            &[
                "todo_application",
                "todo_ai",
                "todo_extensions",
                "todo_production",
                "todo_interaction",
                "todo_ui",
                "todo_storage",
                "todo_media",
                "todo_preview",
                "eframe",
                "egui",
                "raw-window-handle",
                "rfd",
                "serde",
                "serde_json",
            ],
        ),
    ];
    for package in metadata["packages"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap();
        let (_, allowed) = rules
            .iter()
            .find(|(item, _)| *item == name)
            .unwrap_or_else(|| panic!("Define the layer boundary for {name}"));
        let allowed: BTreeSet<_> = allowed.iter().copied().collect();
        let actual: BTreeSet<_> = package["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|dep| dep["kind"].is_null())
            .map(|dep| dep["name"].as_str().unwrap())
            .collect();
        let unexpected: Vec<_> = actual.difference(&allowed).collect();
        assert!(
            unexpected.is_empty(),
            "{name} has dependencies outside its layer: {unexpected:?}"
        );
    }
}
