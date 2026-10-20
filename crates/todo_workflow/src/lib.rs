//! Workflow / extension packages (`todo-extension/1` + optional `todo-graph/1`):
//! discovery, installation, worker pool, and graph projection for the workflow
//! panel. Manifest / protocol / items / node catalog live in
//! `todo_production::extension`.
pub mod budget;
pub mod check;
pub mod install;
pub mod pool;
pub mod registry;
pub mod runner;
pub mod runtime;
pub mod workflow;

pub use budget::Budget;
pub use check::{check_extension, check_extension_fixtures, load_and_check, CheckReport};
pub use install::{install, pack, uninstall, InstallError};
pub use pool::{PoolHandle, PoolOptions, RunId, RunRequest, WorkerEvent, WorkerPool};
pub use registry::{load, Extension, Origin, Registry, Rejected, MANIFEST_FILE};
pub use runner::{run, segment_script, NodeRun, RunInput, RunReport, Services};
pub use runtime::{worker_command, RuntimeError, RuntimePaths};
pub use workflow::{
    copy_package, create_blank_workflow, Workflow, WorkflowNodeCard, WorkflowPortCard,
};
