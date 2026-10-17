# comruvi

**comruvi** (ComfyUI Rust Viewer) is a local-first image browser intended for
AI-generated images. It shows folders, thumbnail and filmstrip views, and the
generation metadata embedded in an image. It currently supports PNG and JPEG on
Windows; the UI and filesystem code are deliberately platform-neutral for a later
macOS/Linux release.

comruvi is a fully AI coded Project using both ChatGPT 5.6 and CursorAI.

## Install and run

Install the current stable Rust toolchain from [rustup](https://rustup.rs/), then
clone or download this project and run:

```powershell
cargo run --release -- <folder-with-images>
```

Alternatively, use **Open folder** in the app. Original images are never written.
The only files comruvi creates are PNG thumbnails under the selected folder's
`.thumbnails` directory. Their names follow Geeqie's compatible convention:
`image.jpg` becomes `.thumbnails/image.jpg.png`.

Supported input formats in this alpha release are PNG, JPG, and JPEG. Builds are
currently tested on Windows; the application uses portable Rust APIs in preparation
for macOS and Linux support.

## Using the viewer

- The **Folders** pane is a tree. Select a folder to browse it; use **↑ Up** to
  move to its parent. The lower **Favorite places** area provides persistent
  shortcuts.
- Choose **Thumbnails** for a grid or **Filmstrip** for the selected image plus a
  centered thumbnail strip. Click an image to select it; double-click to open the
  separate viewer window.
- In the filmstrip image and separate viewer, scroll the mouse wheel or press
  **Up/Down** to zoom, and drag with the mouse to pan. Press **0** to reset to fit
  window or **1** for 100% (actual-pixel) zoom. Filmstrip mode shows the current
  zoom level in the top toolbar.
- The metadata pane exposes prompt, model, LoRA, workflow, and sampling data.
  Its copy buttons place their values on the system clipboard.
- **Recreate thumbnails** performs work in the background. A progress indicator is
  displayed while thumbnails are created or updated.

## Settings and favorite places

comruvi creates a readable TOML configuration at
`~/.config/comruvi/config.toml` (on Windows, `%USERPROFILE%\.config\comruvi\config.toml`).
It records `version = 0.02`, the last opened folder, and up to ten favorites as
`quick0` through `quick9`; unused quick keys are omitted. It also writes the
reserved future settings `thumbnailsize`, `fonttype`, `fontsize`, and
`displaymode`. The lower part of the Folders pane provides controls to add,
select, and remove favorites.

## Metadata

For PNG files comruvi reads `tEXt` and uncompressed `iTXt` chunks without modifying
the image. It recognizes ComfyUI's `prompt` and `workflow` chunks, extracts common
nodes (checkpoint/model, prompts, LoRA names/weights, sampler, seed, steps, CFG,
and scheduler), and always exposes the raw metadata for copying. Compressed PNG
text (`zTXt`) and JPEG EXIF/XMP parsing are planned; JPEGs currently show basic
image details.

## Credits

No code has been copied from other viewers. The thumbnail layout and filename
convention are intentionally compatible with [Geeqie](https://github.com/BestImageViewer/geeqie).

## Current status

**Alpha 2 (0.02).** Implemented: folder tree, resizable/toggleable panes,
persistent favorites, thumbnail grid, filmstrip, background thumbnail generation,
ComfyUI-oriented metadata (including rgthree Power Lora Loader), and a
synchronized secondary viewer window with zoom/pan. See
[CHANGELOG.md](CHANGELOG.md) for the detailed history.
