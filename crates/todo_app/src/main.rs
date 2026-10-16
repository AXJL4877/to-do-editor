mod adapters;
mod runtime;
use eframe::egui;
fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([960.0, 640.0])
        .with_decorations(false)
        .with_transparent(false)
        .with_title("TO DO");
    eframe::run_native(
        "TO DO",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(runtime::App::new(cc)))),
    )
}
