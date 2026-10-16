use std::{env, process, time::Instant};
use todo_media::MediaService;

fn main() {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        eprintln!("usage: cargo run -p todo_media --example edit_proxy -- <repo_root> <stored_or_absolute_path>");
        process::exit(2);
    }
    let Some(stored) = arguments[1].to_str() else {
        eprintln!("media path must be valid Unicode");
        process::exit(2);
    };
    let service = MediaService::new(&arguments[0]);
    let started = Instant::now();
    match service.get_edit_proxy(stored) {
        Ok(proxy) => {
            println!("path={}", proxy.path.display());
            println!(
                "source={}x{} proxy={}x{}",
                proxy.source_width, proxy.source_height, proxy.width, proxy.height
            );
            println!("elapsed_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
        }
        Err(error) => {
            eprintln!(
                "editing proxy failed after {:.3}s: {error}",
                started.elapsed().as_secs_f64()
            );
            process::exit(1);
        }
    }
}
