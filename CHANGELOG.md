# Changelog

All notable changes to comruvi (ComfyUI Rust Viewer) are documented here.

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
