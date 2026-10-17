//! Supervised native child processes: hidden window, both pipes drained
//! concurrently, bounded diagnostics, timeout and cancellation with kill + wait.
//! Shared by media tools (FFmpeg/FFprobe) and extension bakers; callers map
//! [`ProcessError`] into their own error type.
mod duplex;
mod job;

pub use duplex::{Duplex, Output, MAX_LINE};
pub use job::{Job, JobError};

use std::{
    io::{BufReader, Read},
    path::PathBuf,
    process::{ChildStdout, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// Bytes of stderr kept for diagnostics.
pub const DIAGNOSTIC_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("process cancelled")]
    Cancelled,
    #[error("{tool}: {message}")]
    Spawn { tool: PathBuf, message: String },
    #[error("{tool}: {source}")]
    Io {
        tool: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} timed out")]
    Timeout(PathBuf),
    #[error("{tool}: exit {}: {stderr}", code.map_or_else(|| "terminated".into(), |code| code.to_string()))]
    Exit {
        tool: PathBuf,
        code: Option<i32>,
        stderr: String,
    },
    /// The stdout reader rejected the output (limit, format) or panicked.
    #[error("{0}")]
    Output(String),
    #[error("{0}")]
    Job(#[from] JobError),
}

/// Run `command`, read stdout with `read` on its own thread, and supervise the
/// child on the calling thread. `cancelled` is polled every 10 ms.
pub fn supervise<T, F>(
    command: Command,
    timeout: Duration,
    cancelled: Option<Arc<AtomicBool>>,
    read: F,
) -> Result<T, ProcessError>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T, ProcessError> + Send + 'static,
{
    supervise_with_diagnostics(command, timeout, cancelled, read).map(|(value, _)| value)
}

/// Like [`supervise`] but keeps the bounded stderr tail on successful exits.
/// Media quality filters such as ebur128 and psnr report measurements on
/// stderr even when FFmpeg exits successfully.
pub fn supervise_with_diagnostics<T, F>(
    command: Command,
    timeout: Duration,
    cancelled: Option<Arc<AtomicBool>>,
    read: F,
) -> Result<(T, String), ProcessError>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T, ProcessError> + Send + 'static,
{
    let mut command = command;
    let is_cancelled = || {
        cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    };
    if is_cancelled() {
        return Err(ProcessError::Cancelled);
    }
    let tool = PathBuf::from(command.get_program());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut command);
    let mut child = command.spawn().map_err(|error| ProcessError::Spawn {
        tool: tool.clone(),
        message: error.to_string(),
    })?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let output = thread::spawn(move || read(stdout));
    let diagnostics = thread::spawn(move || keep_diagnostics(stderr));
    let started = Instant::now();
    let status: Result<ExitStatus, ProcessError> = loop {
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            break Err(ProcessError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(ProcessError::Timeout(tool.clone()));
            }
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(ProcessError::Io {
                    tool: tool.clone(),
                    source,
                });
            }
        }
    };
    let result = output
        .join()
        .map_err(|_| ProcessError::Output("process output reader panicked".into()))?;
    let stderr = diagnostics.join().unwrap_or_default();
    let status = status?;
    if is_cancelled() {
        return Err(ProcessError::Cancelled);
    }
    if !status.success() {
        return Err(ProcessError::Exit {
            tool,
            code: status.code(),
            stderr,
        });
    }
    Ok((result?, stderr))
}

/// Collect stdout, failing once it exceeds `limit` bytes.
pub fn capture(
    command: Command,
    timeout: Duration,
    limit: usize,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<Vec<u8>, ProcessError> {
    supervise(command, timeout, cancelled, move |stdout| {
        read_capture(stdout, limit)
    })
}

pub fn capture_with_diagnostics(
    command: Command,
    timeout: Duration,
    limit: usize,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<(Vec<u8>, String), ProcessError> {
    supervise_with_diagnostics(command, timeout, cancelled, move |stdout| {
        read_capture(stdout, limit)
    })
}

fn read_capture(stdout: ChildStdout, limit: usize) -> Result<Vec<u8>, ProcessError> {
    let mut output = Vec::new();
    stdout
        .take(limit as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|error| ProcessError::Output(format!("tool output: {error}")))?;
    if output.len() > limit {
        return Err(ProcessError::Output(
            "tool output exceeded its limit".into(),
        ));
    }
    Ok(output)
}

fn keep_diagnostics(stderr: impl Read) -> String {
    let mut reader = BufReader::new(stderr);
    let mut kept = Vec::new();
    let mut buffer = [0; 4096];
    while let Ok(count) = reader.read(&mut buffer) {
        if count == 0 {
            break;
        }
        // Keep draining after the budget so the child never blocks on a full pipe.
        let keep = count.min(DIAGNOSTIC_BYTES - kept.len());
        kept.extend_from_slice(&buffer[..keep]);
    }
    String::from_utf8_lossy(&kept).trim().to_owned()
}

/// Child processes never flash a console window.
pub fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
#[path = "../../../tests/support/mod.rs"]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
