use super::{capture, capture_with_diagnostics, Duplex, Output, ProcessError};
use crate::test_support;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

#[test]
fn cancellation_reaps_a_stubborn_native_job_promptly() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        signal.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let result = capture(
        test_support::command("stubborn"),
        Duration::from_secs(3),
        128,
        Some(cancelled),
    );
    trigger.join().unwrap();
    assert!(matches!(result, Err(ProcessError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn already_cancelled_jobs_do_not_try_to_spawn_the_tool() {
    let result = capture(
        std::process::Command::new("missing-edit-proxy-tool"),
        Duration::from_secs(3),
        128,
        Some(Arc::new(AtomicBool::new(true))),
    );
    assert!(matches!(result, Err(ProcessError::Cancelled)));
}

#[test]
fn pipes_are_drained_without_deadlock_and_diagnostics_remain_bounded() {
    assert!(capture(
        test_support::command("stderr"),
        Duration::from_secs(3),
        128,
        None
    )
    .unwrap()
    .ends_with(b"done"));
    let error = capture(
        test_support::command("fail"),
        Duration::from_secs(3),
        128,
        None,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("exit 7"));
    assert!(message.len() < 5000);
}

#[test]
fn successful_processes_can_return_bounded_stderr_for_measurements() {
    let (_stdout, stderr) = capture_with_diagnostics(
        test_support::command("stderr"),
        Duration::from_secs(3),
        128,
        None,
    )
    .unwrap();
    assert!(stderr.contains("diagnostic") || !stderr.is_empty());
}
#[test]
fn output_limits_and_timeouts_terminate_native_jobs() {
    assert!(capture(
        test_support::command("large"),
        Duration::from_secs(3),
        32,
        None
    )
    .is_err());
    let started = Instant::now();
    assert!(matches!(
        capture(
            test_support::command("stubborn"),
            Duration::from_millis(80),
            128,
            None
        ),
        Err(ProcessError::Timeout(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

fn echoed(receiver: &std::sync::mpsc::Receiver<Output>) -> Option<String> {
    loop {
        match receiver.recv_timeout(Duration::from_secs(5)).ok()? {
            // The test harness prints its own lines; only the fixture's count.
            Output::Line(line) if line.starts_with("echo:") => return Some(line),
            Output::Line(_) => {}
            _ => return None,
        }
    }
}

#[test]
fn duplex_round_trips_lines_and_reports_the_end_of_output() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut child = Duplex::spawn(test_support::command("echo"), sender, |output| output).unwrap();
    child.send_line("hello").unwrap();
    assert_eq!(echoed(&receiver).as_deref(), Some("echo:5"));
    let long = "x".repeat(2 * 1024 * 1024);
    child.send_line(&long).unwrap();
    assert_eq!(echoed(&receiver), Some(format!("echo:{}", long.len())));
    child.send_line("quit").unwrap();
    assert_eq!(echoed(&receiver).as_deref(), Some("echo:4"));
    assert_eq!(echoed(&receiver), None);
}

#[test]
fn duplex_writes_never_block_on_a_child_that_stops_reading() {
    let (sender, _receiver) = std::sync::mpsc::channel();
    let mut child = Duplex::spawn(test_support::command("deaf"), sender, |output| output).unwrap();
    let started = Instant::now();
    let chunk = "y".repeat(1024 * 1024);
    for _ in 0..8 {
        child.send_line(&chunk).unwrap();
    }
    assert!(started.elapsed() < Duration::from_millis(500));
    child.kill();
    assert!(child.exited());
    assert!(child.send_line("late").is_err());
}
