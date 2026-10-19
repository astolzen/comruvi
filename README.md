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

- The **Folders** pane is a tree with its own scrollbar. Click a name to open that
  folder; use ▸/▾ to expand or collapse. **↑ Up** moves to the parent. The lower
  **Favorite places** area provides persistent shortcuts.
- Choose **Thumbnails** for a grid or **Filmstrip** for the selected image plus a
  centered thumbnail strip. Click an image to select it; double-click to open the
  separate viewer window. **Left/Right** move the selection in both modes;
  **Up/Down** move by grid row in Thumbnails.
- In the filmstrip image and separate viewer, scroll the mouse wheel or press
  **Up/Down** to zoom, and drag with the mouse to pan. Press **0** to reset to fit
  window or **1** for 100% (actual-pixel) zoom. Filmstrip mode shows the current
  zoom level in the top toolbar.
- The metadata pane is driven by **translator** files: each image is matched to a
  parser config by a trigger, then the right pane shows the configured fields.
  Copy buttons place values on the system clipboard.
- **Recreate thumbnails** performs work in the background. A progress indicator is
  displayed while thumbnails are created or updated. JPEG EXIF orientation is
  applied for filmstrip, the separate viewer, and newly built thumbnails.

## Settings and favorite places

comruvi creates a readable TOML configuration at
`~/.config/comruvi/config.toml` (on Windows, `%USERPROFILE%\.config\comruvi\config.toml`).
It records `version = 0.04`, the last opened folder, `parserdir` (directory of
translator files), and up to ten favorites as `quick0` through `quick9`; unused
quick keys are omitted. It also writes the reserved future settings
`thumbnailsize`, `fonttype`, `fontsize`, and `displaymode`. The lower part of
the Folders pane provides controls to add, select, and remove favorites.

## Metadata translators

Display of the right-hand info pane is modular. Human-readable TOML **parser /
translator** files live in `parserdir` (default
`~/.config/comruvi/parsers`). On first run, comruvi copies the shipped defaults
from the repo's `parsers/` folder into that directory (existing files are never
overwritten).

Shipped defaults cover Krea2, Anima-style positive/negative workflows, Z-Image,
Flux2 lora stacks, a generic ComfyUI fallback, and digital-camera JPEGs (EXIF).
Add or edit TOML files in `parserdir` to teach comruvi new formats without
recompiling.

**Full syntax reference:** [parsers/README.md](parsers/README.md) — identity
keys, triggers, field sources (`constant` / `input` / `extract` / `exif`),
built-ins, and examples.

For PNG files comruvi reads `tEXt` and uncompressed `iTXt` chunks without modifying
the image. JPEG EXIF is read for camera metadata and for **Orientation** when
displaying or thumbnailing photos. Compressed PNG `zTXt` is not yet parsed.

## Credits

No code has been copied from other viewers. The thumbnail layout and filename
convention are intentionally compatible with [Geeqie](https://github.com/BestImageViewer/geeqie).

## Current status

**Alpha 4 (0.04).** Implemented: folder tree with independent scroll, persistent
favorites, thumbnail grid, filmstrip, background thumbnail generation, modular
metadata translators (ComfyUI + EXIF photo), JPEG orientation-aware display, and
a synchronized secondary viewer window with zoom/pan. See
[CHANGELOG.md](CHANGELOG.md) for the detailed history.
