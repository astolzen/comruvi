mod meta_parsers;

use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, sync::mpsc::{self, Receiver}, time::Duration};

use anyhow::{Context, Result};
use arboard::Clipboard;
use eframe::egui::{self, ColorImage, Context as EguiContext, Image, ScrollArea, TextureHandle, TextureOptions, Vec2};
use image::imageops::FilterType;
use meta_parsers::{default_parser_dir, exif_orientation, load_parsers, read_metadata, Metadata, ParserConfig};

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
    parsers: Vec<ParserConfig>,
    /// Last laid-out thumbnail grid column count (for Up/Down row navigation).
    thumb_columns: usize,
    /// Cap synchronous thumb→texture loads per frame so the UI stays responsive.
    thumb_loads_left: usize,
    /// Cached subdirectory listings for the folder tree (path → children).
    folder_children: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Manual expand/collapse overrides for tree nodes (true=open, false=closed).
    folder_open: BTreeMap<PathBuf, bool>,
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
    parserdir: PathBuf,
}
impl Config {
    fn defaults() -> Self {
        Self {
            favorites: vec![], lastfolder: None, thumbnailsize: "160x160".into(), fonttype: "default".into(),
            fontsize: 14, displaymode: "dark".into(), parserdir: default_parser_dir(),
        }
    }
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
                "parserdir" => if !value.is_empty() { config.parserdir = PathBuf::from(value); },
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
        let mut contents = format!(
            "version = 0.04\nthumbnailsize = \"{}\"\nfonttype = \"{}\"\nfontsize = {}\ndisplaymode = \"{}\"\nparserdir = \"{}\"\n",
            quote(&self.thumbnailsize), quote(&self.fonttype), self.fontsize, quote(&self.displaymode),
            quote(&self.parserdir.to_string_lossy()),
        );
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
        let (parsers, parser_notice) = match load_parsers(&config.parserdir) {
            Ok(parsers) => (parsers, None),
            Err(error) => (Vec::new(), Some(format!("Could not load parsers: {error}"))),
        };
        let mut app = Self {
            tree_root: folder.clone(), folder, entries: vec![], selected: None, mode: Mode::Thumbnails,
            show_tree: true, show_info: true, viewer_open: false, thumb_textures: BTreeMap::new(),
            image_texture: None, metadata: Metadata::default(), notice: parser_notice, config, parsers,
            thumb_columns: 1, thumb_loads_left: 0, folder_children: BTreeMap::new(), folder_open: BTreeMap::new(),
            thumbnail_job: None, film_view: ViewTransform::default(), viewer_view: ViewTransform::default(),
        };
        app.load_folder();
        // Create/update the config on first launch as well, so every documented
        // placeholder has a stable default value.
        app.remember_folder();
        app
    }
    fn load_folder(&mut self) {
        // Drop any in-flight job for the previous folder so the worker exits on
        // the next send and the new folder can queue its own missing thumbs.
        // Without this, rapid folder switches leave intermediate folders unqueued
        // until the old job finishes — only the last folder gets a fresh job.
        self.thumbnail_job = None;
        self.folder_children.clear();
        self.entries.clear(); self.selected = None; self.image_texture = None; self.thumb_textures.clear(); self.metadata = Metadata::default();
        if let Ok(read) = fs::read_dir(&self.folder) {
            let mut files: Vec<_> = read.flatten().filter_map(|x| {
                let p = x.path(); is_image(&p).then(|| Entry { name: x.file_name().to_string_lossy().into_owned(), path: p })
            }).collect();
            files.sort_by_key(|x| x.name.to_lowercase()); self.entries = files;
        }
        self.queue_missing_thumbnails();
    }
    fn tree_children(&mut self, dir: &Path) -> Vec<PathBuf> {
        if let Some(children) = self.folder_children.get(dir) { return children.clone(); }
        let mut children: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.file_name().is_some_and(|name| name != ".thumbnails"))
            .collect();
        children.sort_by(|a, b| {
            let an = a.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            let bn = b.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            an.cmp(&bn)
        });
        self.folder_children.insert(dir.to_path_buf(), children.clone());
        children
    }
    fn select(&mut self, index: usize, ctx: &EguiContext) {
        if self.selected == Some(index) { return; }
        self.selected = Some(index); self.image_texture = None;
        self.film_view.reset(); self.viewer_view.reset();
        let path = self.entries[index].path.clone();
        self.metadata = read_metadata(&path, &self.parsers).unwrap_or_else(|e| Metadata::error(e.to_string()));
        if let Ok(image) = load_image(&path) { self.image_texture = Some(ctx.load_texture("full-image", image, TextureOptions::default())); }
    }
    fn thumbnail(&mut self, entry: &Entry, ctx: &EguiContext) -> Option<TextureHandle> {
        if let Some(t) = self.thumb_textures.get(&entry.path) { return Some(t.clone()); }
        let thumb = thumbnail_path(&entry.path);
        // Missing thumbnails are generated by the worker, not on the UI thread.
        if !thumb.exists() { return None; }
        // Decode at most a few thumbs per frame; otherwise a large folder freezes
        // the UI even though creation itself runs on a background thread.
        if self.thumb_loads_left == 0 {
            ctx.request_repaint();
            return None;
        }
        self.thumb_loads_left -= 1;
        let image = load_image(&thumb).ok()?;
        let texture = ctx.load_texture(format!("thumb-{}", entry.path.display()), image, TextureOptions::default());
        self.thumb_textures.insert(entry.path.clone(), texture.clone()); Some(texture)
    }
    fn recreate_thumbnails(&mut self) {
        if self.thumbnail_job.is_some() { return; }
        let paths: Vec<_> = self.entries.iter().map(|entry| entry.path.clone()).collect();
        self.start_thumbnail_job(paths, true);
    }
    fn queue_missing_thumbnails(&mut self) {
        if self.thumbnail_job.is_some() { return; }
        let paths: Vec<_> = self.entries.iter().filter(|entry| !thumbnail_path(&entry.path).exists()).map(|entry| entry.path.clone()).collect();
        if !paths.is_empty() { self.start_thumbnail_job(paths, false); }
    }
    fn start_thumbnail_job(&mut self, paths: Vec<PathBuf>, clear_all_textures: bool) {
        let total = paths.len();
        if total == 0 { return; }
        let (sender, receiver) = mpsc::channel();
        let job_paths = paths.clone();
        std::thread::spawn(move || {
            for path in job_paths {
                let succeeded = create_thumbnail(&path, &thumbnail_path(&path)).is_ok();
                if sender.send(succeeded).is_err() { break; }
            }
        });
        if clear_all_textures {
            self.thumb_textures.clear();
        } else {
            for path in &paths { self.thumb_textures.remove(path); }
        }
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
        // Keep the parent expanded so its subfolders stay clickable after going up.
        self.folder_open.insert(parent.clone(), true);
        if self.folder == self.tree_root || !parent.starts_with(&self.tree_root) {
            self.tree_root = parent.clone();
        }
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
        self.thumb_loads_left = 4;
        self.poll_thumbnail_job();
        if self.thumbnail_job.is_some() { ctx.request_repaint_after(Duration::from_millis(50)); }
        // Left/Right move the selection in both modes. Up/Down move by grid row
        // in Thumbnails only — Filmstrip keeps those keys for zoom.
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft)) { self.navigate(-1, ctx); }
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowRight)) { self.navigate(1, ctx); }
        if self.mode == Mode::Thumbnails {
            let step = self.thumb_columns.max(1) as isize;
            if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) { self.navigate(-step, ctx); }
            if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) { self.navigate(step, ctx); }
        }
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
                // Reserve a fixed favorites block so the tree ScrollArea owns its
                // hover rect (otherwise the unbounded favorites scroller steals the wheel).
                let favorites_block = 150.0_f32;
                let tree_height = (ui.available_height() - favorites_block).max(100.0);
                ScrollArea::vertical()
                    .id_salt("folder_tree_scroll")
                    .max_height(tree_height)
                    .min_scrolled_height(tree_height)
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        folder_tree(ui, self, &root, &mut changed);
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Favorite places").strong());
                    if ui.small_button("+ Add…").clicked() { if let Some(path) = rfd::FileDialog::new().pick_folder() { self.add_favorite(path); } }
                });
                let favorites_height = (ui.available_height()).max(60.0);
                ScrollArea::vertical()
                    .id_salt("favorites_scroll")
                    .max_height(favorites_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut remove = None;
                        let favorites = self.config.favorites.clone();
                        for (index, favorite) in favorites.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let name = favorite.file_name().unwrap_or_else(|| favorite.as_os_str()).to_string_lossy();
                                if ui.selectable_label(self.folder == *favorite, name).clicked() {
                                    self.tree_root = favorite.clone();
                                    self.folder = favorite.clone();
                                    self.folder_open.insert(favorite.clone(), true);
                                    changed = true;
                                }
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

/// Folder tree with selection separated from expand/collapse.
/// Clicking the name opens that folder; ▸/▾ only toggles children.
fn folder_tree(ui: &mut egui::Ui, app: &mut App, dir: &Path, changed: &mut bool) {
    let name = dir.file_name().unwrap_or_else(|| dir.as_os_str()).to_string_lossy().to_string();
    let is_selected = dir == app.folder;
    let on_path = app.folder.starts_with(dir);
    let children = app.tree_children(dir);
    let has_children = !children.is_empty();

    // Ancestors of the active folder stay open so the user can navigate back down.
    let open = if on_path && !is_selected {
        true
    } else if let Some(forced) = app.folder_open.get(dir).copied() {
        forced
    } else {
        on_path
    };

    ui.horizontal(|ui| {
        if has_children {
            let arrow = if open { "▾" } else { "▸" };
            if ui.add(egui::Button::new(arrow).frame(false)).clicked() {
                app.folder_open.insert(dir.to_path_buf(), !open);
            }
        } else {
            ui.add_space(14.0);
        }
        let label = if is_selected {
            egui::RichText::new(name).background_color(ui.visuals().selection.bg_fill).color(ui.visuals().selection.stroke.color)
        } else {
            egui::RichText::new(name)
        };
        if ui.selectable_label(is_selected, label).clicked() {
            if dir != app.folder {
                app.folder = dir.to_path_buf();
                app.folder_open.insert(dir.to_path_buf(), true);
                *changed = true;
            }
        }
    });

    if has_children && open {
        let children = children.clone();
        ui.indent(egui::Id::new(("folder-indent", dir.to_path_buf())), |ui| {
            for child in children {
                folder_tree(ui, app, &child, changed);
            }
        });
    }
}

fn thumbnails_ui(ui: &mut egui::Ui, app: &mut App, ctx: &EguiContext) {
    ui.heading(app.folder.display().to_string()); ui.label(format!("{} image(s) · double-click an image to open the separate viewer", app.entries.len()));
    let width = ui.available_width(); let columns = (width / 184.0).max(1.0) as usize;
    app.thumb_columns = columns;
    ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("thumb-grid").spacing([18.0, 14.0]).show(ui, |ui| {
            for i in 0..app.entries.len() {
                let entry = app.entries[i].clone();
                let texture = app.thumbnail(&entry, ctx);
                ui.vertical(|ui| {
                    let r = if let Some(t) = texture {
                        ui.add(egui::ImageButton::new(Image::new(&t).fit_to_exact_size(Vec2::splat(160.0))).selected(app.selected == Some(i)))
                    } else {
                        ui.button("…")
                    };
                    if r.clicked() { app.select(i, ctx); }
                    if r.double_clicked() { app.viewer_open = true; }
                    if app.selected == Some(i) { r.scroll_to_me(Some(egui::Align::Center)); }
                    ui.label(egui::RichText::new(entry.name).small());
                });
                if (i + 1) % columns == 0 { ui.end_row(); }
            }
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

fn metadata_ui(ui: &mut egui::Ui, m: &Metadata) {
    ui.heading("Image information");
    if let Some(parser) = &m.parser {
        ui.label(egui::RichText::new(format!("Parser: {parser}")).small().weak());
    }
    ScrollArea::vertical().show(ui, |ui| {
        // Render translator fields in config order. LoRA rows stay grouped with
        // inactive entries shown in gray ("; inactive)" marker from extractors).
        let mut index = 0;
        while index < m.fields.len() {
            let (field, value) = &m.fields[index];
            if field == "LoRA" || field.eq_ignore_ascii_case("LoRA List") {
                ui.label(egui::RichText::new("LoRA list").strong());
                while index < m.fields.len() && (m.fields[index].0 == "LoRA" || m.fields[index].0.eq_ignore_ascii_case("LoRA List")) {
                    let value = &m.fields[index].1;
                    let inactive = value.contains("; inactive)");
                    ui.horizontal_wrapped(|ui| {
                        if inactive { ui.colored_label(egui::Color32::GRAY, value); }
                        else { ui.label(value); }
                        if ui.small_button("Copy").clicked() { copy(value); }
                    });
                    index += 1;
                }
                ui.separator();
                continue;
            }
            metadata_field(ui, field, value);
            index += 1;
        }
        if let Some((_, workflow)) = m.raw.iter().find(|(key, _)| key.eq_ignore_ascii_case("workflow")) {
            if ui.button("Copy workflow").clicked() { copy(workflow); }
            ui.separator();
        }
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
fn create_thumbnail(source: &Path, destination: &Path) -> Result<()> {
    let img = open_oriented(source).with_context(|| format!("opening {}", source.display()))?;
    if let Some(p) = destination.parent() { fs::create_dir_all(p)?; }
    img.resize(THUMB_SIZE, THUMB_SIZE, FilterType::Lanczos3).save_with_format(destination, image::ImageFormat::Png)?;
    Ok(())
}
fn load_image(path: &Path) -> Result<ColorImage> {
    let img = open_oriented(path)?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// Open an image and apply JPEG EXIF Orientation so portrait shots are not shown sideways.
fn open_oriented(path: &Path) -> Result<image::DynamicImage> {
    let img = image::open(path)?;
    Ok(apply_exif_orientation(img, exif_orientation(path)))
}

fn apply_exif_orientation(img: image::DynamicImage, orientation: u32) -> image::DynamicImage {
    use image::imageops::{self, flip_horizontal, flip_vertical};
    match orientation {
        2 => image::DynamicImage::ImageRgba8(flip_horizontal(&img)),
        3 => image::DynamicImage::ImageRgba8(imageops::rotate180(&img)),
        4 => image::DynamicImage::ImageRgba8(flip_vertical(&img)),
        5 => {
            let rotated = imageops::rotate90(&img);
            image::DynamicImage::ImageRgba8(flip_horizontal(&rotated))
        }
        6 => image::DynamicImage::ImageRgba8(imageops::rotate90(&img)),
        7 => {
            let rotated = imageops::rotate270(&img);
            image::DynamicImage::ImageRgba8(flip_horizontal(&rotated))
        }
        8 => image::DynamicImage::ImageRgba8(imageops::rotate270(&img)),
        _ => img,
    }
}
