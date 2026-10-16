use std::{
    fs,
    io::{self, BufRead, Write},
    time::Duration,
};

fn emit(value: &str) {
    print!("{value}");
    io::stdout().flush().unwrap();
}
pub(super) fn run(mode: &str, marker: Option<&std::ffi::OsStr>) {
    match mode {
        "stubborn" => {
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        "stderr" => {
            io::stderr().write_all(&vec![b'e'; 1024 * 1024]).unwrap();
            emit("done");
            return;
        }
        "fail" => {
            io::stderr().write_all(&vec![b'e'; 8192]).unwrap();
            std::process::exit(7);
        }
        "large" => {
            let _ = io::stdout().write_all(&vec![b'x'; 1024 * 1024]);
            return;
        }
        "source" => emit(&"audio-clock 1 0 1\n".repeat(65)),
        "clock" => emit("audio-clock 3.250000000 1 0\npreview-bounds 8 9 100 80 1\n"),
        _ => {}
    }
    let mut lines = Vec::new();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        lines.push(line.clone());
        match mode {
            "transform" if line == "transform 0.25 -0.5 2" => emit("preview-pointer down 0.25 -0.5\npreview-pointer move 1.25 -0.25\npreview-pointer up 1.25 -0.25\n"),
            "gizmo" if line == "transport 42 0" => { fs::write(marker.unwrap(), lines.join("\n")).unwrap(); emit("audio-clock 42 0 1\n"); }
            "source" => {
                use fs::OpenOptions;
                writeln!(OpenOptions::new().create(true).append(true).open(marker.unwrap()).unwrap(), "{line}").unwrap();
                if line == "path-token 42 C:/a b.mp4" { emit("preview-pointer down 0.5 0.5\nsource-aspect 41 1\nsource-aspect 42 1.3333333333333333\n"); }
            }
            "clock" if line.starts_with("seek ") => emit("audio-clock 9.000000000 0 1\n"),
            _ => {}
        }
        if line == "quit" {
            if mode == "clock" {
                fs::write(marker.unwrap(), "quit").unwrap();
            }
            break;
        }
    }
}
