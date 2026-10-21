//! Start an OS file drag so the user can drop an image into ComfyUI (or any app).

use std::path::{Path, PathBuf};

use raw_window_handle::HasWindowHandle;

/// Begin dragging `path` out of the app. On success the OS drag loop runs until
/// the user drops or cancels. Returns an error message if drag cannot start.
pub fn drag_file(window: &impl HasWindowHandle, path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("File not found: {}", path.display()));
    }
    let absolute = dunce_canonicalize(path)?;
    platform_drag_file(window, &absolute)
}

fn dunce_canonicalize(path: &Path) -> Result<PathBuf, String> {
    // Prefer a path without the Windows `\\?\` prefix when possible — some drop
    // targets (browsers) are happier with ordinary drive paths.
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("Could not resolve {}: {error}", path.display()))?;
    Ok(strip_verbatim_prefix(canonical))
}

fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

#[cfg(windows)]
fn platform_drag_file(window: &impl HasWindowHandle, path: &Path) -> Result<(), String> {
    let item = drag::DragItem::Files(vec![path.to_path_buf()]);
    let preview = drag::Image::File(path.to_path_buf());
    drag::start_drag(
        window,
        item,
        preview,
        |_result, _cursor| {},
        drag::Options::default(),
    )
    .map_err(|error| format!("Could not start file drag: {error}"))
}

#[cfg(target_os = "linux")]
fn platform_drag_file(_window: &impl HasWindowHandle, path: &Path) -> Result<(), String> {
    // eframe/winit has no GTK window for the `drag` crate on Linux. Reveal the
    // file in the file manager so the user can drag it into ComfyUI instead.
    let folder = path.parent().unwrap_or(path);
    let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
    Err(format!(
        "Opened folder — drag {} into the ComfyUI browser window.",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("the image")
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_drag_file(_window: &impl HasWindowHandle, path: &Path) -> Result<(), String> {
    Err(format!(
        "Drag-out is not supported on this OS. Open {} from your file manager and drop it into ComfyUI.",
        path.display()
    ))
}
