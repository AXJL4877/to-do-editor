use super::process::{capture_command, capture_command_cancellable};
use crate::{test_support, MediaError};
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
    let result = capture_command_cancellable(
        test_support::command("stubborn"),
        Duration::from_secs(3),
        128,
        cancelled,
    );
    trigger.join().unwrap();
    assert!(matches!(result, Err(MediaError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn already_cancelled_jobs_do_not_try_to_spawn_the_tool() {
    let result = capture_command_cancellable(
        std::process::Command::new("missing-edit-proxy-tool"),
        Duration::from_secs(3),
        128,
        Arc::new(AtomicBool::new(true)),
    );
    assert!(matches!(result, Err(MediaError::Cancelled)));
}

#[test]
fn pipes_are_drained_without_deadlock_and_diagnostics_remain_bounded() {
    assert!(
        capture_command(test_support::command("stderr"), Duration::from_secs(3), 128)
            .unwrap()
            .ends_with(b"done")
    );
    let error =
        capture_command(test_support::command("fail"), Duration::from_secs(3), 128).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("exit 7"));
    assert!(message.len() < 5000);
}
#[test]
fn output_limits_and_timeouts_terminate_native_jobs() {
    assert!(capture_command(test_support::command("large"), Duration::from_secs(3), 32).is_err());
    let started = Instant::now();
    assert!(matches!(
        capture_command(
            test_support::command("stubborn"),
            Duration::from_millis(80),
            128
        ),
        Err(MediaError::Timeout(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}
