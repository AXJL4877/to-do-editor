use super::*;

#[test]
fn output_validation_rejects_truncated_successful_encodes() {
    let info = |duration| VideoInfo {
        index: 0,
        width: 96,
        height: 64,
        sar: 1.0,
        start: 0.0,
        duration,
        origin: 0.0,
        time_base: "1:1000".into(),
        frames: None,
        codec: "h264".into(),
        has_audio: false,
        hdr: false,
    };
    assert!(validate_output(&info(60.0), &info(30.0), 96, 64).is_err());
    assert!(validate_output(&info(60.0), &info(60.0), 96, 64).is_ok());
}

#[test]
fn capacity_reserves_only_the_remaining_budget_and_keeps_existing_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/edit-proxy-budget-tests")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).unwrap();
    let file = root.join("existing.mp4");
    let original = vec![0; 3 * 1024 * 1024];
    fs::write(&file, &original).unwrap();
    assert_eq!(
        available_capacity(&root, 4 * 1024 * 1024, 2 * 1024 * 1024).unwrap(),
        1024 * 1024
    );
    fs::write(root.join("manifest.json"), b"{}").unwrap();
    assert!(available_capacity(&root, 4 * 1024 * 1024, 2 * 1024 * 1024).is_err());
    assert_eq!(fs::read(&file).unwrap(), original);
    fs::remove_dir_all(&root).unwrap();
}
