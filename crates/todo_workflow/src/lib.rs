//! Workflow packages (`todo-extension/1` + optional `todo-graph/1`):
//! standards, discovery, installation, worker pool, and graph execution.
pub mod budget;
pub mod check;
pub mod install;
pub mod nodes;
pub mod package;
pub mod pool;
pub mod registry;
pub mod runner;
pub mod runtime;
pub mod workflow;

pub use budget::Budget;
pub use check::{check_extension, check_extension_fixtures, load_and_check, CheckReport};
pub use install::{install, pack, uninstall, validate_package, InstallError};
pub use package::{
    copy_package, create_blank_workflow, Workflow, WorkflowNodeCard, WorkflowPortCard,
};
pub use pool::{PoolHandle, PoolOptions, RunId, RunRequest, WorkerEvent, WorkerPool};
pub use registry::{load, Extension, Origin, Registry, Rejected, MANIFEST_FILE};
pub use runner::{run, segment_script, NodeRun, RunInput, RunReport, Services};
pub use runtime::{worker_command, RuntimeError, RuntimePaths};
