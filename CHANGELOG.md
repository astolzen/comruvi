# Changelog

All notable changes to Rustview are documented here.

## 0.01-alpha — 2026-10-17

### Added

- Initial Rust/egui image viewer for PNG, JPG, and JPEG.
- Folder tree, thumbnail grid, filmstrip, and synchronized secondary viewer.
- Compatible `.thumbnails/<original filename>.png` thumbnail storage.
- Background thumbnail creation with a progress indicator.
- Persistent TOML configuration at `~/.config/rustview/config.toml`, including
  last folder, up to ten favorite locations, and future-facing display settings.
- ComfyUI PNG `prompt`/`workflow` metadata extraction, field copy controls, raw
  workflow copy, model details, and sampling parameters.
- Standard and LoraManager LoRA extraction, with inactive LoRAs shown in gray.
- Independent zoom/pan controls in filmstrip and secondary viewer: wheel or
  Up/Down to zoom, drag to pan, `0` to fit, and `1` for 100% zoom.

### Changed

- Metadata panel order is now stable: file, dimensions, model, LoRAs, prompts,
  workflow copy, then expandable other parameters.
- Positive and negative prompts are resolved from KSampler links instead of JSON
  node order. Identical embedded positive/negative values are explicitly marked.
- The active folder is highlighted in both the tree and favorites list; **↑ Up**
  navigates to the parent folder.

### Known limitations

- JPEG EXIF/XMP and compressed PNG `zTXt` metadata are not yet parsed.
- Thumbnail generation is not yet cancelable.
