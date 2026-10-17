//! Extension packages (`todo-extension/1`): discovery, installation and, next,
//! the worker process pool. The standard itself (manifest, protocol, items)
//! is pure data in `todo_production::extension`.
pub mod budget;
pub mod check;
pub mod install;
pub mod pool;
pub mod registry;
pub mod runtime;

pub use budget::Budget;
pub use check::{check_extension, check_extension_fixtures, load_and_check, CheckReport};
pub use install::{install, pack, uninstall, InstallError};
pub use pool::{PoolHandle, PoolOptions, RunId, RunRequest, WorkerEvent, WorkerPool};
pub use registry::{load, Extension, Origin, Registry, Rejected, MANIFEST_FILE};
pub use runtime::{worker_command, RuntimeError, RuntimePaths};
