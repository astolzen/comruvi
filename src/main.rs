#![cfg_attr(windows, windows_subsystem = "windows")]

mod file_drag;
mod meta_parsers;
mod ui_icons;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use arboard::Clipboard;
use eframe::egui::{self, ColorImage, Context as EguiContext, Image, ScrollArea, TextureHandle, TextureOptions, Vec2};
use image::imageops::FilterType;
use meta_parsers::{
    default_parser_dir, exif_orientation, image_has_active_lora, image_matches_volltext,
    load_parsers, metadata_cache_is_stale, read_metadata, write_image_metadata_cache, Metadata,
    ParserConfig,
};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

const THUMB_SIZE: u32 = 160;
/// From repo-root `VERSION` via `build.rs` (not `CARGO_PKG_VERSION`).
const APP_VERSION: &str = env!("COMRUVI_VERSION");
const CONFIG_VERSION: &str = env!("COMRUVI_CONFIG_VERSION");
const GITHUB_URL: &str = "https://github.com/astolzen/comruvi";
const INSTANCE_ID: &str = "comruvi-astolzen-single-instance";

/// How the process was started (CLI / file association / cold start).
struct LaunchRequest {
    folder: PathBuf,
    /// Image to select and show in the separate viewer (from "Open with" / double-click).
    open_image: Option<PathBuf>,
    /// When true, CLI/file-association wins over `config.lastfolder`.
    from_cli: bool,
}

fn main() -> eframe::Result<()> {
    let launch = parse_launch_request();

    // One UI process: further "Open with" invocations hand the path to the running app.
    let instance = single_instance::SingleInstance::new(INSTANCE_ID)
        .unwrap_or_else(|error| panic!("single-instance: {error}"));
    if !instance.is_single() {
        let handoff = launch
            .open_image
            .as_ref()
            .or(if launch.from_cli { Some(&launch.folder) } else { None });
        if let Some(path) = handoff {
            if let Err(error) = write_open_request(path) {
                // No console on Windows GUI builds; exit non-zero for diagnostics.
                let _ = error;
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(app_icon_arc()),
        ..Default::default()
    };
    eframe::run_native(
        "comruvi",
        options,
        Box::new(move |cc| {
            start_open_request_watcher(cc.egui_ctx.clone());
            Ok(Box::new(App::new(launch, instance)))
        }),
    )
}

fn parse_launch_request() -> LaunchRequest {
    let fallback = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(raw) = std::env::args().nth(1) else {
        return LaunchRequest { folder: fallback, open_image: None, from_cli: false };
    };
    let path = PathBuf::from(&raw);
    let path = path.canonicalize().unwrap_or(path);
    if path.is_file() {
        let folder = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| fallback.clone());
        let open_image = is_image(&path).then_some(path);
        return LaunchRequest { folder, open_image, from_cli: true };
    }
    if path.is_dir() {
        return LaunchRequest { folder: path, open_image: None, from_cli: true };
    }
    // Path may not exist yet (rare); if it looks like an image, use the parent.
    if is_image(&path) {
        let folder = path.parent().map(Path::to_path_buf).unwrap_or(fallback);
        return LaunchRequest { folder, open_image: Some(path), from_cli: true };
    }
    LaunchRequest { folder: fallback, open_image: None, from_cli: true }
}

fn open_request_path() -> PathBuf {
    Config::path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("open-request")
}

fn write_open_request(path: &Path) -> Result<()> {
    let target = open_request_path();
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = target.with_extension("tmp");
    fs::write(&tmp, path.to_string_lossy().as_bytes())?;
    fs::rename(&tmp, &target)?;
    Ok(())
}

fn take_open_request() -> Option<PathBuf> {
    let target = open_request_path();
    let contents = fs::read_to_string(&target).ok()?;
    let _ = fs::remove_file(&target);
    let path = PathBuf::from(contents.trim().trim_matches('"'));
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

fn start_open_request_watcher(ctx: EguiContext) {
    let watch_path = open_request_path();
    let _ = std::thread::Builder::new()
        .name("comruvi-open-request".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            if watch_path.is_file() {
                ctx.request_repaint();
            }
        });
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Rasterize `cv-ivon.svg` for textures and the window / taskbar icon.
fn load_app_icon_image() -> ColorImage {
    egui_extras::image::load_svg_bytes_with_size(
        include_bytes!("../cv-ivon.svg"),
        Some(egui::SizeHint::Size(256, 256)),
    )
    .expect("cv-ivon.svg must rasterize")
}

fn app_icon_arc() -> std::sync::Arc<egui::IconData> {
    use std::sync::OnceLock;
    static ICON: OnceLock<std::sync::Arc<egui::IconData>> = OnceLock::new();
    ICON.get_or_init(|| {
        let image = load_app_icon_image();
        std::sync::Arc::new(egui::IconData {
            rgba: image.as_raw().to_vec(),
            width: image.width() as u32,
            height: image.height() as u32,
        })
    })
    .clone()
}

#[derive(Clone)]
struct Entry { path: PathBuf, name: String }

struct App {
    folder: PathBuf, tree_root: PathBuf,
    /// All images in the folder (unfiltered).
    all_entries: Vec<Entry>,
    /// Visible images after the active view filter.
    entries: Vec<Entry>,
    selected: Option<usize>, mode: Mode,
    show_tree: bool, show_info: bool, viewer_open: bool, about_open: bool,
    thumb_textures: BTreeMap<PathBuf, TextureHandle>,
    image_texture: Option<TextureHandle>, about_icon: Option<TextureHandle>,
    metadata: Metadata, notice: Option<String>, config: Config,
    parsers: Vec<ParserConfig>,
    /// Last laid-out thumbnail grid column count (for Up/Down row navigation).
    thumb_columns: usize,
    /// Cap synchronous thumb→texture loads per frame so the UI stays responsive.
    thumb_loads_left: usize,
    /// Cached subdirectory listings for the folder tree (path → children).
    folder_children: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Manual expand/collapse overrides for tree nodes (true=open, false=closed).
    folder_open: BTreeMap<PathBuf, bool>,
    thumbnail_job: Option<BackgroundJob>,
    metadata_job: Option<BackgroundJob>,
    film_view: ViewTransform, viewer_view: ViewTransform,
    /// When set, start an OS file drag after the UI pass (needs the eframe window handle).
    pending_file_drag: Option<PathBuf>,
    /// Scroll the selected thumb into view once (arrow / Home / End navigation only).
    scroll_selection_into_view: bool,
    /// Live folder watcher (non-recursive) for the current image folder.
    folder_watcher: Option<RecommendedWatcher>,
    folder_events: Option<Receiver<notify::Result<Event>>>,
    watched_folder: Option<PathBuf>,
    /// Debounce timer after filesystem events before rescanning images.
    folder_dirty_since: Option<Instant>,
    /// Holds the OS single-instance lock for the lifetime of the UI process.
    _instance: single_instance::SingleInstance,
    /// Image/folder to open once egui context is available (CLI / file association).
    pending_external_open: Option<PathBuf>,
    /// Active view filters (volltext phrase, …).
    view_filter: ViewFilter,
    /// Bumped on every folder load so stale background workers are ignored/cancelled.
    job_generation: Arc<AtomicU64>,
}

#[derive(Default, Clone)]
struct ViewFilter {
    /// Contiguous phrase; empty means no full-text filter. Matching is case-insensitive.
    volltext: String,
    /// When set, only images with this LoRA marked active are shown.
    active_lora: Option<String>,
}

impl ViewFilter {
    fn is_active(&self) -> bool {
        !self.volltext.trim().is_empty() || self.active_lora.is_some()
    }

    fn reset(&mut self) {
        self.volltext.clear();
        self.active_lora = None;
    }
}
struct BackgroundJob {
    receiver: Receiver<bool>,
    completed: usize,
    total: usize,
    generation: u64,
    cancel: Arc<AtomicBool>,
}
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
            "version = {CONFIG_VERSION}\nthumbnailsize = \"{}\"\nfonttype = \"{}\"\nfontsize = {}\ndisplaymode = \"{}\"\nparserdir = \"{}\"\n",
            quote(&self.thumbnailsize),
            quote(&self.fonttype),
            self.fontsize,
            quote(&self.displaymode),
            quote(&self.parserdir.to_string_lossy()),
        );
        if let Some(folder) = &self.lastfolder { contents.push_str(&format!("lastfolder = \"{}\"\n", quote(&folder.to_string_lossy()))); }
        for (index, favorite) in self.favorites.iter().take(10).enumerate() { contents.push_str(&format!("quick{index} = \"{}\"\n", quote(&favorite.to_string_lossy()))); }
        fs::write(path, contents)?;
        Ok(())
    }
}

impl App {
    fn new(launch: LaunchRequest, instance: single_instance::SingleInstance) -> Self {
        let config = Config::load();
        // Drop a stale hand-off from a previous crash so we do not reopen it cold.
        let _ = fs::remove_file(open_request_path());
        let folder = if launch.from_cli {
            launch.folder
        } else {
            config
                .lastfolder
                .as_ref()
                .filter(|path| path.is_dir())
                .cloned()
                .unwrap_or(launch.folder)
        };
        let pending_external_open = launch.open_image.or_else(|| {
            // CLI folder-only: still switch into that folder (already applied above).
            None
        });
        let (parsers, parser_notice) = match load_parsers(&config.parserdir) {
            Ok(parsers) => (parsers, None),
            Err(error) => (Vec::new(), Some(format!("Could not load parsers: {error}"))),
        };
        let mut app = Self {
            tree_root: folder.clone(), folder,
            all_entries: vec![], entries: vec![], selected: None, mode: Mode::Thumbnails,
            show_tree: true, show_info: true, viewer_open: false, about_open: false,
            thumb_textures: BTreeMap::new(),
            image_texture: None, about_icon: None, metadata: Metadata::default(), notice: parser_notice, config, parsers,
            thumb_columns: 1, thumb_loads_left: 0, folder_children: BTreeMap::new(), folder_open: BTreeMap::new(),
            thumbnail_job: None, metadata_job: None,
            film_view: ViewTransform::default(), viewer_view: ViewTransform::default(),
            pending_file_drag: None, scroll_selection_into_view: false,
            folder_watcher: None, folder_events: None, watched_folder: None, folder_dirty_since: None,
            _instance: instance,
            pending_external_open,
            view_filter: ViewFilter::default(),
            job_generation: Arc::new(AtomicU64::new(0)),
        };
        app.load_folder();
        // Create/update the config on first launch as well, so every documented
        // placeholder has a stable default value.
        app.remember_folder();
        app
    }

    /// Open a folder or image from the shell / a second-instance hand-off.
    fn open_external_path(&mut self, path: PathBuf, ctx: &EguiContext) {
        let path = path.canonicalize().unwrap_or(path);
        if path.is_dir() {
            self.tree_root = path.clone();
            self.folder = path;
            self.mode = Mode::Thumbnails;
            self.viewer_open = false;
            self.load_folder();
            self.remember_folder();
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }
        if !is_image(&path) {
            self.notice = Some(format!("Not a supported image: {}", path.display()));
            return;
        }
        let folder = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.folder.clone());
        let folder = folder.canonicalize().unwrap_or(folder);
        if !paths_equal(&self.folder, &folder) {
            self.tree_root = folder.clone();
            self.folder = folder;
            self.load_folder();
            self.remember_folder();
        }
        self.mode = Mode::Thumbnails;
        // Shell opens should show the image even if a view filter would hide it.
        if self.view_filter.is_active()
            && !self
                .entries
                .iter()
                .any(|entry| paths_equal(&entry.path, &path))
            && self
                .all_entries
                .iter()
                .any(|entry| paths_equal(&entry.path, &path))
        {
            self.reset_view_filter(ctx);
        }
        if let Some(index) = self.entries.iter().position(|entry| paths_equal(&entry.path, &path)) {
            if self.selected != Some(index) {
                self.select(index, ctx);
            } else if self.image_texture.is_none() {
                self.selected = None;
                self.select(index, ctx);
            }
            self.viewer_open = true;
            self.viewer_view.reset();
            self.scroll_selection_into_view = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        } else {
            self.notice = Some(format!("Image not found in folder: {}", path.display()));
        }
    }

    fn poll_open_requests(&mut self, ctx: &EguiContext) {
        if let Some(path) = self.pending_external_open.take() {
            self.open_external_path(path, ctx);
        }
        if let Some(path) = take_open_request() {
            self.open_external_path(path, ctx);
        }
    }

    fn load_folder(&mut self) {
        // Cancel in-flight workers from the previous folder. Dropping the Receiver
        // alone left them writing and spamming the directory watcher.
        self.cancel_background_jobs();
        self.folder_children.clear();
        self.all_entries.clear();
        self.entries.clear();
        self.selected = None;
        self.image_texture = None;
        self.thumb_textures.clear();
        self.metadata = Metadata::default();
        self.all_entries = scan_images(&self.folder);
        self.apply_view_filter(None);
        self.watched_folder = None; // recreate watcher for the new folder on next frame
        self.folder_dirty_since = None;
        // Drain any pending watch events from the previous folder.
        if let Some(receiver) = &self.folder_events {
            while receiver.try_recv().is_ok() {}
        }
        self.queue_missing_thumbnails();
        self.queue_missing_metadata();
    }

    fn cancel_background_jobs(&mut self) {
        if let Some(job) = &self.thumbnail_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(job) = &self.metadata_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.thumbnail_job = None;
        self.metadata_job = None;
        self.job_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Rescan the current folder without wiping selection or cached thumbnails.
    fn refresh_entries(&mut self, ctx: &EguiContext) {
        let selected_path = self.selected_path();
        let new_entries = scan_images(&self.folder);
        if new_entries.len() == self.all_entries.len()
            && new_entries
                .iter()
                .zip(self.all_entries.iter())
                .all(|(a, b)| a.path == b.path)
        {
            return;
        }
        let keep: std::collections::BTreeSet<_> = new_entries.iter().map(|e| e.path.clone()).collect();
        self.thumb_textures.retain(|path, _| keep.contains(path));
        self.all_entries = new_entries;
        self.apply_view_filter(Some(ctx));
        if let Some(path) = selected_path {
            if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
                self.selected = Some(index);
            } else {
                self.selected = None;
                self.image_texture = None;
                self.metadata = Metadata::default();
            }
        }
        self.queue_missing_thumbnails();
        self.queue_missing_metadata();
        ctx.request_repaint();
    }

    fn apply_view_filter(&mut self, ctx: Option<&EguiContext>) {
        let selected_path = self.selected_path();
        let needle = self.view_filter.volltext.trim().to_ascii_lowercase();
        let active_lora = self.view_filter.active_lora.clone();
        self.entries = self
            .all_entries
            .iter()
            .filter(|entry| {
                if !needle.is_empty()
                    && !image_matches_volltext(&entry.path, &entry.name, &needle)
                {
                    return false;
                }
                if let Some(lora) = &active_lora {
                    if !image_has_active_lora(&entry.path, lora) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        if let Some(path) = selected_path {
            if let Some(index) = self
                .entries
                .iter()
                .position(|entry| paths_equal(&entry.path, &path))
            {
                self.selected = Some(index);
            } else if let Some(ctx) = ctx {
                if self.entries.is_empty() {
                    self.selected = None;
                    self.image_texture = None;
                    self.metadata = Metadata::default();
                    self.viewer_open = false;
                } else {
                    self.select(0, ctx);
                }
            } else {
                self.selected = None;
                self.image_texture = None;
                self.metadata = Metadata::default();
            }
        } else if self.selected.is_some() && self.selected.map(|i| i >= self.entries.len()).unwrap_or(false) {
            self.selected = None;
        }
    }

    fn reset_view_filter(&mut self, ctx: &EguiContext) {
        self.view_filter.reset();
        self.apply_view_filter(Some(ctx));
    }

    /// Watch the open folder for added/removed/renamed images (ignore cache dirs).
    fn ensure_folder_watch(&mut self, ctx: &EguiContext) {
        if self.watched_folder.as_ref() == Some(&self.folder) {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        let mut watcher = match notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
            repaint.request_repaint();
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                self.notice = Some(format!("Folder watch unavailable: {error}"));
                self.folder_watcher = None;
                self.folder_events = None;
                self.watched_folder = Some(self.folder.clone());
                return;
            }
        };
        if let Err(error) = watcher.watch(&self.folder, RecursiveMode::NonRecursive) {
            self.notice = Some(format!("Could not watch {}: {error}", self.folder.display()));
            self.folder_watcher = None;
            self.folder_events = None;
            self.watched_folder = Some(self.folder.clone());
            return;
        }
        self.folder_watcher = Some(watcher);
        self.folder_events = Some(receiver);
        self.watched_folder = Some(self.folder.clone());
    }

    fn poll_folder_changes(&mut self, ctx: &EguiContext) {
        // While our own thumbnail/metadata workers run, their creates under
        // `.thumbnails` / `.metadata` (and parent-dir noise on Windows) would
        // otherwise trigger refresh_entries and fight the jobs — sometimes
        // leaving the UI with an empty image list until restart.
        let jobs_running = self.thumbnail_job.is_some() || self.metadata_job.is_some();
        if jobs_running {
            if let Some(receiver) = &self.folder_events {
                while receiver.try_recv().is_ok() {}
            }
            self.folder_dirty_since = None;
            return;
        }

        let watched = self.watched_folder.clone();
        let mut dirty = false;
        if let Some(receiver) = &self.folder_events {
            while let Ok(message) = receiver.try_recv() {
                if let Ok(event) = message {
                    if folder_event_relevant(&event, watched.as_deref()) {
                        dirty = true;
                    }
                }
            }
        }
        if dirty {
            self.folder_dirty_since = Some(Instant::now());
        }
        if let Some(since) = self.folder_dirty_since {
            if since.elapsed() >= Duration::from_millis(400) {
                self.folder_dirty_since = None;
                self.refresh_entries(ctx);
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }
    fn tree_children(&mut self, dir: &Path) -> Vec<PathBuf> {
        if let Some(children) = self.folder_children.get(dir) { return children.clone(); }
        let mut children: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path.file_name().is_some_and(|name| {
                        name != ".thumbnails" && name != ".metadata"
                    })
            })
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
        let paths: Vec<_> = self.all_entries.iter().map(|entry| entry.path.clone()).collect();
        self.start_thumbnail_job(paths, true);
    }
    fn recreate_metadata(&mut self) {
        if self.metadata_job.is_some() { return; }
        let paths: Vec<_> = self.all_entries.iter().map(|entry| entry.path.clone()).collect();
        self.start_metadata_job(paths);
    }
    fn queue_missing_thumbnails(&mut self) {
        if self.thumbnail_job.is_some() { return; }
        let paths: Vec<_> = self
            .all_entries
            .iter()
            .filter(|entry| !thumbnail_path(&entry.path).exists())
            .map(|entry| entry.path.clone())
            .collect();
        if !paths.is_empty() { self.start_thumbnail_job(paths, false); }
    }
    fn queue_missing_metadata(&mut self) {
        if self.metadata_job.is_some() { return; }
        let paths: Vec<_> = self
            .all_entries
            .iter()
            .filter(|entry| metadata_cache_is_stale(&entry.path))
            .map(|entry| entry.path.clone())
            .collect();
        if !paths.is_empty() {
            self.start_metadata_job(paths);
        }
    }
    fn start_thumbnail_job(&mut self, paths: Vec<PathBuf>, clear_all_textures: bool) {
        let total = paths.len();
        if total == 0 { return; }
        let (sender, receiver) = mpsc::channel();
        let job_paths = paths.clone();
        let generation = self.job_generation.load(Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            for path in job_paths {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let succeeded = create_thumbnail(&path, &thumbnail_path(&path)).is_ok();
                if sender.send(succeeded).is_err() {
                    break;
                }
            }
        });
        if clear_all_textures {
            self.thumb_textures.clear();
        } else {
            for path in &paths { self.thumb_textures.remove(path); }
        }
        self.thumbnail_job = Some(BackgroundJob {
            receiver,
            completed: 0,
            total,
            generation,
            cancel,
        });
        self.refresh_job_notice();
    }
    fn start_metadata_job(&mut self, paths: Vec<PathBuf>) {
        let total = paths.len();
        if total == 0 { return; }
        let (sender, receiver) = mpsc::channel();
        let job_paths = paths;
        let parsers = self.parsers.clone();
        let generation = self.job_generation.load(Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            for path in job_paths {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let succeeded = write_image_metadata_cache(&path, &parsers).is_ok();
                if sender.send(succeeded).is_err() {
                    break;
                }
            }
        });
        self.metadata_job = Some(BackgroundJob {
            receiver,
            completed: 0,
            total,
            generation,
            cancel,
        });
        self.refresh_job_notice();
    }
    fn navigate(&mut self, direction: isize, ctx: &EguiContext) {
        if self.entries.is_empty() { return; }
        let current = self.selected.unwrap_or(0) as isize;
        let last = self.entries.len() as isize - 1;
        let target = (current + direction).clamp(0, last) as usize;
        self.jump_to(target, ctx);
    }
    fn jump_to(&mut self, index: usize, ctx: &EguiContext) {
        if index >= self.entries.len() { return; }
        if self.selected != Some(index) { self.select(index, ctx); }
        self.scroll_selection_into_view = true;
    }
    fn jump_home(&mut self, ctx: &EguiContext) { self.jump_to(0, ctx); }
    fn jump_end(&mut self, ctx: &EguiContext) {
        if self.entries.is_empty() { return; }
        self.jump_to(self.entries.len() - 1, ctx);
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
    fn poll_background_jobs(&mut self) {
        let current_gen = self.job_generation.load(Ordering::Relaxed);
        let mut thumb_done = false;
        let mut meta_done = false;

        // Drop jobs cancelled by a folder switch (generation mismatch).
        if self
            .thumbnail_job
            .as_ref()
            .is_some_and(|job| job.generation != current_gen)
        {
            if let Some(job) = &self.thumbnail_job {
                job.cancel.store(true, Ordering::Relaxed);
            }
            self.thumbnail_job = None;
        }
        if self
            .metadata_job
            .as_ref()
            .is_some_and(|job| job.generation != current_gen)
        {
            if let Some(job) = &self.metadata_job {
                job.cancel.store(true, Ordering::Relaxed);
            }
            self.metadata_job = None;
        }

        if let Some(job) = &mut self.thumbnail_job {
            while job.receiver.try_recv().is_ok() {
                job.completed += 1;
            }
            if job.completed >= job.total || job.cancel.load(Ordering::Relaxed) {
                thumb_done = true;
            }
        }
        if let Some(job) = &mut self.metadata_job {
            while job.receiver.try_recv().is_ok() {
                job.completed += 1;
            }
            if job.completed >= job.total || job.cancel.load(Ordering::Relaxed) {
                meta_done = true;
            }
        }
        if thumb_done {
            let cancelled = self
                .thumbnail_job
                .as_ref()
                .is_some_and(|job| job.cancel.load(Ordering::Relaxed));
            let total = self.thumbnail_job.as_ref().map(|j| j.total).unwrap_or(0);
            self.thumbnail_job = None;
            if !cancelled {
                self.notice = Some(format!("Thumbnail update finished ({total} images)."));
                self.queue_missing_thumbnails();
            }
        }
        if meta_done {
            let cancelled = self
                .metadata_job
                .as_ref()
                .is_some_and(|job| job.cancel.load(Ordering::Relaxed));
            let total = self.metadata_job.as_ref().map(|j| j.total).unwrap_or(0);
            self.metadata_job = None;
            if !cancelled {
                self.notice = Some(format!("Metadata cache update finished ({total} images)."));
                // New sidecars may change which images match the active filter.
                self.apply_view_filter(None);
                self.queue_missing_metadata();
            }
        }
        if self.thumbnail_job.is_some() || self.metadata_job.is_some() {
            self.refresh_job_notice();
        }
    }

    fn refresh_job_notice(&mut self) {
        let mut parts = Vec::new();
        if let Some(job) = &self.thumbnail_job {
            parts.push(format!("Thumbnails ({}/{})", job.completed, job.total));
        }
        if let Some(job) = &self.metadata_job {
            parts.push(format!("Metadata ({}/{})", job.completed, job.total));
        }
        if !parts.is_empty() {
            self.notice = Some(format!("Creating/updating {}", parts.join(" · ")));
        }
    }
    fn selected_path(&self) -> Option<PathBuf> {
        self.selected.and_then(|index| self.entries.get(index).map(|entry| entry.path.clone()))
    }
    /// Queue an OS file drag (drop into ComfyUI to open a tab with embedded workflow).
    fn queue_file_drag(&mut self, path: PathBuf) {
        self.pending_file_drag = Some(path);
    }

    fn run_pending_file_drag(&mut self, frame: &eframe::Frame) {
        let Some(path) = self.pending_file_drag.take() else { return; };
        match file_drag::drag_file(frame, &path) {
            // start_drag blocks until drop/cancel; success means the OS drag loop ran.
            Ok(()) => self.notice = Some("If you dropped on ComfyUI, a new tab with the workflow should open.".into()),
            Err(error) => self.notice = Some(error),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &EguiContext, frame: &mut eframe::Frame) {
        self.thumb_loads_left = 4;
        self.poll_open_requests(ctx);
        self.ensure_folder_watch(ctx);
        self.poll_folder_changes(ctx);
        self.poll_background_jobs();
        if self.thumbnail_job.is_some() || self.metadata_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        // Left/Right move the selection in both modes. Up/Down move by grid row
        // in Thumbnails only — Filmstrip keeps those keys for zoom.
        // Home/End jump to first/last. Arrow/Home/End also scroll the selection into view.
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft)) { self.navigate(-1, ctx); }
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowRight)) { self.navigate(1, ctx); }
        if ctx.input(|input| input.key_pressed(egui::Key::Home)) { self.jump_home(ctx); }
        if ctx.input(|input| input.key_pressed(egui::Key::End)) { self.jump_end(ctx); }
        if self.mode == Mode::Thumbnails {
            let step = self.thumb_columns.max(1) as isize;
            if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) { self.navigate(-step, ctx); }
            if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) { self.navigate(step, ctx); }
        }
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui_icons::icon_text_button(ui, ui_icons::Icon::FolderOpen, "Open folder").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.tree_root = p.clone();
                        self.folder = p;
                        self.load_folder();
                        self.remember_folder();
                    }
                }
                if ui_icons::icon_text_button(ui, ui_icons::Icon::ArrowUp, "Up")
                    .on_hover_text("Go to parent folder")
                    .clicked()
                {
                    self.go_up();
                }
                ui.menu_button("Recreate", |ui| {
                    let thumbs_busy = self.thumbnail_job.is_some();
                    let meta_busy = self.metadata_job.is_some();
                    if ui
                        .add_enabled(!thumbs_busy, egui::Button::new("Thumbnails"))
                        .on_hover_text("Rebuild .thumbnails for every image in this folder")
                        .clicked()
                    {
                        self.recreate_thumbnails();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(!meta_busy, egui::Button::new("Metadata"))
                        .on_hover_text("Rebuild .metadata/*.json sidecars for every image in this folder")
                        .clicked()
                    {
                        self.recreate_metadata();
                        ui.close_menu();
                    }
                });
                {
                    let filter_active = self.view_filter.is_active();
                    let mut filter_changed = false;
                    let filter_button = egui::Button::new("Filter").selected(filter_active);
                    egui::menu::menu_custom_button(ui, filter_button, |ui| {
                        if ui
                            .add_enabled(filter_active, egui::Button::new("reset"))
                            .on_hover_text("Clear all view filters")
                            .clicked()
                        {
                            self.view_filter.reset();
                            filter_changed = true;
                            ui.close_menu();
                        }
                        if let Some(lora) = &self.view_filter.active_lora {
                            ui.label(
                                egui::RichText::new(format!("LoRA (active): {lora}")).small(),
                            );
                        }
                        ui.separator();
                        ui.label("full text");
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.view_filter.volltext)
                                .desired_width(220.0)
                                .hint_text("blue eyes"),
                        );
                        if edit.changed() {
                            filter_changed = true;
                        }
                        if filter_active {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} / {} images",
                                    self.entries.len(),
                                    self.all_entries.len()
                                ))
                                .small()
                                .weak(),
                            );
                        }
                    });
                    if filter_changed {
                        self.apply_view_filter(Some(ctx));
                    }
                }
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
                if self.thumbnail_job.is_some() || self.metadata_job.is_some() {
                    ui.add(egui::Spinner::new());
                }
                if let Some(n) = &self.notice { ui.label(n); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("About").clicked() { self.about_open = true; }
                });
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
                    if ui_icons::icon_button(ui, ui_icons::Icon::Plus)
                        .on_hover_text("Add favorite folder")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.add_favorite(path);
                        }
                    }
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
                                if ui_icons::icon_button(ui, ui_icons::Icon::Close)
                                    .on_hover_text("Remove favorite")
                                    .clicked()
                                {
                                    remove = Some(index);
                                }
                            });
                        }
                        if let Some(index) = remove { self.config.favorites.remove(index); if let Err(error) = self.config.save() { self.notice = Some(format!("Could not save config: {error}")); } }
                    });
            });
            if changed { self.load_folder(); self.remember_folder(); }
        }
        if self.show_info {
            egui::SidePanel::right("metadata").resizable(true).default_width(330.0).show(ctx, |ui| {
                metadata_ui(ui, self, ctx);
            });
        }
        egui::CentralPanel::default().show(ctx, |ui| match self.mode { Mode::Thumbnails => thumbnails_ui(ui, self, ctx), Mode::Filmstrip => filmstrip_ui(ui, self, ctx) });
        if self.viewer_open { viewer_ui(ctx, self); }
        if self.about_open { about_ui(ctx, self); }
        // OS drag blocks; run after the egui pass so we still have a valid window handle.
        self.run_pending_file_drag(frame);
    }
}

/// Folder tree with selection separated from expand/collapse.
/// Clicking the name opens that folder; chevron icons only toggle children.
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
            let chevron = if open { ui_icons::Icon::ChevronDown } else { ui_icons::Icon::ChevronRight };
            if ui_icons::icon_button(ui, chevron).clicked() {
                app.folder_open.insert(dir.to_path_buf(), !open);
            }
        } else {
            ui.add_space(ui.spacing().interact_size.y.min(22.0));
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
    ui.heading(app.folder.display().to_string());
    if app.view_filter.is_active() {
        ui.label(format!(
            "{} of {} image(s) · double-click to open viewer · Shift+drag into ComfyUI",
            app.entries.len(),
            app.all_entries.len()
        ));
        if app.entries.is_empty() && !app.all_entries.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(200, 140, 40),
                "Active filter hides all images in this folder — use Filter → reset to show them.",
            );
        }
    } else {
        ui.label(format!(
            "{} image(s) · double-click to open viewer · Shift+drag into ComfyUI",
            app.entries.len()
        ));
    }
    let width = ui.available_width(); let columns = (width / 184.0).max(1.0) as usize;
    app.thumb_columns = columns;
    let scroll_selected = app.scroll_selection_into_view;
    ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("thumb-grid").spacing([18.0, 14.0]).show(ui, |ui| {
            for i in 0..app.entries.len() {
                let entry = app.entries[i].clone();
                let texture = app.thumbnail(&entry, ctx);
                ui.vertical(|ui| {
                    let r = if let Some(t) = texture {
                        ui.add(
                            egui::ImageButton::new(Image::new(&t).fit_to_exact_size(Vec2::splat(160.0)))
                                .selected(app.selected == Some(i))
                                .sense(egui::Sense::click_and_drag()),
                        )
                        .on_hover_text("Shift+drag into ComfyUI to open the embedded workflow")
                    } else {
                        ui.add(egui::Button::new("...").sense(egui::Sense::click_and_drag()))
                    };
                    if r.clicked() { app.select(i, ctx); }
                    if r.double_clicked() { app.viewer_open = true; }
                    let shift = ui.input(|input| input.modifiers.shift);
                    if shift && r.drag_started() {
                        app.select(i, ctx);
                        app.queue_file_drag(entry.path.clone());
                    }
                    if scroll_selected && app.selected == Some(i) {
                        r.scroll_to_me(Some(egui::Align::Center));
                    }
                    ui.label(egui::RichText::new(entry.name).small());
                });
                if (i + 1) % columns == 0 { ui.end_row(); }
            }
        });
    });
    if scroll_selected { app.scroll_selection_into_view = false; }
}

fn filmstrip_ui(ui: &mut egui::Ui, app: &mut App, ctx: &EguiContext) {
    if app.selected.is_none() && !app.entries.is_empty() { app.select(0, ctx); }
    let image_height = (ui.available_height() - 195.0).max(100.0);
    if let Some(t) = &app.image_texture {
        if zoomable_image(ui, t, &mut app.film_view, Vec2::new(ui.available_width(), image_height), true) {
            if let Some(path) = app.selected_path() { app.queue_file_drag(path); }
        }
    } else {
        ui.allocate_ui(Vec2::new(ui.available_width(), image_height), |ui| {
            ui.centered_and_justified(|ui| { ui.label("Select an image"); });
        });
    }
    ui.separator();
    let scroll_selected = app.scroll_selection_into_view;
    ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            for i in 0..app.entries.len() {
                let entry = app.entries[i].clone();
                if let Some(t) = app.thumbnail(&entry, ctx) {
                    let r = ui.add(
                        egui::ImageButton::new(Image::new(&t).fit_to_exact_size(Vec2::splat(110.0)))
                            .selected(app.selected == Some(i))
                            .sense(egui::Sense::click_and_drag()),
                    )
                    .on_hover_text("Shift+drag into ComfyUI");
                    if r.clicked() { app.select(i, ctx); }
                    if r.double_clicked() { app.viewer_open = true; }
                    let shift = ui.input(|input| input.modifiers.shift);
                    if shift && r.drag_started() {
                        app.select(i, ctx);
                        app.queue_file_drag(entry.path.clone());
                    }
                    if scroll_selected && app.selected == Some(i) {
                        r.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            }
        });
    });
    if scroll_selected { app.scroll_selection_into_view = false; }
}

fn viewer_ui(ctx: &EguiContext, app: &mut App) {
    let texture = app.image_texture.clone();
    let mut close_requested = false;
    let mut start_file_drag = false;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("separate-viewer"),
        egui::ViewportBuilder::default().with_title("comruvi — viewer").with_inner_size([1000.0, 750.0]),
        |viewer_ctx, _| {
            close_requested = viewer_ctx.input(|input| input.viewport().close_requested());
            egui::CentralPanel::default().show(viewer_ctx, |ui| {
                if let Some(t) = &texture {
                    if zoomable_image(ui, t, &mut app.viewer_view, ui.available_size(), true) {
                        start_file_drag = true;
                    }
                } else {
                    ui.label("Select an image in the main window.");
                }
            });
        },
    );
    if start_file_drag {
        if let Some(path) = app.selected_path() { app.queue_file_drag(path); }
    }
    if close_requested { app.viewer_open = false; }
}

fn about_ui(ctx: &EguiContext, app: &mut App) {
    if app.about_icon.is_none() {
        let image = load_app_icon_image();
        app.about_icon = Some(ctx.load_texture("about-icon", image, TextureOptions::LINEAR));
    }
    let icon = app.about_icon.clone();
    let mut close_requested = false;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("about-window"),
        egui::ViewportBuilder::default()
            .with_title("About comruvi")
            .with_inner_size([360.0, 320.0])
            .with_resizable(false)
            .with_icon(app_icon_arc()),
        |about_ctx, _| {
            close_requested = about_ctx.input(|input| input.viewport().close_requested());
            egui::CentralPanel::default().show(about_ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(16.0);
                    if let Some(texture) = &icon {
                        ui.add(Image::new(texture).fit_to_exact_size(Vec2::splat(96.0)));
                    }
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("Comruvi").strong().size(32.0));
                    ui.add_space(6.0);
                    ui.label(format!("Version {APP_VERSION}"));
                    ui.add_space(12.0);
                    ui.hyperlink_to("github.com/astolzen/comruvi", GITHUB_URL);
                });
            });
        },
    );
    if close_requested { app.about_open = false; }
}

/// Draw an image into a clipped viewport with independent zoom and pan state.
/// `None` scale means "fit viewport"; `Some(1.0)` means original pixels.
/// Returns true when Shift+drag should start an OS file drag-out (ComfyUI drop).
fn zoomable_image(
    ui: &mut egui::Ui,
    texture: &TextureHandle,
    view: &mut ViewTransform,
    desired_size: Vec2,
    allow_file_drag: bool,
) -> bool {
    let response = ui.allocate_response(desired_size, egui::Sense::drag());
    if allow_file_drag {
        response.clone().on_hover_text("Shift+drag to drop this file into ComfyUI");
    }
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
    let shift = ui.input(|input| input.modifiers.shift);
    let start_file_drag = allow_file_drag && shift && response.drag_started();
    if response.dragged() && !shift { view.pan += response.drag_delta(); }
    let scale = view.scale.unwrap_or(fit);
    let image_rect = egui::Rect::from_center_size(response.rect.center() + view.pan, source * scale);
    ui.painter().with_clip_rect(response.rect).image(texture.id(), image_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    start_file_drag
}

fn metadata_ui(ui: &mut egui::Ui, app: &mut App, ctx: &EguiContext) {
    let m = &app.metadata;
    ui.heading("Image information");
    if let Some(parser) = &m.parser {
        ui.label(egui::RichText::new(format!("Parser: {parser}")).small().weak());
    }
    let has_workflow = m.raw.iter().any(|(key, _)| key.eq_ignore_ascii_case("workflow"));
    let workflow = m.raw.iter().find(|(key, _)| key.eq_ignore_ascii_case("workflow")).map(|(_, value)| value.clone());

    ScrollArea::vertical().show(ui, |ui| {
        // Render translator fields in config order. LoRA rows stay grouped with
        // inactive entries shown in gray ("; inactive)" marker from extractors).
        let mut index = 0;
        while index < app.metadata.fields.len() {
            let (field, value) = &app.metadata.fields[index];
            if field == "LoRA" || field.eq_ignore_ascii_case("LoRA List") {
                ui.label(egui::RichText::new("LoRA list").strong());
                while index < app.metadata.fields.len()
                    && (app.metadata.fields[index].0 == "LoRA" || app.metadata.fields[index].0.eq_ignore_ascii_case("LoRA List"))
                {
                    let value = app.metadata.fields[index].1.clone();
                    let inactive = value.contains("; inactive)");
                    let lora_name = value
                        .split(" (")
                        .next()
                        .unwrap_or(value.as_str())
                        .trim()
                        .to_string();
                    let lora_filter_on = app
                        .view_filter
                        .active_lora
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&lora_name));
                    ui.horizontal_wrapped(|ui| {
                        if inactive {
                            ui.colored_label(egui::Color32::GRAY, &value);
                        } else {
                            ui.label(&value);
                        }
                        if ui.small_button("Copy").clicked() {
                            copy(&value);
                        }
                        if !inactive && !lora_name.is_empty() {
                            if ui
                                .add(egui::Button::new("Filter").selected(lora_filter_on).small())
                                .on_hover_text("Show only images where this LoRA is active")
                                .clicked()
                            {
                                if lora_filter_on {
                                    app.view_filter.active_lora = None;
                                } else {
                                    app.view_filter.active_lora = Some(lora_name.clone());
                                }
                                app.apply_view_filter(Some(ctx));
                            }
                        }
                    });
                    index += 1;
                }
                ui.separator();
                continue;
            }
            let field = field.clone();
            let value = value.clone();
            metadata_field(ui, &field, &value);
            index += 1;
        }
        if has_workflow {
            if let Some(workflow) = &workflow {
                if ui.button("Copy workflow").clicked() { copy(workflow); }
            }
            ui.label(egui::RichText::new("Shift+drag the thumbnail or image into ComfyUI to open the workflow.").small().weak());
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

fn scan_images(folder: &Path) -> Vec<Entry> {
    let Ok(read) = fs::read_dir(folder) else { return Vec::new(); };
    let mut files: Vec<_> = read.flatten().filter_map(|entry| {
        let path = entry.path();
        is_image(&path).then(|| Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
        })
    }).collect();
    files.sort_by_key(|entry| entry.name.to_lowercase());
    files
}

/// Ignore cache-dir writes and pure "watched root" noise; other changes may add/remove images.
fn folder_event_relevant(event: &Event, watched_folder: Option<&Path>) -> bool {
    event.paths.iter().any(|path| {
        if let Some(watched) = watched_folder {
            if path == watched {
                // Windows often emits the parent dir when children change; ignore that.
                return false;
            }
        }
        let is_cache = path.components().any(|component| {
            let name = component.as_os_str();
            name == ".thumbnails" || name == ".metadata"
        });
        !is_cache
    })
}

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
