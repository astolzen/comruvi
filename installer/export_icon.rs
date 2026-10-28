//! One-shot helper: rasterize cv-ivon.svg to a BMP for the MSI warning dialog.

use std::env;
use std::path::PathBuf;

fn main() {
    let out = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("installer/wix/comruvi-icon.bmp"));

    let image = egui_extras::image::load_svg_bytes_with_size(
        include_bytes!("../cv-ivon.svg"),
        Some(egui::SizeHint::Size(64, 64)),
    )
    .expect("rasterize cv-ivon.svg");

    image::save_buffer(
        &out,
        image.as_raw(),
        image.width() as u32,
        image.height() as u32,
        image::ColorType::Rgba8,
    )
    .unwrap_or_else(|error| panic!("write {}: {error}", out.display()));

    eprintln!("Wrote {}", out.display());
}
