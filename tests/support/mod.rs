//! Use the Rust test executable itself as a child; no runtime compiler or script host.
mod process_fixture;
use std::process::Command;

pub fn command(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "test_support::subprocess_fixture", "--nocapture"])
        .env("TODO_TEST_PROCESS_MODE", mode);
    command
}

#[test]
fn subprocess_fixture() {
    let Ok(mode) = std::env::var("TODO_TEST_PROCESS_MODE") else {
        return;
    };
    let marker = std::env::var_os("TODO_TEST_PROCESS_MARKER");
    process_fixture::run(&mode, marker.as_deref());
    std::process::exit(0);
}
