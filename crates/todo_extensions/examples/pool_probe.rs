//! Debug: run one op of a package through the worker pool and print events.
use std::sync::{mpsc, Arc};
use std::time::Duration;
use todo_extensions::{load, Budget, Origin, PoolOptions, RunRequest, RuntimePaths, WorkerPool};
use todo_production::extension::protocol::Canvas;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&args[1]);
    let package = std::path::PathBuf::from(&args[2]);
    let op = args[3].clone();
    let workdir = std::path::PathBuf::from(&args[4]);
    let input: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&args[5]).unwrap()).unwrap();
    let ext = Arc::new(load(&package, Origin::Builtin).unwrap());
    let (tx, rx) = mpsc::channel();
    let pool = WorkerPool::start(
        PoolOptions {
            runtime: RuntimePaths::new(root),
            ..PoolOptions::default()
        },
        tx,
    );
    pool.run(RunRequest {
        extension: ext.clone(),
        op,
        input,
        params: serde_json::json!({}),
        canvas: Canvas {
            width: 960,
            height: 540,
            fps: 30.0,
        },
        workdir,
        seed: 1,
        budget: Arc::new(Budget::new(&ext.manifest.budget)),
    });
    loop {
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok(event) => {
                let text = format!("{event:?}");
                println!("{}", text.chars().take(400).collect::<String>());
                if text.starts_with("Done") || text.starts_with("Failed") {
                    break;
                }
            }
            Err(e) => {
                println!("timeout {e:?}");
                break;
            }
        }
    }
}
