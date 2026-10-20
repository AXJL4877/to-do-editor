//! Render one built-in text style to a transparent PNG, to look at a style.
//!
//! cargo run -p todo_text --example style --release -- <style-id> "<text>" [out.png] [font-size]
use std::path::PathBuf;
use todo_core::{builtin_text_styles, TextContent};
use todo_text::{render, FontLibrary, RasterRequest};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(id), Some(text)) = (args.first(), args.get(1)) else {
        eprintln!("usage: style <style-id> <text> [out.png] [font-size]");
        std::process::exit(2);
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(format!("target/style-{id}.png")));
    let size = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.1);
    let mut fonts = FontLibrary::new([
        root.join("dependencies/fonts"),
        PathBuf::from(r"C:\Windows\Fonts"),
    ]);
    let Some(style) = builtin_text_styles().into_iter().find(|s| &s.id == id) else {
        eprintln!("no built-in style {id}");
        std::process::exit(2);
    };
    let mut content = TextContent::new(text.as_str(), id.as_str());
    content.layout.font_size = size;
    let image = render(
        &mut fonts,
        RasterRequest {
            content: &content,
            style: &style,
            canvas: (1920, 1080),
            reveal: None,
        },
    )
    .expect("render");
    std::fs::write(&out, image.encode_png().expect("png")).expect("write");
    println!("wrote {}", out.display());
}
