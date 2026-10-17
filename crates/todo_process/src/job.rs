//! Windows Job Object ownership for extension worker processes.
//!
//! The non-Windows implementation is deliberately a no-op so the pure Rust
//! protocol tests can run on other hosts. The Windows implementation always
//! enables kill-on-close and can optionally cap each process's private memory.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("无法创建工作程序 Job Object：{0}")]
    Create(String),
    #[error("无法配置工作程序 Job Object：{0}")]
    Configure(String),
    #[error("无法把工作程序加入 Job Object：{0}")]
    Assign(String),
}

#[cfg(windows)]
mod platform {
    use super::JobError;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };

    pub struct Job {
        handle: HANDLE,
    }

    impl Job {
        pub fn new(memory_limit: Option<u64>) -> Result<Self, JobError> {
            // A null name creates an unnamed object owned by this process.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(JobError::Create(last_error()));
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(bytes) = memory_limit {
                limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                limits.ProcessMemoryLimit = bytes as usize;
            }
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &mut limits as *mut _ as *mut _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                unsafe { CloseHandle(handle) };
                return Err(JobError::Configure(last_error()));
            }
            Ok(Self { handle })
        }

        pub fn assign(&self, child: &Child) -> Result<(), JobError> {
            let process = child.as_raw_handle() as HANDLE;
            if unsafe { AssignProcessToJobObject(self.handle, process) } == 0 {
                return Err(JobError::Assign(last_error()));
            }
            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe { CloseHandle(self.handle) };
                self.handle = std::ptr::null_mut();
            }
        }
    }

    fn last_error() -> String {
        format!("Windows error {}", unsafe { GetLastError() })
    }
}

#[cfg(not(windows))]
mod platform {
    use super::JobError;
    use std::process::Child;

    pub struct Job;

    impl Job {
        pub fn new(_: Option<u64>) -> Result<Self, JobError> {
            Ok(Self)
        }

        pub fn assign(&self, _: &Child) -> Result<(), JobError> {
            Ok(())
        }
    }
}

pub use platform::Job;
