use std::path::PathBuf;
use todo_extensions::load_and_check;

fn main() {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: check_extension <package>");
        std::process::exit(2);
    };
    let root = std::env::current_dir().expect("current directory");
    match load_and_check(&path, &root) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": report.id,
                "version": report.version,
                "ops": report.ops,
                "ready": report.ready,
                "fixtureRuns": report.fixture_runs,
            }))
            .expect("report serializes")
        ),
        Err(error) => {
            eprintln!("extension check failed: {error}");
            std::process::exit(1);
        }
    }
}
