//! Render the reference frame (s00_intro1_hold.png) to `target/text_reference.png`.
//!
//! cargo run -p todo_text --example reference --release
use std::path::PathBuf;
use std::time::Instant;
use todo_core::{builtin_text_styles, TextContent, DEFAULT_TEXT_STYLE_ID};
use todo_text::{render, FontLibrary, RasterRequest};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut fonts = FontLibrary::new([
        root.join("dependencies/fonts"),
        PathBuf::from(r"C:\Windows\Fonts"),
    ]);
    let style = builtin_text_styles()
        .into_iter()
        .find(|s| s.id == DEFAULT_TEXT_STYLE_ID)
        .unwrap();
    let mut content = TextContent::new("钱越来越 不值钱了", DEFAULT_TEXT_STYLE_ID);
    content.layout.font_size = 0.155;
    let request = RasterRequest {
        content: &content,
        style: &style,
        canvas: (1920, 1080),
        reveal: None,
    };
    // First call includes reading the font file.
    let started = Instant::now();
    render(&mut fonts, request).expect("render");
    let cold = started.elapsed();
    let started = Instant::now();
    let image = render(&mut fonts, request).expect("render");
    let warm = started.elapsed();
    let started = Instant::now();
    let png = image.encode_png().expect("png");
    let encode = started.elapsed();
    let out = root.join("target/text_reference.png");
    std::fs::write(&out, png).expect("write");
    println!(
        "wrote {} (cold {cold:?}, warm {warm:?}, png {encode:?})",
        out.display()
    );
}
