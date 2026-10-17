use std::io::Write;
use std::path::PathBuf;
use todo_ai::artifacts::{verify_image, verify_wav};
use todo_ai::ArtifactStore;

fn scratch(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ai-artifact-tests")
        .join(format!("{name}-{unique}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn wav(samples: usize) -> Vec<u8> {
    let data = samples * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&[1, 0, 1, 0]);
    out.extend_from_slice(&24000u32.to_le_bytes());
    out.extend_from_slice(&48000u32.to_le_bytes());
    out.extend_from_slice(&[2, 0, 16, 0]);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    out.resize(out.len() + data, 0);
    out
}

#[test]
fn verified_results_are_content_addressed_and_deduplicated() {
    let store = ArtifactStore::new(scratch("dedupe"));
    let bytes = png(64, 48);
    let first = store
        .put("task-a", &bytes, "png", |p| {
            verify_image(p, (32, 32)).map(|_| ())
        })
        .unwrap();
    assert!(store.contains(&first));
    assert_eq!(first.bytes, bytes.len() as u64);
    assert!(store
        .path(&first)
        .ends_with(format!("{}/{}.png", &first.sha256[..2], first.sha256)));
    let again = store
        .put("task-b", &bytes, "png", |p| {
            verify_image(p, (32, 32)).map(|_| ())
        })
        .unwrap();
    assert_eq!(first, again);
}

#[test]
fn truncated_or_undersized_images_never_publish() {
    let store = ArtifactStore::new(scratch("truncated"));
    let bytes = png(64, 64);
    let error = store
        .put("cut", &bytes[..bytes.len() / 2], "png", |p| {
            verify_image(p, (1, 1)).map(|_| ())
        })
        .unwrap_err();
    assert!(error.to_string().contains("does not decode"), "{error}");
    assert!(
        store.part("cut").unwrap().is_empty(),
        "failed parts are discarded"
    );
    let small = store
        .put("small", &png(16, 16), "png", |p| {
            verify_image(p, (1024, 1024)).map(|_| ())
        })
        .unwrap_err();
    assert!(small.to_string().contains("16x16"));
}

#[test]
fn parts_resume_across_writers() {
    let store = ArtifactStore::new(scratch("resume"));
    let bytes = png(40, 40);
    let part = store.part("download").unwrap();
    part.append().unwrap().write_all(&bytes[..100]).unwrap();
    drop(part);
    // A later attempt continues where the previous one stopped (HTTP Range).
    let part = store.part("download").unwrap();
    assert_eq!(part.len(), 100);
    part.append().unwrap().write_all(&bytes[100..]).unwrap();
    let artifact = store
        .publish(part, "png", |p| verify_image(p, (1, 1)).map(|_| ()))
        .unwrap();
    assert_eq!(std::fs::read(store.path(&artifact)).unwrap(), bytes);
}

#[test]
fn wav_checks_the_declared_data_size() {
    let dir = scratch("wav");
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.wav");
    std::fs::write(&good, wav(2400)).unwrap();
    assert!(verify_wav(&good).is_ok());
    let cut = dir.join("cut.wav");
    let bytes = wav(2400);
    std::fs::write(&cut, &bytes[..bytes.len() - 100]).unwrap();
    assert!(verify_wav(&cut).unwrap_err().contains("declares"));
    let streamed = dir.join("streamed.wav");
    let mut bytes = wav(10);
    bytes[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&streamed, bytes).unwrap();
    assert!(
        verify_wav(&streamed).is_err(),
        "0xFFFFFFFF streaming headers are rejected"
    );
}
