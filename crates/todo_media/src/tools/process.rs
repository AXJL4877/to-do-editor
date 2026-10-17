//! FFmpeg/FFprobe jobs on top of the shared `todo_process` supervisor.
use crate::MediaError;
use std::{
    ffi::OsString,
    path::Path,
    process::{ChildStdout, Command},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use todo_process::ProcessError;

impl From<ProcessError> for MediaError {
    fn from(error: ProcessError) -> Self {
        match error {
            ProcessError::Cancelled => Self::Cancelled,
            ProcessError::Spawn { tool, message } => Self::Tool { tool, message },
            ProcessError::Io { tool, source } => Self::io(&tool, source),
            ProcessError::Timeout(tool) => Self::Timeout(tool),
            ProcessError::Exit { tool, code, stderr } => Self::Tool {
                tool,
                message: format!(
                    "exit {}: {stderr}",
                    code.map_or_else(|| "terminated".into(), |code| code.to_string())
                ),
            },
            ProcessError::Output(message) => Self::Invalid(message),
            ProcessError::Job(error) => Self::Invalid(error.to_string()),
        }
    }
}

fn command(tool: &Path, args: Vec<OsString>) -> Command {
    let mut command = Command::new(tool);
    command.args(args);
    command
}

pub(super) fn capture_command(
    command: Command,
    timeout: Duration,
    limit: usize,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<Vec<u8>, MediaError> {
    Ok(todo_process::capture(command, timeout, limit, cancelled)?)
}

pub(crate) fn capture_cancellable(
    tool: &Path,
    args: Vec<OsString>,
    timeout: Duration,
    limit: usize,
    cancelled: Arc<AtomicBool>,
) -> Result<Vec<u8>, MediaError> {
    capture_command(command(tool, args), timeout, limit, Some(cancelled))
}

pub(crate) fn capture_with_diagnostics(
    tool: &Path,
    args: Vec<OsString>,
    timeout: Duration,
) -> Result<(Vec<u8>, String), MediaError> {
    let command = command(tool, args);
    Ok(todo_process::capture_with_diagnostics(
        command,
        timeout,
        4 * 1024 * 1024,
        None,
    )?)
}

pub(crate) fn capture(
    tool: &Path,
    args: Vec<OsString>,
    timeout: Duration,
    limit: usize,
) -> Result<Vec<u8>, MediaError> {
    capture_command(command(tool, args), timeout, limit, None)
}

/// Drain both pipes concurrently and supervise the native child on this worker.
pub(crate) fn execute<T, F>(
    tool: &Path,
    args: Vec<OsString>,
    timeout: Duration,
    read: F,
) -> Result<T, MediaError>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T, MediaError> + Send + 'static,
{
    execute_command(command(tool, args), timeout, read)
}

pub(super) fn execute_command<T, F>(
    command: Command,
    timeout: Duration,
    read: F,
) -> Result<T, MediaError>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T, MediaError> + Send + 'static,
{
    // The reader's own MediaError survives the process boundary unchanged;
    // timeouts, cancellation and failed exits still take precedence.
    let slot = Arc::new(std::sync::Mutex::new(None));
    let kept = slot.clone();
    let outcome = todo_process::supervise(command, timeout, None, move |stdout| {
        read(stdout).map_err(|error| {
            let message = error.to_string();
            *kept.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(error);
            ProcessError::Output(message)
        })
    });
    let reader_error = matches!(&outcome, Err(ProcessError::Output(_)))
        .then(|| {
            slot.lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take()
        })
        .flatten();
    match reader_error {
        Some(error) => Err(error),
        None => Ok(outcome?),
    }
}
