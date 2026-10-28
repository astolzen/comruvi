//! Rasterize `cv-ivon.svg` for the MSI dialog (BMP) and the PE / shortcuts (ICO).

use std::env;
use std::fs::File;
use std::path::PathBuf;

fn raster(size: u32) -> egui::ColorImage {
    egui_extras::image::load_svg_bytes_with_size(
        include_bytes!("../cv-ivon.svg"),
        Some(egui::SizeHint::Size(size, size)),
    )
    .unwrap_or_else(|e| panic!("rasterize cv-ivon.svg at {size}px: {e}"))
}

fn write_bmp(path: &PathBuf) {
    let image = raster(64);
    image::save_buffer(
        path,
        image.as_raw(),
        image.width() as u32,
        image.height() as u32,
        image::ColorType::Rgba8,
    )
    .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("Wrote {}", path.display());
}

fn write_ico(path: &PathBuf) {
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16_u32, 32, 48, 64, 128, 256] {
        let image = raster(size);
        let icon_image = ico::IconImage::from_rgba_data(size, size, image.as_raw().to_vec());
        dir.add_entry(
            ico::IconDirEntry::encode(&icon_image)
                .unwrap_or_else(|e| panic!("encode {size}px icon: {e}")),
        );
    }
    let file = File::create(path).unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    dir.write(file)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("Wrote {}", path.display());
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let args: Vec<String> = env::args().skip(1).collect();

    // `export-icon [bmp-path]` — always refresh repo-root comruvi.ico; BMP path is optional
    // (defaults to installer/wix/comruvi-icon.bmp for the MSI warning dialog).
    let bmp = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("installer/wix/comruvi-icon.bmp"));
    if let Some(parent) = bmp.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    write_bmp(&bmp);
    write_ico(&root.join("comruvi.ico"));
}
