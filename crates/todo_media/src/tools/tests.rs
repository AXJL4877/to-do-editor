use super::process::capture_command;
use crate::{test_support, MediaError};
use std::time::{Duration, Instant};

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
