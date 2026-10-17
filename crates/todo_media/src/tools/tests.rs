//! Supervision itself is tested in `todo_process`; these pin the MediaError mapping.
use super::process::{capture_command, execute_command};
use crate::{test_support, MediaError};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[test]
fn process_failures_map_to_media_errors() {
    let error = capture_command(
        test_support::command("fail"),
        Duration::from_secs(3),
        128,
        None,
    )
    .unwrap_err();
    assert!(
        matches!(&error, MediaError::Tool { message, .. } if message.contains("exit 7")),
        "{error}"
    );
    assert!(matches!(
        capture_command(
            test_support::command("stubborn"),
            Duration::from_millis(80),
            128,
            None
        ),
        Err(MediaError::Timeout(_))
    ));
    assert!(matches!(
        capture_command(
            test_support::command("stubborn"),
            Duration::from_secs(3),
            128,
            Some(Arc::new(AtomicBool::new(true)))
        ),
        Err(MediaError::Cancelled)
    ));
}

#[test]
fn reader_errors_survive_unchanged() {
    let error = execute_command(
        test_support::command("stderr"),
        Duration::from_secs(3),
        |mut stdout| {
            // Drain first: a reader that drops the pipe makes the child fail, and a
            // failed exit takes precedence over the reader's own error.
            std::io::copy(&mut stdout, &mut std::io::sink()).unwrap();
            Err::<(), _>(MediaError::Invalid("reader said no".into()))
        },
    )
    .unwrap_err();
    assert!(matches!(error, MediaError::Invalid(message) if message == "reader said no"));
}
