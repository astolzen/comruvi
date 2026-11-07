//! Human-readable metadata translator (parser) configs.
//!
//! Cached metadata for filters lives either in per-folder `.metadata` JSON files
//! or in a central SQLite DB — see `meta_store`.
//!
//! Files live under `parserdir` (default `~/.config/comruvi/parsers`) as TOML.
//! Each file has a trigger that selects it, then a list of display fields.
//!
//! Per-folder caches live next to images under `.metadata/<filename>.json`
//! (same idea as `.thumbnails/`).

use std::{
    collections::BTreeMap,
    fs,
    io::BufReader,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};
use exif::{In, Reader as ExifReader, Tag, Value as ExifValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct ParserConfig {
    pub name: String,
    #[serde(default)]
    pub priority: i32,
    /// png | jpg | jpeg | *  (jpg also matches jpeg)
    #[serde(default = "default_filetype")]
    pub filetype: String,
    /// PNG text chunk key that holds the ComfyUI node graph (usually "prompt").
    #[serde(default = "default_chunk")]
    pub chunk: String,
    /// Optional built-in extractor, e.g. "comfy_prompt" for the generic fallback.
    #[serde(default)]
    pub builtin: Option<String>,
    #[serde(default)]
    pub trigger: Vec<Trigger>,
    #[serde(default)]
    pub field: Vec<FieldRule>,
}

fn default_filetype() -> String { "*".into() }
fn default_chunk() -> String { "prompt".into() }

#[derive(Debug, Clone, Deserialize)]
pub struct Trigger {
    #[serde(default)]
    pub node_class: Option<String>,
    #[serde(default)]
    pub node_title: Option<String>,
    #[serde(default)]
    pub node_title_contains: Option<String>,
    /// EXIF tag must exist (friendly name, e.g. Model, Make, LensModel).
    #[serde(default)]
    pub exif: Option<String>,
    /// XMP property must exist (short or `prefix:Name`, e.g. `subject`, `dc:subject`).
    #[serde(default)]
    pub xmp: Option<String>,
    /// Raw text metadata key must exist (e.g. prompt).
    #[serde(default)]
    pub text_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldRule {
    pub label: String,
    #[serde(default)]
    pub constant: Option<String>,
    #[serde(default)]
    pub node_class: Option<String>,
    #[serde(default)]
    pub node_title: Option<String>,
    #[serde(default)]
    pub node_title_contains: Option<String>,
    /// Input key under the node's `inputs` object.
    #[serde(default)]
    pub input: Option<String>,
    /// power_lora | lora_manager | lora_loader | lora_stack |
    /// krea2_regional_loras | krea2_regional_regions
    #[serde(default)]
    pub extract: Option<String>,
    #[serde(default)]
    pub exif: Option<String>,
    /// XMP property (short or `prefix:Name`, e.g. `Rating` / `xmp:Rating` / `dc:subject`).
    #[serde(default)]
    pub xmp: Option<String>,
}

#[derive(Debug, Default)]
pub struct ImageMetaBag {
    pub extension: String,
    pub text: BTreeMap<String, String>,
    pub exif: BTreeMap<String, String>,
    /// Flattened XMP properties from JPEG APP1 / sidecar (`dc:subject`, `xmp:Rating`, …).
    pub xmp: BTreeMap<String, String>,
    pub prompt_nodes: Option<serde_json::Map<String, Value>>,
}

#[derive(Default)]
pub struct Metadata {
    pub fields: Vec<(String, String)>,
    pub raw: Vec<(String, String)>,
    pub parser: Option<String>,
}

impl Metadata {
    pub fn error(e: String) -> Self {
        Self { fields: vec![("Error".into(), e)], raw: vec![], parser: None }
    }
}

/// On-disk sidecar written under `.metadata/` for fast future filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedImageMeta {
    pub version: u32,
    pub source_mtime: u64,
    pub source_size: u64,
    pub parser: Option<String>,
    pub fields: Vec<(String, String)>,
    /// Convenience copies for filters (derived from `fields`).
    #[serde(default)]
    pub positive: Option<String>,
    #[serde(default)]
    pub negative: Option<String>,
    /// All LoRA names mentioned in the translator fields.
    #[serde(default)]
    pub loras: Vec<String>,
    /// LoRA names marked active (not `; inactive)` in the display string).
    #[serde(default)]
    pub loras_active: Vec<String>,
    /// Display rotation in 90° clockwise steps from the file's raw pixels (0–3).
    /// Initialized from EXIF Orientation; user keys 2/3 adjust this without
    /// rewriting the original image.
    #[serde(default)]
    pub orientation: u8,
    /// Lowercased EXIF/XMP/PNG raw keys+values so volltext can match embedded
    /// metadata even when no translator surfaces those fields.
    #[serde(default)]
    pub raw_search: String,
}

pub const METADATA_CACHE_VERSION: u32 = 4;

fn raw_search_blob(raw: &[(String, String)]) -> String {
    raw.iter()
        .flat_map(|(key, value)| [key.as_str(), value.as_str()])
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase()
}

/// `.metadata/<filename>.json` next to the image (Geeqie-style sidecar layout).
pub fn metadata_cache_path(image: &Path) -> PathBuf {
    let name = image
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());
    image
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".metadata")
        .join(format!("{name}.json"))
}

pub fn source_stamp(path: &Path) -> Result<(u64, u64)> {
    let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let mtime = meta
        .modified()
        .with_context(|| format!("mtime {}", path.display()))?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok((mtime, meta.len()))
}

fn field_value<'a>(fields: &'a [(String, String)], label: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(label))
        .map(|(_, value)| value.as_str())
}

fn lora_display_name(value: &str) -> String {
    value
        .split(" (")
        .next()
        .unwrap_or(value)
        .trim()
        .to_string()
}

fn is_inactive_lora_value(value: &str) -> bool {
    value.contains("; inactive)")
}

fn derive_loras(fields: &[(String, String)], active_only: bool) -> Vec<String> {
    fields
        .iter()
        .filter(|(key, _)| key == "LoRA" || key.eq_ignore_ascii_case("LoRA List"))
        .filter(|(_, value)| !active_only || !is_inactive_lora_value(value))
        .map(|(_, value)| lora_display_name(value))
        .filter(|name| !name.is_empty())
        .collect()
}

/// True when no file cache exists, version mismatches, or the image changed on disk.
pub fn metadata_cache_is_stale_file(image: &Path) -> bool {
    let cache = metadata_cache_path(image);
    let Ok(text) = fs::read_to_string(&cache) else {
        return true;
    };
    let Ok(cached) = serde_json::from_str::<CachedImageMeta>(&text) else {
        return true;
    };
    if cached.version != METADATA_CACHE_VERSION {
        return true;
    }
    match source_stamp(image) {
        Ok((mtime, size)) => cached.source_mtime != mtime || cached.source_size != size,
        Err(_) => true,
    }
}

/// Load a `.metadata` sidecar if present and parseable.
pub fn load_cached_image_meta_file(image: &Path) -> Option<CachedImageMeta> {
    let text = fs::read_to_string(metadata_cache_path(image)).ok()?;
    serde_json::from_str(&text).ok()
}

impl CachedImageMeta {
    /// Lowercased blob used for case-insensitive phrase search (volltext filter).
    pub fn search_blob_lower(&self) -> String {
        let mut parts = Vec::new();
        if let Some(parser) = &self.parser {
            parts.push(parser.clone());
        }
        if let Some(positive) = &self.positive {
            parts.push(positive.clone());
        }
        if let Some(negative) = &self.negative {
            parts.push(negative.clone());
        }
        for lora in &self.loras {
            parts.push(lora.clone());
        }
        for lora in &self.loras_active {
            parts.push(lora.clone());
        }
        for (key, value) in &self.fields {
            parts.push(key.clone());
            parts.push(value.clone());
        }
        if !self.raw_search.is_empty() {
            parts.push(self.raw_search.clone());
        }
        parts.join("\n").to_ascii_lowercase()
    }

    pub fn active_lora_names(&self) -> Vec<String> {
        if !self.loras_active.is_empty() {
            return self.loras_active.clone();
        }
        // Older caches / fallback: derive from LoRA field rows.
        derive_loras(&self.fields, true)
    }
}

/// Map EXIF Orientation (1–8) to quarter-turns clockwise from raw pixels.
/// Mirrored orientations are approximated to the nearest rotation.
pub fn exif_to_quarter_turns(exif: u32) -> u8 {
    match exif {
        6 | 5 => 1, // 90° CW
        3 | 4 => 2, // 180°
        8 | 7 => 3, // 270° CW
        _ => 0,     // 1, 2, missing → upright
    }
}

pub fn write_cached_image_meta_file(image: &Path, cached: &CachedImageMeta) -> Result<PathBuf> {
    let dest = metadata_cache_path(image);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(cached).context("serialize metadata cache")?;
    let tmp = dest.with_extension("json.tmp");
    fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &dest).with_context(|| format!("renaming {}", dest.display()))?;
    Ok(dest)
}

/// Extract translator fields for `image` and write `.metadata/<name>.json`.
pub fn write_image_metadata_cache_file(image: &Path, parsers: &[ParserConfig]) -> Result<PathBuf> {
    let (mtime, size) = source_stamp(image)?;
    // Keep a user-chosen rotation when the source file itself has not changed
    // and the sidecar already carries a real orientation (v3+).
    let orientation = load_cached_image_meta_file(image)
        .filter(|cached| {
            cached.version >= METADATA_CACHE_VERSION
                && cached.source_mtime == mtime
                && cached.source_size == size
        })
        .map(|cached| cached.orientation % 4)
        .unwrap_or_else(|| exif_to_quarter_turns(exif_orientation(image)));
    let meta = read_metadata(image, parsers)?;
    let cached = CachedImageMeta {
        version: METADATA_CACHE_VERSION,
        source_mtime: mtime,
        source_size: size,
        parser: meta.parser.clone(),
        positive: field_value(&meta.fields, "Positive prompt").map(str::to_string),
        negative: field_value(&meta.fields, "Negative prompt").map(str::to_string),
        loras: derive_loras(&meta.fields, false),
        loras_active: derive_loras(&meta.fields, true),
        fields: meta.fields,
        orientation,
        raw_search: raw_search_blob(&meta.raw),
    };
    write_cached_image_meta_file(image, &cached)
}

const DEFAULT_PARSERS: &[(&str, &str)] = &[
    ("krea-ai.toml", include_str!("../parsers/krea-ai.toml")),
    ("k2regional.toml", include_str!("../parsers/k2regional.toml")),
    ("anima-ai.toml", include_str!("../parsers/anima-ai.toml")),
    ("z-image.toml", include_str!("../parsers/z-image.toml")),
    ("flux2.toml", include_str!("../parsers/flux2.toml")),
    ("comfyui-generic.toml", include_str!("../parsers/comfyui-generic.toml")),
    ("digikam.toml", include_str!("../parsers/digikam.toml")),
    ("photo.toml", include_str!("../parsers/photo.toml")),
];

pub fn default_parser_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
        .join(".config").join("comruvi").join("parsers")
}

/// Create `dir` and write shipped defaults for any missing filenames (never overwrite).
pub fn ensure_default_parsers(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating parser dir {}", dir.display()))?;
    for (name, contents) in DEFAULT_PARSERS {
        let path = dir.join(name);
        if !path.exists() {
            fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn load_parsers(dir: &Path) -> Result<Vec<ParserConfig>> {
    ensure_default_parsers(dir)?;
    let mut parsers = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?
        .flatten().map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("toml")))
        .collect();
    entries.sort();
    for path in entries {
        let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        match toml::from_str::<ParserConfig>(&text) {
            Ok(parser) => parsers.push(parser),
            Err(error) => eprintln!("comruvi: skipping parser {}: {error}", path.display()),
        }
    }
    parsers.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.name.cmp(&b.name)));
    Ok(parsers)
}

pub fn read_metadata(path: &Path, parsers: &[ParserConfig]) -> Result<Metadata> {
    // Header-only size probe — never decode pixels (metadata jobs hit every file).
    let (width, height) = match image::image_dimensions(path) {
        Ok((w, h)) => oriented_dimensions(w, h, exif_orientation(path)),
        Err(_) => (0, 0),
    };
    let mut meta = Metadata {
        fields: vec![
            ("File".into(), path.file_name().unwrap_or_default().to_string_lossy().into()),
            ("Dimensions".into(), if width > 0 {
                format!("{width} × {height}")
            } else {
                "? × ?".into()
            }),
            ("Format".into(), path.extension().and_then(|x| x.to_str()).unwrap_or("unknown").to_ascii_uppercase()),
        ],
        raw: vec![],
        parser: None,
    };

    let bag = collect_bag(path)?;
    for (key, value) in &bag.text {
        meta.raw.push((key.clone(), value.clone()));
    }
    for (key, value) in &bag.exif {
        meta.raw.push((format!("exif:{key}"), value.clone()));
    }
    for (key, value) in &bag.xmp {
        meta.raw.push((format!("xmp:{key}"), value.clone()));
    }

    if let Some(parser) = select_parser(parsers, &bag) {
        meta.parser = Some(parser.name.clone());
        apply_parser(parser, &bag, &mut meta.fields);
    }

    Ok(meta)
}

fn collect_bag(path: &Path) -> Result<ImageMetaBag> {
    let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut bag = ImageMetaBag { extension: ext.clone(), ..Default::default() };

    if ext == "png" {
        for (key, value) in png_text_chunks(path)? {
            if key == "prompt" {
                if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&value) {
                    bag.prompt_nodes = Some(map);
                }
            }
            bag.text.insert(key, value);
        }
    } else if matches!(ext.as_str(), "jpg" | "jpeg") {
        bag.exif = read_exif_map(path)?;
        bag.xmp = crate::xmp_jpeg::load_xmp_map(path).unwrap_or_default();
    }
    Ok(bag)
}

fn select_parser<'a>(parsers: &'a [ParserConfig], bag: &ImageMetaBag) -> Option<&'a ParserConfig> {
    parsers.iter().find(|parser| filetype_matches(&parser.filetype, &bag.extension) && triggers_match(parser, bag))
}

fn filetype_matches(spec: &str, ext: &str) -> bool {
    let spec = spec.trim().to_ascii_lowercase();
    if spec.is_empty() || spec == "*" || spec == "any" { return true; }
    if spec == "jpg" || spec == "jpeg" { return matches!(ext, "jpg" | "jpeg"); }
    spec == ext
}

fn triggers_match(parser: &ParserConfig, bag: &ImageMetaBag) -> bool {
    if parser.trigger.is_empty() { return false; }
    parser.trigger.iter().all(|trigger| trigger_match(trigger, bag, &parser.chunk))
}

fn trigger_match(trigger: &Trigger, bag: &ImageMetaBag, chunk: &str) -> bool {
    if let Some(key) = &trigger.text_key {
        if !bag.text.contains_key(key) { return false; }
    }
    if let Some(tag) = &trigger.exif {
        if lookup_map(&bag.exif, tag).is_none() { return false; }
    }
    if let Some(tag) = &trigger.xmp {
        if lookup_map(&bag.xmp, tag).is_none() { return false; }
    }
    let needs_node = trigger.node_class.is_some() || trigger.node_title.is_some() || trigger.node_title_contains.is_some();
    if needs_node {
        let Some(nodes) = nodes_for_chunk(bag, chunk) else { return false; };
        if !nodes.values().any(|node| node_matches(node, trigger.node_class.as_deref(), trigger.node_title.as_deref(), trigger.node_title_contains.as_deref())) {
            return false;
        }
    }
    // A trigger with only empty fields never matches.
    trigger.text_key.is_some() || trigger.exif.is_some() || trigger.xmp.is_some() || needs_node
}

/// Match exact key, or `prefix:Name` / bare `Name` (case-insensitive).
fn lookup_map<'a>(map: &'a BTreeMap<String, String>, needle: &str) -> Option<&'a str> {
    if let Some(value) = map.get(needle) {
        return Some(value.as_str());
    }
    let needle_l = needle.to_ascii_lowercase();
    for (key, value) in map {
        if key.eq_ignore_ascii_case(needle) {
            return Some(value.as_str());
        }
        if let Some((_, local)) = key.rsplit_once(':') {
            if local.eq_ignore_ascii_case(needle) || key.to_ascii_lowercase() == needle_l {
                return Some(value.as_str());
            }
        }
    }
    None
}

fn nodes_for_chunk<'a>(bag: &'a ImageMetaBag, chunk: &str) -> Option<&'a serde_json::Map<String, Value>> {
    if chunk == "prompt" { return bag.prompt_nodes.as_ref(); }
    let text = bag.text.get(chunk)?;
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Object(_) => bag.prompt_nodes.as_ref(), // only prompt is cached; fall through
        _ => None,
    }
}

fn node_matches(node: &Value, class: Option<&str>, title: Option<&str>, title_contains: Option<&str>) -> bool {
    if let Some(class) = class {
        if node.get("class_type").and_then(Value::as_str) != Some(class) { return false; }
    }
    let node_title = node.get("_meta").and_then(|m| m.get("title")).and_then(Value::as_str).unwrap_or("");
    if let Some(title) = title {
        if node_title != title { return false; }
    }
    if let Some(part) = title_contains {
        if !node_title.contains(part) { return false; }
    }
    true
}

fn apply_parser(parser: &ParserConfig, bag: &ImageMetaBag, fields: &mut Vec<(String, String)>) {
    for rule in &parser.field {
        if let Some(value) = &rule.constant {
            fields.push((rule.label.clone(), value.clone()));
            continue;
        }
        if let Some(tag) = &rule.exif {
            if let Some(value) = lookup_map(&bag.exif, tag) {
                fields.push((rule.label.clone(), value.to_string()));
            }
            continue;
        }
        if let Some(tag) = &rule.xmp {
            if let Some(value) = lookup_map(&bag.xmp, tag) {
                fields.push((rule.label.clone(), value.to_string()));
            }
            continue;
        }
        let Some(nodes) = nodes_for_chunk(bag, &parser.chunk) else { continue; };
        for node in nodes.values() {
            if !node_matches(node, rule.node_class.as_deref(), rule.node_title.as_deref(), rule.node_title_contains.as_deref()) {
                continue;
            }
            let Some(inputs) = node.get("inputs").and_then(Value::as_object) else { continue; };
            if let Some(extract) = &rule.extract {
                for value in run_extract(extract, inputs) {
                    fields.push((rule.label.clone(), value));
                }
            } else if let Some(input) = &rule.input {
                if let Some(value) = input_text(inputs, input) {
                    fields.push((rule.label.clone(), value));
                }
            }
        }
    }

    if parser.builtin.as_deref() == Some("comfy_prompt") {
        if let Some(prompt) = bag.text.get(&parser.chunk) {
            comfy_prompt_fields(prompt, fields);
        }
    }
}

fn run_extract(kind: &str, inputs: &serde_json::Map<String, Value>) -> Vec<String> {
    match kind {
        "lora_loader" => {
            let Some(name) = input_text(inputs, "lora_name") else { return vec![]; };
            let strength = input_text(inputs, "strength_model").unwrap_or_else(|| "?".into());
            vec![format!("{name} (weight {strength}; active)")]
        }
        "lora_manager" => {
            let Some(loras) = inputs.get("loras") else { return vec![]; };
            let Some(entries) = loras.as_array().or_else(|| loras.get("__value__").and_then(Value::as_array)) else { return vec![]; };
            entries.iter().filter_map(|lora| {
                let name = lora.get("name").and_then(Value::as_str)?;
                let strength = lora.get("strength").map(value_to_string).unwrap_or_else(|| "?".into());
                let clip = lora.get("clipStrength").map(value_to_string).unwrap_or_else(|| strength.clone());
                let active = lora.get("active").and_then(Value::as_bool).unwrap_or(true);
                let state = if active { "active" } else { "inactive" };
                Some(format!("{name} (model {strength}, CLIP {clip}; {state})"))
            }).collect()
        }
        "power_lora" => {
            let mut slots: Vec<_> = inputs.iter()
                .filter(|(key, value)| key.starts_with("lora_") && value.as_object().is_some_and(|slot| slot.contains_key("lora")))
                .collect();
            slots.sort_by(|(a, _), (b, _)| a.cmp(b));
            slots.into_iter().filter_map(|(_, value)| {
                let slot = value.as_object()?;
                let name = slot.get("lora").and_then(Value::as_str)?;
                let strength = slot.get("strength").map(value_to_string).unwrap_or_else(|| "?".into());
                let active = slot.get("on").and_then(Value::as_bool).unwrap_or(true);
                let state = if active { "active" } else { "inactive" };
                Some(format!("{name} (weight {strength}; {state})"))
            }).collect()
        }
        "lora_stack" => {
            let mut out = Vec::new();
            for index in 1..=8 {
                let key = format!("lora_{index:02}");
                let strength_key = format!("strength_{index:02}");
                let Some(name) = input_text(inputs, &key) else { continue; };
                if name.eq_ignore_ascii_case("None") || name.is_empty() { continue; }
                let strength = input_text(inputs, &strength_key).unwrap_or_else(|| "?".into());
                out.push(format!("{name} (weight {strength}; active)"));
            }
            out
        }
        // Krea2RegionalBuilder: LoRAs from regions_data[].loras and inline <lora:…:…> tags.
        "krea2_regional_loras" => krea2_regional_loras(inputs),
        // Krea2RegionalBuilder: one row per region description.
        "krea2_regional_regions" => krea2_regional_regions(inputs),
        _ => vec![],
    }
}

/// Parse `<lora:name:strength>` tags from free text (ComfyUI / Krea convention).
fn parse_inline_lora_tags(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 7 < bytes.len() {
        if !text[i..].starts_with("<lora:") {
            i += 1;
            continue;
        }
        let start = i + "<lora:".len();
        let Some(end_rel) = text[start..].find('>') else { break; };
        let inner = &text[start..start + end_rel];
        i = start + end_rel + 1;
        let (name, strength) = match inner.rsplit_once(':') {
            Some((name, strength)) => (name.trim(), strength.trim()),
            None => (inner.trim(), "1"),
        };
        if !name.is_empty() {
            out.push((name.to_string(), strength.to_string()));
        }
    }
    out
}

fn push_unique_lora(
    out: &mut Vec<String>,
    seen: &mut std::collections::BTreeSet<String>,
    name: &str,
    strength: &str,
) {
    let key = name.to_ascii_lowercase();
    if name.is_empty() || !seen.insert(key) {
        return;
    }
    out.push(format!("{name} (weight {strength}; active)"));
}

fn krea2_regional_loras(inputs: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if let Some(raw) = input_text(inputs, "regions_data") {
        if let Ok(data) = serde_json::from_str::<Value>(&raw) {
            if let Some(regions) = data.get("regions").and_then(Value::as_array) {
                for region in regions {
                    if let Some(loras) = region.get("loras").and_then(Value::as_array) {
                        for lora in loras {
                            let name = lora.get("name").and_then(Value::as_str).unwrap_or("");
                            let strength = lora
                                .get("strength")
                                .map(value_to_string)
                                .unwrap_or_else(|| "?".into());
                            push_unique_lora(&mut out, &mut seen, name, &strength);
                        }
                    }
                    if let Some(desc) = region.get("desc").and_then(Value::as_str) {
                        for (name, strength) in parse_inline_lora_tags(desc) {
                            push_unique_lora(&mut out, &mut seen, &name, &strength);
                        }
                    }
                }
            }
        }
    }

    if let Some(base) = input_text(inputs, "base_prompt") {
        for (name, strength) in parse_inline_lora_tags(&base) {
            push_unique_lora(&mut out, &mut seen, &name, &strength);
        }
    }

    out
}

fn krea2_regional_regions(inputs: &serde_json::Map<String, Value>) -> Vec<String> {
    let Some(raw) = input_text(inputs, "regions_data") else {
        return vec![];
    };
    let Ok(data) = serde_json::from_str::<Value>(&raw) else {
        return vec![];
    };
    let Some(regions) = data.get("regions").and_then(Value::as_array) else {
        return vec![];
    };
    regions
        .iter()
        .enumerate()
        .filter_map(|(index, region)| {
            let desc = region.get("desc").and_then(Value::as_str)?.trim();
            if desc.is_empty() {
                return None;
            }
            Some(format!("{}. {desc}", index + 1))
        })
        .collect()
}

fn comfy_prompt_fields(prompt: &str, fields: &mut Vec<(String, String)>) {
    let Ok(nodes) = serde_json::from_str::<Value>(prompt) else {
        fields.push(("ComfyUI prompt".into(), prompt.into()));
        return;
    };
    let Some(map) = nodes.as_object() else { return; };

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
                if let Some(v) = input_text(inputs, "ckpt_name").or_else(|| input_text(inputs, "unet_name")) {
                    fields.push(("Model".into(), v));
                }
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
            _ if class.to_ascii_lowercase().contains("lora") => {
                for value in run_extract("lora_loader", inputs)
                    .into_iter()
                    .chain(run_extract("lora_manager", inputs))
                    .chain(run_extract("power_lora", inputs))
                    .chain(run_extract("lora_stack", inputs))
                {
                    fields.push(("LoRA".into(), value));
                }
            }
            _ => {}
        }
    }

    let positive = fields.iter().find(|(key, _)| key == "Positive prompt").map(|(_, value)| value.clone());
    if let Some(positive) = positive {
        for (key, value) in fields.iter_mut() {
            if key == "Negative prompt" && value == &positive {
                *key = "Negative prompt (same as positive in image metadata)".into();
            }
        }
    }
}

fn input_text(inputs: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    inputs.get(key).map(value_to_string)
}

fn value_to_string(value: &Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string()).trim_matches('"').to_owned()
}

fn linked_node(inputs: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    let value = inputs.get(key)?.as_array()?.first()?;
    value.as_str().map(str::to_owned).or_else(|| value.as_u64().map(|id| id.to_string()))
}

pub fn png_text_chunks(path: &Path) -> Result<Vec<(String, String)>> {
    use std::io::Read;
    let mut bytes = vec![];
    fs::File::open(path)?.read_to_end(&mut bytes)?;
    let mut result = vec![];
    let mut pos = 8usize;
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
            let mut parts = data.splitn(6, |x| *x == 0);
            let key = parts.next().unwrap_or_default();
            let flag = parts.next().unwrap_or_default();
            let _method = parts.next();
            let _language = parts.next();
            let _translated = parts.next();
            let text = parts.next().unwrap_or_default();
            if flag == [0] {
                result.push((String::from_utf8_lossy(key).into_owned(), String::from_utf8_lossy(text).into_owned()));
            }
        }
        pos += 12 + len;
    }
    Ok(result)
}

fn oriented_dimensions(width: u32, height: u32, orientation: u32) -> (u32, u32) {
    match orientation {
        5 | 6 | 7 | 8 => (height, width),
        _ => (width, height),
    }
}

/// EXIF Orientation tag (1–8). Returns 1 when missing or unreadable.
pub fn exif_orientation(path: &Path) -> u32 {
    let Ok(file) = fs::File::open(path) else { return 1; };
    let mut reader = BufReader::new(file);
    let Ok(exif) = ExifReader::new().read_from_container(&mut reader) else { return 1; };
    exif.get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|field| match &field.value {
            ExifValue::Short(values) => values.first().copied().map(u32::from),
            _ => None,
        })
        .unwrap_or(1)
}

fn read_exif_map(path: &Path) -> Result<BTreeMap<String, String>> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let Ok(exif) = ExifReader::new().read_from_container(&mut reader) else {
        return Ok(BTreeMap::new());
    };
    let mut map = BTreeMap::new();
    // Friendly camera tags first (formatted rationals, ISO aliases).
    for (name, tag) in EXIF_TAGS {
        if let Some(field) = exif.get_field(*tag, In::PRIMARY).or_else(|| exif.get_field(*tag, In::THUMBNAIL)) {
            if let Some(text) = format_exif_value(*tag, &field.value) {
                map.insert((*name).into(), text);
            }
        }
    }
    // ISO often lives only under the Exif IFD; kamadak exposes it via PRIMARY when present.
    if !map.contains_key("PhotographicSensitivity") {
        if let Some(field) = exif.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
            if let Some(text) = format_exif_value(Tag::PhotographicSensitivity, &field.value) {
                map.insert("PhotographicSensitivity".into(), text.clone());
                map.entry("ISO".into()).or_insert(text);
            }
        }
    }
    if let Some(iso) = map.get("PhotographicSensitivity").cloned() {
        map.entry("ISO".into()).or_insert(iso);
    }
    // Dump every EXIF field so Full meta shows more than the camera shortlist.
    for field in exif.fields() {
        let name = field.tag.to_string();
        if map.contains_key(&name) {
            continue;
        }
        // XP* tags are UTF-16LE blobs — prefer that decode over kamadak's raw dump.
        if let Some(text) = decode_xp_string(&field.value)
            .or_else(|| format_exif_value(field.tag, &field.value))
        {
            map.insert(name, text);
        }
    }
    // Windows XP keyword tags are often `Tag(0x9C9E)` — expose friendly aliases.
    for (code, alias) in [
        (0x9C9B_u16, "XPTitle"),
        (0x9C9C, "XPComment"),
        (0x9C9D, "XPAuthor"),
        (0x9C9E, "XPKeywords"),
        (0x9C9F, "XPSubject"),
    ] {
        let raw_key = format!("Tag(0x{code:04X})");
        if let Some(value) = map.get(&raw_key).cloned() {
            map.entry(alias.into()).or_insert(value);
        }
    }
    Ok(map)
}

/// Windows XP EXIF string tags store UTF-16LE bytes.
fn decode_xp_string(value: &ExifValue) -> Option<String> {
    let bytes: Vec<u8> = match value {
        ExifValue::Byte(items) => items.clone(),
        ExifValue::Undefined(items, _) => items.clone(),
        _ => return None,
    };
    if bytes.len() < 2 {
        return None;
    }
    let u16s: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    let text = String::from_utf16(&u16s).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

const EXIF_TAGS: &[(&str, Tag)] = &[
    ("Make", Tag::Make),
    ("Model", Tag::Model),
    ("LensModel", Tag::LensModel),
    ("LensMake", Tag::LensMake),
    ("ExposureTime", Tag::ExposureTime),
    ("FNumber", Tag::FNumber),
    ("PhotographicSensitivity", Tag::PhotographicSensitivity),
    ("ISOSpeedRatings", Tag::PhotographicSensitivity),
    ("FocalLength", Tag::FocalLength),
    ("DateTimeOriginal", Tag::DateTimeOriginal),
    ("DateTime", Tag::DateTime),
    ("Orientation", Tag::Orientation),
];

fn format_exif_value(tag: Tag, value: &ExifValue) -> Option<String> {
    match value {
        ExifValue::Ascii(items) => {
            let text = items.iter().flat_map(|b| std::str::from_utf8(b).ok()).collect::<Vec<_>>().join(" ");
            let text = text.trim().trim_end_matches('\0').to_owned();
            (!text.is_empty()).then_some(text)
        }
        ExifValue::Rational(items) if !items.is_empty() => {
            Some(format_exif_rational(tag, items[0].num, items[0].denom))
        }
        ExifValue::SRational(items) if !items.is_empty() => Some(format_exif_rational(
            tag,
            items[0].num.unsigned_abs() as u32,
            items[0].denom.unsigned_abs().max(1),
        )),
        ExifValue::Short(items) if !items.is_empty() => Some(items[0].to_string()),
        ExifValue::Long(items) if !items.is_empty() => Some(items[0].to_string()),
        _ => {
            let display = value.display_as(tag).to_string();
            let display = display.trim().to_owned();
            (!display.is_empty()).then_some(display)
        }
    }
}

/// Format EXIF rationals for humans: f-stops, shutter 1/N, focal length in mm.
fn format_exif_rational(tag: Tag, num: u32, denom: u32) -> String {
    if denom == 0 {
        return num.to_string();
    }
    let value = num as f64 / denom as f64;
    match tag {
        Tag::FNumber => format_fnumber(value),
        Tag::ExposureTime => format_exposure_time(value),
        Tag::FocalLength => format_focal_mm(value),
        _ => format_generic_rational(num, denom),
    }
}

fn format_fnumber(v: f64) -> String {
    if !v.is_finite() || v <= 0.0 {
        return "?".into();
    }
    if (v * 10.0).round() % 10.0 == 0.0 {
        format!("f/{:.0}", v)
    } else {
        format!("f/{:.1}", v)
    }
}

fn format_exposure_time(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "?".into();
    }
    if seconds >= 1.0 {
        if (seconds - seconds.round()).abs() < 0.05 {
            format!("{} s", seconds.round() as i64)
        } else {
            format!("{seconds:.1} s")
        }
    } else {
        // Phones often store awkward fractions (e.g. 77339/1e8); show classic 1/N.
        let inv = (1.0 / seconds).round().max(1.0);
        format!("1/{inv:.0}")
    }
}

fn format_focal_mm(v: f64) -> String {
    if !v.is_finite() || v <= 0.0 {
        return "?".into();
    }
    if (v * 10.0).round() % 10.0 == 0.0 {
        format!("{:.0} mm", v)
    } else {
        format!("{:.1} mm", v)
    }
}

fn format_generic_rational(num: u32, denom: u32) -> String {
    if denom == 0 {
        return num.to_string();
    }
    if num >= denom && num % denom == 0 {
        return (num / denom).to_string();
    }
    let g = gcd(num, denom);
    let n = num / g;
    let d = denom / g;
    if d == 1 {
        n.to_string()
    } else {
        format!("{n}/{d}")
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsers() -> Vec<ParserConfig> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("parsers");
        load_parsers(&dir).expect("load parsers")
    }

    fn example(path: &str) -> Option<PathBuf> {
        let full = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples").join(path);
        full.is_file().then_some(full)
    }

    fn parser_name(path: &str) -> Option<String> {
        let full = example(path)?;
        read_metadata(&full, &parsers()).ok()?.parser
    }

    #[test]
    fn selects_expected_parsers() {
        let Some(_) = example("Anima_00175_.png") else { return; };
        assert_eq!(parser_name("Krea2_2026-07-28_162915_00001_.png").as_deref(), Some("krea-ai"));
        assert_eq!(parser_name("Anima_00175_.png").as_deref(), Some("anima-ai"));
        assert_eq!(parser_name("Z-Image_00069_.png").as_deref(), Some("z-image"));
        assert_eq!(parser_name("f2__00014_.png").as_deref(), Some("flux2"));
        assert_eq!(parser_name("ComfyUI_00002_.png").as_deref(), Some("comfyui-generic"));
        assert_eq!(parser_name("Buer_IMG_4380.jpeg").as_deref(), Some("photo"));
    }

    #[test]
    fn extracts_key_fields() {
        let Some(full) = example("Krea2_2026-07-28_162915_00001_.png") else { return; };
        let meta = read_metadata(&full, &parsers()).unwrap();
        assert!(meta.fields.iter().any(|(k, v)| k == "Type" && v.contains("Krea")));
        assert!(meta.fields.iter().any(|(k, v)| k == "LoRA" && v.contains("Detailer-KREA2")));
        assert!(meta.fields.iter().any(|(k, _)| k == "Positive Prompt"));

        let Some(photo) = example("Buer_IMG_4380.jpeg") else { return; };
        let meta = read_metadata(&photo, &parsers()).unwrap();
        assert!(meta.fields.iter().any(|(k, v)| k == "Kamera" && v.contains("EOS R8")));
        assert!(meta.fields.iter().any(|(k, _)| k == "ISO"));
    }

    #[test]
    fn parses_k2regional_sample_when_present() {
        let path = PathBuf::from(r"F:\sync\Pictures\test\Krea2_2026-11-03_181903_00001_.png");
        if !path.is_file() {
            return;
        }
        let meta = read_metadata(&path, &parsers()).unwrap();
        assert_eq!(meta.parser.as_deref(), Some("k2regional"));
        assert!(meta.fields.iter().any(|(k, v)| k == "Type" && v.contains("Regional")));
        assert!(meta.fields.iter().any(|(k, _)| k == "Positive Prompt"));
        assert!(meta.fields.iter().any(|(k, v)| k == "LoRA" && v.contains("4n4st4s14k2")));
        assert!(meta.fields.iter().any(|(k, v)| k == "LoRA" && v.contains("Detailer-KREA2")));
        assert!(meta.fields.iter().any(|(k, v)| k == "Region" && v.starts_with("1. ")));
    }

    #[test]
    fn formats_photo_exif_rationals() {
        assert_eq!(format_exif_rational(Tag::FNumber, 28, 5), "f/5.6");
        assert_eq!(format_exif_rational(Tag::FNumber, 17, 10), "f/1.7");
        assert_eq!(format_exif_rational(Tag::FNumber, 8, 1), "f/8");
        assert_eq!(format_exif_rational(Tag::ExposureTime, 1, 400), "1/400");
        assert_eq!(format_exif_rational(Tag::ExposureTime, 77339, 100_000_000), "1/1293");
        assert_eq!(format_exif_rational(Tag::FocalLength, 200, 1), "200 mm");
        assert_eq!(format_exif_rational(Tag::FocalLength, 63, 10), "6.3 mm");
    }

    #[test]
    fn formats_canon_and_samsung_samples_when_present() {
        let album = PathBuf::from(r"F:\sync\Pictures\Album\2026\291027 Vilsalpsee");
        let canon = album.join("IMG_3223.JPG");
        let samsung = album.join("20261027_125134.jpg");
        if !canon.is_file() || !samsung.is_file() {
            return;
        }
        let parsers = parsers();
        let canon_meta = read_metadata(&canon, &parsers).unwrap();
        assert_eq!(field(&canon_meta, "Aperture").as_deref(), Some("f/5.6"));
        assert_eq!(field(&canon_meta, "Shutter Speed").as_deref(), Some("1/400"));
        assert_eq!(field(&canon_meta, "Focal Length").as_deref(), Some("200 mm"));

        let phone_meta = read_metadata(&samsung, &parsers).unwrap();
        assert_eq!(field(&phone_meta, "Aperture").as_deref(), Some("f/1.7"));
        assert_eq!(field(&phone_meta, "Shutter Speed").as_deref(), Some("1/1293"));
        assert_eq!(field(&phone_meta, "Focal Length").as_deref(), Some("6.3 mm"));
    }

    fn field(meta: &Metadata, label: &str) -> Option<String> {
        meta.fields
            .iter()
            .find(|(key, _)| key == label)
            .map(|(_, value)| value.clone())
    }
}
