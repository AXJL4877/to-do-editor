use image::{Rgba, RgbaImage};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};
use todo_workflow::{check_extension_fixtures, load, Origin, RuntimePaths};

#[test]
fn built_in_productions_pass_offline_worker_self_check() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = RuntimePaths::new(repo.clone());
    for relative in [
        "extensions/builtin/chalk-hand",
        "extensions/builtin/spongebob-koubo",
    ] {
        let directory = repo.join(relative);
        let extension = load(&directory, Origin::Builtin)
            .unwrap_or_else(|problems| panic!("{}: {problems:?}", directory.display()));
        let work = repo
            .join("target/extension-checks")
            .join(&extension.manifest.id);
        let report = check_extension_fixtures(&extension, &runtime, &work).unwrap();
        assert!(report.ready);
        assert_eq!(report.fixture_runs, 1);
        if relative.ends_with("chalk-hand") {
            let reveal = work.join("out/sample/e1.reveal.png");
            assert!(reveal.is_file(), "chalk fixture should emit a reveal atlas");
            let (meta, color, timing) = todo_media::read_reveal(&reveal).unwrap();
            let at_start = todo_media::compose_reveal(&color, &timing, &meta, 0.0).unwrap();
            let at_end = todo_media::compose_reveal(&color, &timing, &meta, meta.duration).unwrap();
            assert_eq!(at_start.dimensions(), color.dimensions());
            assert!(
                at_start.pixels().all(|p| p[3] == 0),
                "nothing is drawn before the first stroke"
            );
            assert_eq!(at_end, color);
            let hand = meta
                .hand
                .expect("hand-drawn elements carry the chalk hand path");
            assert!(hand.path.len() > 3 && work.join("out/sample").join(&hand.file).is_file());
        }
        assert!(
            !directory.join("tests/work").exists(),
            "self-checks never write into the package"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    let export_root = repo.join("target/e2e-productions");
    let frames = export_root.join("frames");
    let _ = std::fs::remove_dir_all(&export_root);
    std::fs::create_dir_all(&frames).unwrap();
    for index in 0..3 {
        RgbaImage::from_pixel(8, 8, Rgba([30, 100 + index * 20, 220, 255]))
            .save(frames.join(format!("{:06}.png", index + 1)))
            .unwrap();
    }
    let tools = todo_media::MediaTools::discover(&repo);
    if tools.ffmpeg.is_file() && tools.ffprobe.is_file() {
        let output = export_root.join("offline.mp4");
        let result = todo_media::export_frames(
            &tools,
            &todo_media::ExportRequest {
                frames,
                fps: 30.0,
                audio: None,
                output,
                preset: todo_media::ExportPreset::Draft,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(result.output.is_file());
        assert!(result.report_path.is_file());
    }
    let _ = std::fs::remove_dir_all(export_root);
}
