use std::{collections::BTreeMap, fs, io::Read, path::{Path, PathBuf}, sync::mpsc::{self, Receiver}, time::Duration};

use anyhow::{Context, Result};
use arboard::Clipboard;
use eframe::egui::{self, ColorImage, Context as EguiContext, Image, ScrollArea, TextureHandle, TextureOptions, Vec2};
use image::imageops::FilterType;
use serde_json::Value;

const THUMB_SIZE: u32 = 160;

fn main() -> eframe::Result<()> {
    let folder = std::env::args().nth(1).map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok()).unwrap_or_default();
    eframe::run_native("comruvi", eframe::NativeOptions::default(), Box::new(move |_| Ok(Box::new(App::new(folder)))))
}

#[derive(Clone)]
struct Entry { path: PathBuf, name: String }

struct App {
    folder: PathBuf, tree_root: PathBuf, entries: Vec<Entry>, selected: Option<usize>, mode: Mode,
    show_tree: bool, show_info: bool, viewer_open: bool, thumb_textures: BTreeMap<PathBuf, TextureHandle>,
    image_texture: Option<TextureHandle>, metadata: Metadata, notice: Option<String>, config: Config,
    thumbnail_job: Option<ThumbnailJob>,
    film_view: ViewTransform, viewer_view: ViewTransform,
}
struct ThumbnailJob { receiver: Receiver<bool>, completed: usize, total: usize }
#[derive(Default)]
struct ViewTransform { scale: Option<f32>, pan: Vec2 }
impl ViewTransform { fn reset(&mut self) { self.scale = None; self.pan = Vec2::ZERO; } }
#[derive(Clone, Copy, PartialEq)] enum Mode { Thumbnails, Filmstrip }

/// The deliberately small, dependency-free TOML representation keeps preferences
/// portable and readable at `~/.config/comruvi/config.toml`.
struct Config {
    favorites: Vec<PathBuf>, lastfolder: Option<PathBuf>,
    thumbnailsize: String, fonttype: String, fontsize: u32, displaymode: String,
}
impl Config {
    fn defaults() -> Self { Self { favorites: vec![], lastfolder: None, thumbnailsize: "160x160".into(), fonttype: "default".into(), fontsize: 14, displaymode: "dark".into() } }
    fn path() -> PathBuf {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
            .join(".config").join("comruvi").join("config.toml")
    }
    fn load() -> Self {
        let mut config = Self::defaults();
        let Ok(contents) = fs::read_to_string(Self::path()) else { return config; };
        for line in contents.lines() {
            let Some((key, raw_value)) = line.split_once('=') else { continue; };
            let key = key.trim(); let value = raw_value.trim().trim_matches('"').replace("\\\\", "\\").replace("\\\"", "\"");
            match key {
                "lastfolder" => if !value.is_empty() { config.lastfolder = Some(PathBuf::from(value)); },
                "thumbnailsize" => config.thumbnailsize = value,
                "fonttype" => config.fonttype = value,
                "fontsize" => if let Ok(size) = value.parse() { config.fontsize = size; },
                "displaymode" => config.displaymode = value,
                _ if key.starts_with("quick") && key[5..].parse::<usize>().is_ok_and(|index| index < 10) && !value.is_empty() => config.favorites.push(PathBuf::from(value)),
                _ => {}
            }
        }
        config.favorites.truncate(10);
        config
    }
    fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        let quote = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
        let mut contents = format!("version = 0.01\nthumbnailsize = \"{}\"\nfonttype = \"{}\"\nfontsize = {}\ndisplaymode = \"{}\"\n", quote(&self.thumbnailsize), quote(&self.fonttype), self.fontsize, quote(&self.displaymode));
        if let Some(folder) = &self.lastfolder { contents.push_str(&format!("lastfolder = \"{}\"\n", quote(&folder.to_string_lossy()))); }
        for (index, favorite) in self.favorites.iter().take(10).enumerate() { contents.push_str(&format!("quick{index} = \"{}\"\n", quote(&favorite.to_string_lossy()))); }
        fs::write(path, contents)?;
        Ok(())
    }
}

impl App {
    fn new(folder: PathBuf) -> Self {
        let config = Config::load();
        let folder = config.lastfolder.as_ref().filter(|path| path.is_dir()).cloned().unwrap_or(folder);
        let mut app = Self { tree_root: folder.clone(), folder, entries: vec![], selected: None, mode: Mode::Thumbnails, show_tree: true, show_info: true, viewer_open: false, thumb_textures: BTreeMap::new(), image_texture: None, metadata: Metadata::default(), notice: None, config, thumbnail_job: None, film_view: ViewTransform::default(), viewer_view: ViewTransform::default() };
        app.load_folder();
        // Create/update the config on first launch as well, so every documented
        // placeholder has a stable default value.
        app.remember_folder();
        app
    }
    fn load_folder(&mut self) {
        self.entries.clear(); self.selected = None; self.image_texture = None; self.thumb_textures.clear(); self.metadata = Metadata::default();
        if let Ok(read) = fs::read_dir(&self.folder) {
            let mut files: Vec<_> = read.flatten().filter_map(|x| {
                let p = x.path(); is_image(&p).then(|| Entry { name: x.file_name().to_string_lossy().into_owned(), path: p })
            }).collect();
            files.sort_by_key(|x| x.name.to_lowercase()); self.entries = files;
        }
        self.queue_missing_thumbnails();
    }
    fn select(&mut self, index: usize, ctx: &EguiContext) {
        if self.selected == Some(index) { return; }
        self.selected = Some(index); self.image_texture = None;
        self.film_view.reset(); self.viewer_view.reset();
        let path = self.entries[index].path.clone(); self.metadata = read_metadata(&path).unwrap_or_else(|e| Metadata::error(e.to_string()));
        if let Ok(image) = load_image(&path) { self.image_texture = Some(ctx.load_texture("full-image", image, TextureOptions::default())); }
    }
    fn thumbnail(&mut self, entry: &Entry, ctx: &EguiContext) -> Option<TextureHandle> {
        if let Some(t) = self.thumb_textures.get(&entry.path) { return Some(t.clone()); }
        let thumb = thumbnail_path(&entry.path);
        // Missing thumbnails are generated by the worker, not on the UI thread.
        if !thumb.exists() { return None; }
        let image = load_image(&thumb).ok()?;
        let texture = ctx.load_texture(format!("thumb-{}", entry.path.display()), image, TextureOptions::default());
        self.thumb_textures.insert(entry.path.clone(), texture.clone()); Some(texture)
    }
    fn recreate_thumbnails(&mut self) {
        if self.thumbnail_job.is_some() { return; }
        self.start_thumbnail_job(self.entries.iter().map(|entry| entry.path.clone()).collect());
    }
    fn queue_missing_thumbnails(&mut self) {
        if self.thumbnail_job.is_some() { return; }
        let paths: Vec<_> = self.entries.iter().filter(|entry| !thumbnail_path(&entry.path).exists()).map(|entry| entry.path.clone()).collect();
        if !paths.is_empty() { self.start_thumbnail_job(paths); }
    }
    fn start_thumbnail_job(&mut self, paths: Vec<PathBuf>) {
        let total = paths.len();
        if total == 0 { return; }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for path in paths {
                let succeeded = create_thumbnail(&path, &thumbnail_path(&path)).is_ok();
                if sender.send(succeeded).is_err() { break; }
            }
        });
        self.thumb_textures.clear();
        self.notice = Some(format!("Creating/updating thumbnails (0/{total})"));
        self.thumbnail_job = Some(ThumbnailJob { receiver, completed: 0, total });
    }
    fn navigate(&mut self, direction: isize, ctx: &EguiContext) {
        if self.entries.is_empty() { return; }
        let current = self.selected.unwrap_or(0) as isize;
        let last = self.entries.len() as isize - 1;
        let target = (current + direction).clamp(0, last) as usize;
        if self.selected != Some(target) { self.select(target, ctx); }
    }
    fn remember_folder(&mut self) {
        self.config.lastfolder = Some(self.folder.clone());
        if let Err(error) = self.config.save() { self.notice = Some(format!("Could not save config: {error}")); }
    }
    fn add_favorite(&mut self, path: PathBuf) {
        if self.config.favorites.iter().any(|favorite| favorite == &path) { self.notice = Some("That folder is already a favorite.".into()); return; }
        if self.config.favorites.len() == 10 { self.notice = Some("Favorites are limited to 10 folders.".into()); return; }
        self.config.favorites.push(path);
        if let Err(error) = self.config.save() { self.notice = Some(format!("Could not save config: {error}")); }
    }
    fn go_up(&mut self) {
        let Some(parent) = self.folder.parent().map(Path::to_path_buf) else { return; };
        if !parent.starts_with(&self.tree_root) { self.tree_root = parent.clone(); }
        self.folder = parent;
        self.load_folder();
        self.remember_folder();
    }
    fn poll_thumbnail_job(&mut self) {
        let Some(job) = &mut self.thumbnail_job else { return; };
        while job.receiver.try_recv().is_ok() { job.completed += 1; }
        if job.completed >= job.total {
            self.notice = Some(format!("Thumbnail update finished ({} images).", job.total));
            self.thumbnail_job = None;
            self.queue_missing_thumbnails();
        } else {
            self.notice = Some(format!("Creating/updating thumbnails ({}/{})", job.completed, job.total));
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &EguiContext, _: &mut eframe::Frame) {
        self.poll_thumbnail_job();
        if self.thumbnail_job.is_some() { ctx.request_repaint_after(Duration::from_millis(100)); }
        // Navigation remains available in either main-view mode and updates an
        // already-open secondary viewer through the shared selected image.
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft)) { self.navigate(-1, ctx); }
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowRight)) { self.navigate(1, ctx); }
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open folder").clicked() { if let Some(p) = rfd::FileDialog::new().pick_folder() { self.tree_root = p.clone(); self.folder = p; self.load_folder(); self.remember_folder(); } }
                if ui.button("↑ Up").clicked() { self.go_up(); }
                if ui.add_enabled(self.thumbnail_job.is_none(), egui::Button::new("↻ Recreate thumbnails")).clicked() { self.recreate_thumbnails(); }
                ui.separator(); ui.checkbox(&mut self.show_tree, "Folders"); ui.checkbox(&mut self.show_info, "Info"); ui.separator();
                ui.selectable_value(&mut self.mode, Mode::Thumbnails, "Thumbnails"); ui.selectable_value(&mut self.mode, Mode::Filmstrip, "Filmstrip");
                if self.mode == Mode::Filmstrip {
                    let zoom = match self.film_view.scale {
                        Some(scale) => format!("Zoom: {:.0}%", scale * 100.0),
                        None => "Zoom: Fit".into(),
                    };
                    ui.separator();
                    ui.label(zoom);
                }
                if self.thumbnail_job.is_some() { ui.add(egui::Spinner::new()); }
                if let Some(n) = &self.notice { ui.label(n); }
            });
        });
        if self.show_tree {
            let root = self.tree_root.clone();
            let mut changed = false;
            egui::SidePanel::left("folders").resizable(true).default_width(210.0).show(ctx, |ui| {
                ui.heading("Folders");
                let tree_height = (ui.available_height() - 155.0).max(80.0);
                ScrollArea::vertical().max_height(tree_height).show(ui, |ui| { folder_tree(ui, &root, &mut self.folder, &mut changed); });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Favorite places").strong());
                    if ui.small_button("+ Add…").clicked() { if let Some(path) = rfd::FileDialog::new().pick_folder() { self.add_favorite(path); } }
                });
                ScrollArea::vertical().show(ui, |ui| {
                    let mut remove = None;
                    let favorites = self.config.favorites.clone();
                    for (index, favorite) in favorites.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let name = favorite.file_name().unwrap_or_else(|| favorite.as_os_str()).to_string_lossy();
                            if ui.selectable_label(self.folder == *favorite, name).clicked() { self.tree_root = favorite.clone(); self.folder = favorite.clone(); changed = true; }
                            if ui.small_button("×").on_hover_text("Remove favorite").clicked() { remove = Some(index); }
                        });
                    }
                    if let Some(index) = remove { self.config.favorites.remove(index); if let Err(error) = self.config.save() { self.notice = Some(format!("Could not save config: {error}")); } }
                });
            });
            if changed { self.load_folder(); self.remember_folder(); }
        }
        if self.show_info { egui::SidePanel::right("metadata").resizable(true).default_width(330.0).show(ctx, |ui| { metadata_ui(ui, &self.metadata); }); }
        egui::CentralPanel::default().show(ctx, |ui| match self.mode { Mode::Thumbnails => thumbnails_ui(ui, self, ctx), Mode::Filmstrip => filmstrip_ui(ui, self, ctx) });
        if self.viewer_open { viewer_ui(ctx, self); }
    }
}

fn folder_tree(ui: &mut egui::Ui, dir: &Path, selected: &mut PathBuf, changed: &mut bool) {
    let name = dir.file_name().unwrap_or_else(|| dir.as_os_str()).to_string_lossy();
    let is_selected = dir == selected;
    let mut title = egui::RichText::new(name.to_string());
    if is_selected { title = title.background_color(ui.visuals().selection.bg_fill).color(ui.visuals().selection.stroke.color); }
    let response = egui::CollapsingHeader::new(title).default_open(is_selected).show(ui, |ui| {
        let mut children: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().filter(|x| x.path().is_dir() && x.file_name() != ".thumbnails").collect();
        children.sort_by_key(|x| x.file_name());
        for child in children { folder_tree(ui, &child.path(), selected, changed); }
    });
    if response.header_response.clicked() { *selected = dir.to_path_buf(); *changed = true; }
}

fn thumbnails_ui(ui: &mut egui::Ui, app: &mut App, ctx: &EguiContext) {
    ui.heading(app.folder.display().to_string()); ui.label(format!("{} image(s) · double-click an image to open the separate viewer", app.entries.len()));
    let width = ui.available_width(); let columns = (width / 184.0).max(1.0) as usize;
    ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("thumb-grid").spacing([18.0, 14.0]).show(ui, |ui| {
            for i in 0..app.entries.len() { let entry = app.entries[i].clone(); let texture = app.thumbnail(&entry, ctx); ui.vertical(|ui| { let r = if let Some(t) = texture { ui.add(egui::ImageButton::new(Image::new(&t).fit_to_exact_size(Vec2::splat(160.0))).selected(app.selected == Some(i))) } else { ui.button("Unreadable") }; if r.clicked() { app.select(i, ctx); } if r.double_clicked() { app.viewer_open = true; } ui.label(egui::RichText::new(entry.name).small()); }); if (i + 1) % columns == 0 { ui.end_row(); } }
        });
    });
}

fn filmstrip_ui(ui: &mut egui::Ui, app: &mut App, ctx: &EguiContext) {
    if app.selected.is_none() && !app.entries.is_empty() { app.select(0, ctx); }
    let image_height = (ui.available_height() - 195.0).max(100.0);
    if let Some(t) = &app.image_texture { zoomable_image(ui, t, &mut app.film_view, Vec2::new(ui.available_width(), image_height)); }
    else { ui.allocate_ui(Vec2::new(ui.available_width(), image_height), |ui| { ui.centered_and_justified(|ui| { ui.label("Select an image"); }); }); }
    ui.separator(); ScrollArea::horizontal().show(ui, |ui| { ui.horizontal(|ui| { for i in 0..app.entries.len() { let e = app.entries[i].clone(); if let Some(t) = app.thumbnail(&e, ctx) { let r = ui.add(egui::ImageButton::new(Image::new(&t).fit_to_exact_size(Vec2::splat(110.0))).selected(app.selected == Some(i))); if r.clicked() { app.select(i, ctx); } if r.double_clicked() { app.viewer_open = true; } if app.selected == Some(i) { r.scroll_to_me(Some(egui::Align::Center)); } } } }); });
}

fn viewer_ui(ctx: &EguiContext, app: &mut App) {
    let texture = app.image_texture.clone();
    let mut close_requested = false;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("separate-viewer"),
        egui::ViewportBuilder::default().with_title("comruvi — viewer").with_inner_size([1000.0, 750.0]),
        |viewer_ctx, _| {
            close_requested = viewer_ctx.input(|input| input.viewport().close_requested());
            egui::CentralPanel::default().show(viewer_ctx, |ui| {
                if let Some(t) = &texture { zoomable_image(ui, t, &mut app.viewer_view, ui.available_size()); }
                else { ui.label("Select an image in the main window."); }
            });
        },
    );
    if close_requested { app.viewer_open = false; }
}

/// Draw an image into a clipped viewport with independent zoom and pan state.
/// `None` scale means "fit viewport"; `Some(1.0)` means original pixels.
fn zoomable_image(ui: &mut egui::Ui, texture: &TextureHandle, view: &mut ViewTransform, desired_size: Vec2) {
    let response = ui.allocate_response(desired_size, egui::Sense::drag());
    let source = texture.size_vec2();
    let fit = (response.rect.width() / source.x).min(response.rect.height() / source.y);
    if ui.input(|input| input.key_pressed(egui::Key::Num0)) { view.reset(); }
    if ui.input(|input| input.key_pressed(egui::Key::Num1)) { view.scale = Some(1.0); view.pan = Vec2::ZERO; }
    let zoom_in = ui.input(|input| input.key_pressed(egui::Key::ArrowUp));
    let zoom_out = ui.input(|input| input.key_pressed(egui::Key::ArrowDown));
    let wheel = if response.hovered() { ui.input(|input| input.raw_scroll_delta.y) } else { 0.0 };
    if zoom_in || wheel > 0.0 { view.scale = Some(view.scale.unwrap_or(fit) * 1.15); }
    if zoom_out || wheel < 0.0 { view.scale = Some(view.scale.unwrap_or(fit) / 1.15); }
    if let Some(scale) = view.scale { view.scale = Some(scale.clamp(0.02, 32.0)); }
    if response.dragged() { view.pan += response.drag_delta(); }
    let scale = view.scale.unwrap_or(fit);
    let image_rect = egui::Rect::from_center_size(response.rect.center() + view.pan, source * scale);
    ui.painter().with_clip_rect(response.rect).image(texture.id(), image_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
}

#[derive(Default)] struct Metadata { fields: Vec<(String, String)>, raw: Vec<(String, String)> }
impl Metadata { fn error(e: String) -> Self { Self { fields: vec![("Error".into(), e)], raw: vec![] } } }
fn metadata_ui(ui: &mut egui::Ui, m: &Metadata) {
    ui.heading("Image information");
    ScrollArea::vertical().show(ui, |ui| {
        // Keep this presentation order independent of a generator's JSON node order.
        for key in ["File", "Dimensions", "Model"] { for (field, value) in m.fields.iter().filter(|(field, _)| field == key) { metadata_field(ui, field, value); } }
        let loras: Vec<_> = m.fields.iter().filter(|(field, _)| field == "LoRA").collect();
        if !loras.is_empty() {
            ui.label(egui::RichText::new("LoRA list").strong());
            for (_, value) in loras {
                let inactive = value.contains("; inactive)");
                ui.horizontal_wrapped(|ui| {
                    if inactive { ui.colored_label(egui::Color32::GRAY, value); }
                    else { ui.label(value); }
                    if ui.small_button("Copy").clicked() { copy(value); }
                });
            }
            ui.separator();
        }
        for key in ["Text prompt", "Positive prompt", "Negative prompt"] {
            for (field, value) in m.fields.iter().filter(|(field, _)| field == key || (key == "Negative prompt" && field.starts_with("Negative prompt ("))) { metadata_field(ui, field, value); }
        }
        if let Some((_, workflow)) = m.raw.iter().find(|(key, _)| key.eq_ignore_ascii_case("workflow")) {
            if ui.button("Copy workflow").clicked() { copy(workflow); }
            ui.separator();
        }
        ui.collapsing("Other parameters", |ui| {
            for (field, value) in &m.fields {
                let primary = matches!(field.as_str(), "File" | "Dimensions" | "Model" | "LoRA" | "Text prompt" | "Positive prompt") || field.starts_with("Negative prompt");
                if !primary { metadata_field(ui, field, value); }
            }
        });
    });
}
fn metadata_field(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.label(egui::RichText::new(key).strong());
    ui.horizontal_wrapped(|ui| { ui.label(value); if ui.small_button("Copy").clicked() { copy(value); } });
    ui.separator();
}
fn copy(value: &str) { if let Ok(mut clipboard) = Clipboard::new() { let _ = clipboard.set_text(value); } }

fn is_image(p: &Path) -> bool { matches!(p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg")) }
fn thumbnail_path(image: &Path) -> PathBuf { image.parent().unwrap_or(Path::new(".")).join(".thumbnails").join(format!("{}.png", image.file_name().unwrap_or_default().to_string_lossy())) }
fn create_thumbnail(source: &Path, destination: &Path) -> Result<()> { let img = image::open(source).with_context(|| format!("opening {}", source.display()))?; if let Some(p) = destination.parent() { fs::create_dir_all(p)?; } img.resize(THUMB_SIZE, THUMB_SIZE, FilterType::Lanczos3).save_with_format(destination, image::ImageFormat::Png)?; Ok(()) }
fn load_image(path: &Path) -> Result<ColorImage> { let img = image::open(path)?.to_rgba8(); let size = [img.width() as usize, img.height() as usize]; Ok(ColorImage::from_rgba_unmultiplied(size, img.as_raw())) }

fn read_metadata(path: &Path) -> Result<Metadata> {
    let image = image::open(path)?; let mut m = Metadata { fields: vec![("File".into(), path.file_name().unwrap_or_default().to_string_lossy().into()), ("Dimensions".into(), format!("{} × {}", image.width(), image.height())), ("Format".into(), path.extension().and_then(|x| x.to_str()).unwrap_or("unknown").to_ascii_uppercase())], raw: vec![] };
    if path.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("png")) { let chunks = png_text_chunks(path)?; for (key, value) in &chunks { m.raw.push((key.clone(), value.clone())); } if let Some((_, prompt)) = chunks.iter().find(|(k, _)| k == "prompt") { comfy_fields(prompt, &mut m.fields); } }
    Ok(m)
}
fn png_text_chunks(path: &Path) -> Result<Vec<(String, String)>> {
    let mut bytes = vec![];
    fs::File::open(path)?.read_to_end(&mut bytes)?;
    let mut result = vec![];
    let mut pos = 8usize; // PNG signature
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into()?) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        if pos + 12 + len > bytes.len() { break; }
        let data = &bytes[pos + 8..pos + 8 + len];
        if kind == b"tEXt" {
            if let Some(n) = data.iter().position(|x| *x == 0) {
                result.push((String::from_utf8_lossy(&data[..n]).into_owned(), String::from_utf8_lossy(&data[n + 1..]).into_owned()));
            }
        } else if kind == b"iTXt" {
            // keyword, compression flag, method, language, translated keyword, text
            let mut parts = data.splitn(6, |x| *x == 0);
            let key = parts.next().unwrap_or_default();
            let flag = parts.next().unwrap_or_default();
            let _method = parts.next();
            let _language = parts.next();
            let _translated = parts.next();
            let text = parts.next().unwrap_or_default();
            if flag == [0] { result.push((String::from_utf8_lossy(key).into_owned(), String::from_utf8_lossy(text).into_owned())); }
        }
        pos += 12 + len; // data + type/length/crc
    }
    Ok(result)
}
fn comfy_fields(prompt: &str, fields: &mut Vec<(String, String)>) {
    let Ok(nodes) = serde_json::from_str::<Value>(prompt) else { fields.push(("ComfyUI prompt".into(), prompt.into())); return; };
    fields.push(("Generator".into(), "ComfyUI".into()));
    let Some(map) = nodes.as_object() else { return; };

    // A workflow's JSON order is not semantic. The KSampler links tell us which
    // CLIPTextEncode node is positive and which is negative.
    let mut positive_nodes = Vec::new();
    let mut negative_nodes = Vec::new();
    for node in map.values() {
        let class = node.get("class_type").and_then(Value::as_str).unwrap_or("");
        let Some(inputs) = node.get("inputs").and_then(Value::as_object) else { continue; };
        if matches!(class, "KSampler" | "KSamplerAdvanced") {
            if let Some(id) = linked_node(inputs, "positive") { positive_nodes.push(id); }
            if let Some(id) = linked_node(inputs, "negative") { negative_nodes.push(id); }
        }
    }

    for (id, node) in map {
        let class = node.get("class_type").and_then(Value::as_str).unwrap_or("");
        let Some(inputs) = node.get("inputs").and_then(Value::as_object) else { continue; };
        match class {
            "CheckpointLoaderSimple" | "UNETLoader" => {
                if let Some(v) = input_text(inputs, "ckpt_name").or_else(|| input_text(inputs, "unet_name")) { fields.push(("Model".into(), v)); }
            }
            "CLIPTextEncode" => if let Some(v) = input_text(inputs, "text") {
                let label = if positive_nodes.iter().any(|node_id| node_id == id) { "Positive prompt" }
                    else if negative_nodes.iter().any(|node_id| node_id == id) { "Negative prompt" }
                    else { "Text prompt" };
                fields.push((label.into(), v));
            },
            "KSampler" | "KSamplerAdvanced" => for key in ["seed", "steps", "cfg", "sampler_name", "scheduler", "denoise"] {
                if let Some(v) = input_text(inputs, key) { fields.push((key.replace('_', " "), v)); }
            },
            _ if class.to_ascii_lowercase().contains("lora") => add_loras(inputs, fields),
            _ => {}
        }
    }
    // Some workflows genuinely save the same conditioning text on both sampler
    // inputs. Make that provenance visible instead of implying it is a separate
    // negative prompt entered by the user.
    let positive = fields.iter().find(|(key, _)| key == "Positive prompt").map(|(_, value)| value.clone());
    if let Some(positive) = positive {
        for (key, value) in fields.iter_mut() {
            if key == "Negative prompt" && value == &positive { *key = "Negative prompt (same as positive in image metadata)".into(); }
        }
    }
}

fn input_text(inputs: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    inputs.get(key).map(|value| value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string()))
}

fn linked_node(inputs: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    let value = inputs.get(key)?.as_array()?.first()?;
    value.as_str().map(str::to_owned).or_else(|| value.as_u64().map(|id| id.to_string()))
}

fn add_loras(inputs: &serde_json::Map<String, Value>, fields: &mut Vec<(String, String)>) {
    if let Some(name) = input_text(inputs, "lora_name") {
        let model_weight = input_text(inputs, "strength_model").unwrap_or_else(|| "?".into());
        let clip_weight = input_text(inputs, "strength_clip");
        let weights = clip_weight.map_or_else(|| format!("weight {model_weight}"), |clip| format!("model {model_weight}, CLIP {clip}"));
        fields.push(("LoRA".into(), format!("{name} ({weights})")));
    }
    // LoraManager has used both a direct list and a `__value__` wrapped list.
    // Preserve each listed strength and active state for either representation.
    let Some(loras) = inputs.get("loras") else { return; };
    let entries = loras.as_array().or_else(|| loras.get("__value__").and_then(Value::as_array));
    let Some(entries) = entries else { return; };
    for lora in entries {
        let Some(name) = lora.get("name").and_then(Value::as_str) else { continue; };
        let strength = lora.get("strength").map(Value::to_string).unwrap_or_else(|| "?".into()).trim_matches('"').to_owned();
        let clip = lora.get("clipStrength").map(Value::to_string).unwrap_or_else(|| strength.clone()).trim_matches('"').to_owned();
        let active = lora.get("active").and_then(Value::as_bool).unwrap_or(true);
        let state = if active { "active" } else { "inactive" };
        fields.push(("LoRA".into(), format!("{name} (model {strength}, CLIP {clip}; {state})")));
    }
}
