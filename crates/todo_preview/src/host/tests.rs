use super::{lock, pump, resolve_host, spawn_child, HostState, PreviewHost, Shared};
use crate::{
    AlignGuide, HostEvent, PreviewError, PreviewNavigationKind, PreviewPointerKind, TransformGizmo,
    VisualTransform,
};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/todo_preview_tests");
        let path = root.join(format!("{label}_{}_{n}", std::process::id()));
        fs::create_dir_all(&path).unwrap_or_else(|err| {
            panic!("create {} failed: {err}", path.display());
        });
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn host_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PreviewHost>();
}

#[test]
fn resolve_prefers_env_and_walks_up_from_cwd() {
    let dir = TempDir::new("resolve");
    let exe = dir.0.join("native/build/bin/preview_host.exe");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::write(&exe, b"").unwrap();

    let missing = dir.0.join("missing.exe");
    let err = resolve_host(Some(missing.as_os_str()), Some(&dir.0), &dir.0).unwrap_err();
    match err {
        PreviewError::HostNotFound(path) => assert_eq!(path, missing),
        other => panic!("{other}"),
    }

    let nested = dir.0.join("target/debug");
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(resolve_host(None, Some(&nested), &nested).unwrap(), exe);

    let elsewhere = dir.0.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let manifest = dir.0.join("crates/todo_preview");
    fs::create_dir_all(&manifest).unwrap();
    assert_eq!(
        resolve_host(None, Some(&elsewhere), &manifest).unwrap(),
        exe
    );

    assert_eq!(
        resolve_host(Some(exe.as_os_str()), Some(&elsewhere), &elsewhere).unwrap(),
        exe
    );

    let ghost = Path::new(r"C:\todo_preview_no_such_root");
    match resolve_host(None, Some(ghost), ghost).unwrap_err() {
        PreviewError::HostNotFound(path) => {
            assert!(path.ends_with(crate::HOST_RELATIVE));
        }
        other => panic!("{other}"),
    }
}

#[test]
fn reader_thread_fills_mutex_and_channel() {
    let (tx, rx) = sync_channel(8);
    let shared = Arc::new(Shared {
        state: Mutex::new(HostState {
            clock: None,
            bounds: None,
            source_aspect: None,
            pointers: VecDeque::new(),
        }),
        tx,
    });
    let shared_thread = Arc::clone(&shared);
    let payload = b"noise\r\naudio-clock 3.250000000 1 0\r\npreview-bounds 8 9 100 80 1\r\n";
    thread::spawn(move || {
        let mut reader = BufReader::with_capacity(3, Cursor::new(&payload[..]));
        let mut buf = Vec::new();
        pump(&mut reader, &mut buf, &shared_thread).unwrap();
    })
    .join()
    .unwrap();

    let state = lock(&shared.state);
    let clock = state.clock.unwrap();
    assert!((clock.time - 3.25).abs() < 1e-12);
    assert!(clock.playing && !clock.available);
    let bounds = state.bounds.unwrap();
    assert_eq!(
        (bounds.x, bounds.y, bounds.w, bounds.h, bounds.visible),
        (8.0, 9.0, 100.0, 80.0, Some(true))
    );
    drop(state);
    assert!(matches!(rx.try_recv(), Ok(HostEvent::AudioClock(_))));
    assert!(matches!(rx.try_recv(), Ok(HostEvent::PreviewBounds(_))));
    assert!(rx.try_recv().is_err());
}

#[test]
fn full_channel_keeps_latest_sample() {
    let (tx, _rx) = sync_channel(1);
    let shared = Shared {
        state: Mutex::new(HostState {
            clock: None,
            bounds: None,
            source_aspect: None,
            pointers: VecDeque::new(),
        }),
        tx,
    };
    let payload = b"audio-clock 1.000000000 0 1\naudio-clock 2.000000000 1 1\n";
    let mut reader = BufReader::new(Cursor::new(&payload[..]));
    let mut buf = Vec::new();
    pump(&mut reader, &mut buf, &shared).unwrap();
    let clock = lock(&shared.state).clock.unwrap();
    assert!((clock.time - 2.0).abs() < 1e-12);
    assert!(clock.playing);
}

#[test]
fn pointer_edges_survive_full_channel_and_moves_coalesce() {
    let (tx, rx) = sync_channel(1);
    let shared = Shared {
        state: Mutex::new(HostState {
            clock: None,
            bounds: None,
            source_aspect: None,
            pointers: VecDeque::new(),
        }),
        tx,
    };
    let payload = b"audio-clock 1 0 1\npreview-pointer down 0.5 0.25\npreview-pointer move 0.75 0.5\npreview-pointer move -0.25 1.5\npreview-pointer up -0.25 1.5\naudio-clock 2 0 1\n";
    pump(
        &mut BufReader::new(Cursor::new(payload)),
        &mut Vec::new(),
        &shared,
    )
    .unwrap();
    let mut state = lock(&shared.state);
    assert_eq!(state.pointers.len(), 3);
    assert_eq!(
        state.pointers.pop_front(),
        Some(HostEvent::PreviewPointer {
            kind: PreviewPointerKind::Down,
            x: 0.5,
            y: 0.25,
        })
    );
    assert_eq!(
        state.pointers.pop_front(),
        Some(HostEvent::PreviewPointer {
            kind: PreviewPointerKind::Move,
            x: -0.25,
            y: 1.5,
        })
    );
    assert_eq!(
        state.pointers.pop_front(),
        Some(HostEvent::PreviewPointer {
            kind: PreviewPointerKind::Up,
            x: -0.25,
            y: 1.5,
        })
    );
    assert_eq!(state.clock.unwrap().time, 2.0);
    assert!(matches!(rx.try_recv(), Ok(HostEvent::AudioClock(_))));
}

#[test]
fn navigation_edges_and_wheel_survive_full_channel_in_pointer_order() {
    let (tx, _rx) = sync_channel(1);
    let shared = Shared {
        state: Mutex::new(HostState {
            clock: None,
            bounds: None,
            source_aspect: None,
            pointers: VecDeque::new(),
        }),
        tx,
    };
    let payload = b"audio-clock 1 0 1\npreview-navigation pan-down 0 0.5 0.5\npreview-navigation pan-move 0 0.6 0.5\npreview-navigation pan-move 0 0.7 0.5\npreview-navigation pan-up 0 0.7 0.5\npreview-navigation wheel -1 0.7 0.5\npreview-pointer down 0.4 0.5\npreview-pointer up 0.4 0.5\n";
    pump(
        &mut BufReader::new(Cursor::new(payload)),
        &mut Vec::new(),
        &shared,
    )
    .unwrap();
    let state = lock(&shared.state);
    assert_eq!(
        state.pointers.len(),
        6,
        "navigation edges must not be lost behind clock traffic"
    );
    let kinds: Vec<_> = state
        .pointers
        .iter()
        .map(|event| match event {
            HostEvent::PreviewNavigation(input) => match input.kind {
                PreviewNavigationKind::PanDown => "pan-down",
                PreviewNavigationKind::PanMove => {
                    assert_eq!(input.x, 0.7);
                    "pan-move"
                }
                PreviewNavigationKind::PanUp => "pan-up",
                PreviewNavigationKind::Wheel => {
                    assert_eq!(input.delta, -1.0);
                    "wheel"
                }
            },
            HostEvent::PreviewPointer {
                kind: PreviewPointerKind::Down,
                ..
            } => "down",
            HostEvent::PreviewPointer {
                kind: PreviewPointerKind::Up,
                ..
            } => "up",
            _ => panic!("unexpected queued event"),
        })
        .collect();
    assert_eq!(
        kinds,
        ["pan-down", "pan-move", "pan-up", "wheel", "down", "up"]
    );
}

#[test]
fn transform_and_pointer_roundtrip() {
    let host = spawn_child(crate::test_support::command("transform"), Stdio::null()).unwrap();
    assert!(matches!(
        host.transform(f64::NAN, 0.0, 1.0),
        Err(PreviewError::Invalid(_))
    ));
    host.transform(0.25, -0.5, 2.0).unwrap();
    let mut events = Vec::new();
    assert!(wait_until(&host, |host| lock(&host.shared.state)
        .pointers
        .len()
        == 3));
    while let Some(event) = host.try_recv() {
        events.push(event);
    }
    assert_eq!(
        events,
        vec![
            HostEvent::PreviewPointer {
                kind: PreviewPointerKind::Down,
                x: 0.25,
                y: -0.5
            },
            HostEvent::PreviewPointer {
                kind: PreviewPointerKind::Move,
                x: 1.25,
                y: -0.25
            },
            HostEvent::PreviewPointer {
                kind: PreviewPointerKind::Up,
                x: 1.25,
                y: -0.25
            },
        ]
    );
    host.quit().unwrap();
}

#[test]
fn visual_transform_gizmo_host_roundtrip_has_no_invalid_partial_commands() {
    let dir = TempDir::new("visual_transform");
    let marker = dir.0.join("commands.txt");
    let mut command = crate::test_support::command("gizmo");
    command.env("TODO_TEST_PROCESS_MARKER", &marker);
    let host = spawn_child(command, Stdio::null()).unwrap();
    assert!(host
        .set_transform(VisualTransform {
            scale_x: 0.0,
            ..VisualTransform::default()
        })
        .is_err());
    assert!(host
        .set_transform(VisualTransform {
            opacity: f64::NAN,
            ..VisualTransform::default()
        })
        .is_err());
    assert!(host
        .set_gizmo(Some(TransformGizmo {
            left: 1.0,
            top: 0.0,
            right: 0.0,
            bottom: 1.0,
            editable: true,
        }))
        .is_err());
    host.transform(0.25, -0.5, 2.0).unwrap();
    host.set_transform(VisualTransform {
        x: 0.25,
        y: -0.5,
        scale: 2.0,
        scale_x: 0.5,
        scale_y: 3.0,
        opacity: 0.0,
    })
    .unwrap();
    host.set_gizmo(Some(TransformGizmo {
        left: -0.25,
        top: 0.0,
        right: 0.75,
        bottom: 1.0,
        editable: false,
    }))
    .unwrap();
    host.set_gizmo(None).unwrap();
    host.set_guides(&[
        AlignGuide {
            vertical: true,
            position: 0.5,
        },
        AlignGuide {
            vertical: false,
            position: 0.25,
        },
    ])
    .unwrap();
    host.set_guides(&[]).unwrap();
    host.transport(42.0, false).unwrap();
    assert!(wait_until(&host, |host| host
        .clock_sample()
        .is_some_and(|clock| clock.time == 42.0)));
    assert_eq!(fs::read_to_string(&marker).unwrap(),
        "transform 0.25 -0.5 2\ntransform 0.25 -0.5 2 0.5 3 0\ngizmo -0.25 0 0.75 1 0\ngizmo\nguides 0 0.5 1 0.25\nguides\ntransport 42 0");
    host.quit().unwrap();
}

#[test]
fn tagged_source_roundtrip_retains_aspect_before_pointer_when_channel_is_full() {
    let dir = TempDir::new("source_aspect");
    let marker = dir.0.join("commands.txt");
    let mut command = crate::test_support::command("source");
    command.env("TODO_TEST_PROCESS_MARKER", &marker);
    let host = spawn_child(command, Stdio::null()).unwrap();
    assert!(host
        .set_path_with_generation(Some("bad\npath"), 99)
        .is_err());
    host.set_path_with_generation(Some("C:/a b.mp4"), 42)
        .unwrap();
    assert!(wait_until(&host, |host| matches!(
        lock(&host.shared.state).source_aspect,
        Some(HostEvent::SourceAspect { generation: 42, .. })
    )));
    assert_eq!(
        host.try_recv(),
        Some(HostEvent::SourceAspect {
            generation: 42,
            aspect: 4.0 / 3.0
        })
    );
    assert!(matches!(
        host.try_recv(),
        Some(HostEvent::PreviewPointer {
            kind: PreviewPointerKind::Down,
            ..
        })
    ));
    assert!(matches!(host.try_recv(), Some(HostEvent::AudioClock(_))));
    host.set_path_with_generation(None, 43).unwrap();
    host.quit().unwrap();
    assert_eq!(
        fs::read_to_string(marker).unwrap(),
        "path-token 42 C:/a b.mp4\npath-token 43\nquit\n"
    );
}

#[test]
fn overlong_and_partial_lines_are_ignored() {
    let (tx, _rx) = sync_channel(4);
    let shared = Shared {
        state: Mutex::new(HostState {
            clock: None,
            bounds: None,
            source_aspect: None,
            pointers: VecDeque::new(),
        }),
        tx,
    };
    let mut payload = vec![b'x'; 5000];
    payload.extend_from_slice(b"\naudio-clock 4 0 1\n");
    let mut reader = BufReader::with_capacity(1024, Cursor::new(payload));
    let mut buf = Vec::new();
    pump(&mut reader, &mut buf, &shared).unwrap();
    assert!((lock(&shared.state).clock.unwrap().time - 4.0).abs() < 1e-12);

    let (tx, _rx) = sync_channel(4);
    let shared = Shared {
        state: Mutex::new(HostState {
            clock: None,
            bounds: None,
            source_aspect: None,
            pointers: VecDeque::new(),
        }),
        tx,
    };
    let mut reader = BufReader::new(Cursor::new(&b"audio-clock 5 1 1"[..]));
    pump(&mut reader, &mut buf, &shared).unwrap();
    assert!(lock(&shared.state).clock.is_none());
}

#[test]
fn quit_kills_a_child_that_ignores_stdin() {
    let cmd = crate::test_support::command("stubborn");
    let host = spawn_child(cmd, Stdio::null()).unwrap();
    assert!(host.is_running());
    let started = Instant::now();
    host.quit().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!host.is_running());
    assert!(matches!(host.seek(1.0), Err(PreviewError::Closed)));
    host.quit().unwrap();
}

#[test]
fn stdin_stdout_and_quit_roundtrip() {
    let dir = TempDir::new("clock");
    let marker = dir.0.join("quit.txt");
    let mut cmd = crate::test_support::command("clock");
    cmd.env("TODO_TEST_PROCESS_MARKER", &marker);
    let host = spawn_child(cmd, Stdio::null()).unwrap();
    assert!(wait_until(&host, |h| {
        h.clock_sample()
            .is_some_and(|c| (c.time - 3.25).abs() < 1e-9)
            && h.preview_bounds().is_some_and(|b| b.visible == Some(true))
    }));
    assert!(matches!(host.try_recv(), Some(HostEvent::AudioClock(_))));
    host.seek(1.5).unwrap();
    assert!(wait_until(&host, |h| {
        h.clock_sample()
            .is_some_and(|c| (c.time - 9.0).abs() < 1e-9 && !c.playing && c.available)
    }));
    let read = host.read_clock().unwrap();
    assert!(!read.playing && read.available);
    drop(host);
    assert_eq!(fs::read_to_string(&marker).unwrap(), "quit");
}

fn wait_until(host: &PreviewHost, pred: impl Fn(&PreviewHost) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if pred(host) {
            return true;
        }
        thread::sleep(Duration::from_millis(15));
    }
    false
}
