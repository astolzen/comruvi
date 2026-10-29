# Changelog

All notable changes to comruvi (ComfyUI Rust Viewer) are documented here.

## 0.091-alpha — 2026-10-29

### Added

- Separate viewer window title shows the current image file name.

### Fixed

- Camera EXIF display: aperture as `f/5.6`, shutter as `1/400`, focal length as
  `200 mm` (Canon/Samsung and other phones no longer show raw fractions like
  `28/5` or `77339/100000000`).

### Changed

- Config writes `version = 0.091`.

## 0.09-alpha (Alpha 9) — 2026-10-28

### Added

- Per-LoRA **Filter** button next to each active LoRA in the info pane; filters the
  folder to images where that LoRA is active (combinable with full text).

### Changed

- Config writes `version = 0.09`.

### Fixed

- Switching folders while thumbnail/metadata background jobs were running could
  leave the image list empty until restart; jobs are cancelled on folder change,
  and folder-watch refreshes are suppressed while workers run.

## 0.08-alpha (Alpha 8) — 2026-10-28

### Added

- Background `.metadata/<filename>.json` sidecars (same lifecycle as thumbnails):
  auto-fill for missing/stale entries, toolbar progress, and **Recreate → Metadata**.
- Toolbar **Recreate** menu replaces the old “Recreate thumbnails” button
  (options: Thumbnails, Metadata).
- Toolbar **Filter** menu: **reset**, **full text** phrase search (case-insensitive,
  contiguous match including spaces) against `.metadata` sidecars; button highlights
  like Thumbnails/Filmstrip while a filter is active.

### Changed

- Config writes `version = 0.08`.

## 0.07-alpha (Alpha 7) — 2026-10-28

### Added

- Open via file association / CLI path: show the image's folder in Thumbnails,
  open the image in the separate viewer; if comruvi is already running, hand the
  path to that instance (single-instance).
- `comruvi.ico` embedded in the PE (and Start Menu shortcuts) so pinning to the
  taskbar shows the comruvi icon.
- Repo-root `VERSION` file as single source of truth (About, config.toml,
  MSI/zip); `build.rs` syncs `Cargo.toml`.

### Changed

- Config writes `version = 0.07`.

## 0.06-alpha (Alpha 6) — 2026-10-28

### Added

- Windows MSI installer warning dialog (Accept / Decline) for early-alpha
  disclaimer with app icon and a link to the GitHub repository.
- First published Windows MSI release artifact.

### Changed

- Windows release binary uses the GUI subsystem so no console window appears
  when launching `comruvi.exe`.
- Config writes `version = 0.06`.
- README: **How to Install** (Windows MSI; macOS/Linux via `cargo run --release`).

## 0.05-alpha (Alpha 5) — 2026-10-21

### Added

- **Shift+drag** a thumbnail, filmstrip image, or viewer image into an open
  ComfyUI browser window to open a new tab with the embedded workflow (same as
  dropping from Explorer/Digikam).
- **Home** / **End** jump to the first or last image in Thumbnails and Filmstrip.
- The current folder is watched so new or removed images update the view while
  comruvi is running (`.thumbnails` writes are ignored).
- Application window/taskbar icon from `cv-ivon.svg`.
- Windows install packages: MSI (`installer/build-msi.ps1` →
  `dist/comruvi-<version>-windows-x64.msi`) and zip/script
  (`installer/pack-windows.ps1`), installing the binary under
  `%LOCALAPPDATA%\comruvi\` and default config/parsers under
  `%USERPROFILE%\.config\comruvi\` with Overwrite/Keep conflict handling.

### Changed

- Project license set to **AGPL-3.0** (`LICENSE`, `Cargo.toml`, README).
- README includes an application screenshot and notes successful builds on
  **Fedora Linux 44** as well as Windows.
- Config writes `version = 0.05`.
- Folder tree and toolbar use painted vector icons (chevrons, up arrow, refresh,
  folder, …) so controls stay readable without relying on Unicode font glyphs.

### Fixed

- Thumbnail view no longer keeps snapping the scroll position back to the
  selected image while using the mouse wheel; only arrow / Home / End keys
  scroll the selection into view.

## 0.04-alpha (Alpha 4) — 2026-10-19

### Fixed

- Folder tree: selecting a folder is separate from expand/collapse, so after
  **↑ Up** you can click back into a subfolder. Ancestors of the current folder
  stay open.
- Folder tree mouse-wheel scroll no longer steals events for the favorites list;
  the tree has its own scroll region and an always-visible scrollbar.
- Thumbnail creation stays responsive: at most a few thumb→texture loads run per
  frame (creation itself was already on a worker thread).
- JPEG **EXIF Orientation** is applied in filmstrip, full viewer, and new
  thumbnails so portrait photos are not shown as landscape. Use
  **↻ Recreate thumbnails** once for older thumbnail caches.

### Changed

- Config writes `version = 0.04`.

## 0.03-alpha (Alpha 3) — 2026-10-19

### Added

- Modular, human-readable **metadata translator** files (TOML) that decide how
  the right-hand info pane is filled. Each file has a trigger and a display-field
  list.
- Default translators shipped in `parsers/` and installed on first run into
  `~/.config/comruvi/parsers` (overridable via `parserdir` in config.toml):
  - `krea-ai` — Krea2 rebalance workflows (Power Lora / LoraManager)
  - `anima-ai` — Positive/Negative CLIP title workflows
  - `z-image` — Z-Image emoji prompt titles + Power Lora
  - `flux2` — rgthree Lora Loader Stack workflows
  - `comfyui-generic` — fallback ComfyUI prompt graph
  - `photo` — JPEG EXIF camera metadata
- JPEG EXIF extraction (camera, lens, shutter, aperture, ISO, focal length, date).
- Info pane shows which translator matched (`Parser: …`).

### Changed

- Right-pane fields are no longer hardcoded in Rust; they come from the matched
  translator, in file order.
- Config writes `version = 0.03` and `parserdir`.
- Generic ComfyUI fallback no longer shows a redundant `Generator: ComfyUI` row
  (`Type` already identifies the image).

### Documentation

- Parser/translator TOML syntax is documented in [parsers/README.md](parsers/README.md).

### Known limitations

- Compressed PNG `zTXt` / `iTXt` is not yet parsed.
- An abandoned thumbnail worker may finish the image it was already processing
  before it exits.

## 0.02-alpha (Alpha 2) — 2026-10-17

### Fixed

- Switching folders while thumbnails are generating now abandons the previous
  job and starts one for the new folder, so intermediate folders are not left
  without an indicator or generation pass.
- LoRA extraction now reads rgthree **Power Lora Loader** slots (`lora_1`,
  `lora_2`, … with on/strength), so Krea2-style workflows show their LoRAs.

### Known limitations

- JPEG EXIF/XMP and compressed PNG `zTXt` metadata are not yet parsed.
- An abandoned thumbnail worker may finish the image it was already processing
  before it exits.

## 0.01-alpha (Alpha 1) — 2026-10-17

### Added

- Initial Rust/egui image viewer for PNG, JPG, and JPEG.
- Folder tree, thumbnail grid, filmstrip, and synchronized secondary viewer.
- Compatible `.thumbnails/<original filename>.png` thumbnail storage.
- Background thumbnail creation with a progress indicator.
- Persistent TOML configuration at `~/.config/comruvi/config.toml`, including
  last folder, up to ten favorite locations, and future-facing display settings.
- ComfyUI PNG `prompt`/`workflow` metadata extraction, field copy controls, raw
  workflow copy, model details, and sampling parameters.
- Standard and LoraManager LoRA extraction, with inactive LoRAs shown in gray.
- Independent zoom/pan controls in filmstrip and secondary viewer: wheel or
  Up/Down to zoom, drag to pan, `0` to fit, and `1` for 100% zoom.
- Filmstrip mode shows the current zoom level (`Fit` or percent) in the top
  toolbar.

### Changed

- Project renamed from rustview to **comruvi** (ComfyUI Rust Viewer); config path
  is now `~/.config/comruvi/config.toml`.
- Metadata panel order is now stable: file, dimensions, model, LoRAs, prompts,
  workflow copy, then expandable other parameters.
- Positive and negative prompts are resolved from KSampler links instead of JSON
  node order. Identical embedded positive/negative values are explicitly marked.
- The active folder is highlighted in both the tree and favorites list; **↑ Up**
  navigates to the parent folder.
- Filmstrip drag-to-pan no longer snaps back after mouse movement stops.

### Known limitations

- JPEG EXIF/XMP and compressed PNG `zTXt` metadata are not yet parsed.
- Thumbnail generation is not yet cancelable.
