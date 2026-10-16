use crate::MediaError;
use std::{
    ffi::OsString,
    io::{BufReader, Read},
    path::Path,
    process::{ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub(super) fn capture_command_cancellable(
    command: Command,
    timeout: Duration,
    limit: usize,
    cancelled: Arc<AtomicBool>,
) -> Result<Vec<u8>, MediaError> {
    supervise(command, timeout, Some(cancelled), move |stdout| {
        read_capture(stdout, limit)
    })
}

pub(crate) fn capture_cancellable(
    tool: &Path,
    args: Vec<OsString>,
    timeout: Duration,
    limit: usize,
    cancelled: Arc<AtomicBool>,
) -> Result<Vec<u8>, MediaError> {
    let mut command = Command::new(tool);
    command.args(args);
    capture_command_cancellable(command, timeout, limit, cancelled)
}

pub(crate) fn capture(
    tool: &Path,
    args: Vec<OsString>,
    timeout: Duration,
    limit: usize,
) -> Result<Vec<u8>, MediaError> {
    let mut command = Command::new(tool);
    command.args(args);
    capture_command(command, timeout, limit)
}

pub(super) fn capture_command(
    command: Command,
    timeout: Duration,
    limit: usize,
) -> Result<Vec<u8>, MediaError> {
    supervise(command, timeout, None, move |stdout| {
        read_capture(stdout, limit)
    })
}

fn read_capture(stdout: ChildStdout, limit: usize) -> Result<Vec<u8>, MediaError> {
    let mut output = Vec::new();
    stdout
        .take(limit as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|error| MediaError::Invalid(format!("tool output: {error}")))?;
    if output.len() > limit {
        return Err(MediaError::Invalid(
            "media tool output exceeded its limit".into(),
        ));
    }
    Ok(output)
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
    let mut command = Command::new(tool);
    command.args(args);
    supervise(command, timeout, None, read)
}

fn supervise<T, F>(
    mut command: Command,
    timeout: Duration,
    cancelled: Option<Arc<AtomicBool>>,
    read: F,
) -> Result<T, MediaError>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T, MediaError> + Send + 'static,
{
    let is_cancelled = || {
        cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    };
    if is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let tool = std::path::PathBuf::from(command.get_program());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|error| MediaError::Tool {
        tool: tool.clone(),
        message: error.to_string(),
    })?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let output = thread::spawn(move || read(stdout));
    let diagnostics = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut kept = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let keep = count.min(4096 - kept.len());
            kept.extend_from_slice(&buffer[..keep]);
        }
        String::from_utf8_lossy(&kept).trim().to_owned()
    });
    let started = Instant::now();
    let status = loop {
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            break Err(MediaError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(MediaError::Timeout(tool.clone()));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(MediaError::io(&tool, error));
            }
        }
    };
    let result = output
        .join()
        .map_err(|_| MediaError::Invalid("media output reader panicked".into()))?;
    let stderr = diagnostics.join().unwrap_or_default();
    let status = status?;
    if is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    if !status.success() {
        return Err(MediaError::Tool {
            tool,
            message: format!(
                "exit {}: {stderr}",
                status
                    .code()
                    .map_or_else(|| "terminated".into(), |code| code.to_string())
            ),
        });
    }
    result
}
