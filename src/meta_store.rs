//! Metadata cache backends: per-folder `.metadata` JSON files, or a central SQLite DB.
//!
//! DB records are keyed by a path-independent id derived from file name + mtime + size,
//! so moving an image between folders keeps its cached metadata.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::meta_parsers::{
    exif_orientation, exif_to_quarter_turns, load_cached_image_meta_file, read_metadata,
    source_stamp, write_cached_image_meta_file, write_image_metadata_cache_file, CachedImageMeta,
    ParserConfig, METADATA_CACHE_VERSION,
};

pub const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataMode {
    Files,
    Db,
}

impl MetadataMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "db" | "database" | "sqlite" => Self::Db,
            _ => Self::Files,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Db => "db",
        }
    }
}

/// Default SQLite path: `%LOCALAPPDATA%\comruvi\metadata.sqlite` on Windows,
/// `~/.local/share/comruvi/metadata.sqlite` elsewhere.
pub fn default_metadata_db_path() -> PathBuf {
    local_comruvi_dir().join("metadata.sqlite")
}

/// Default central thumbnail root: `%LOCALAPPDATA%\comruvi\thumbnails\` on Windows.
pub fn default_thumbnail_folder() -> PathBuf {
    local_comruvi_dir().join("thumbnails")
}

fn local_comruvi_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("comruvi");
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("comruvi")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailMode {
    /// Per-folder `.thumbnails/<filename>.png` (Geeqie-compatible).
    Distributed,
    /// Central tree: `<root>/<aa>/<bb>/<id>.png` using the metadata content id.
    Central,
}

impl ThumbnailMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            // Accept the common typo from docs/config experiments.
            "central" | "center" | "shared" => Self::Central,
            "distributed" | "ditributed" | "local" | "folder" => Self::Distributed,
            _ => Self::Distributed,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Distributed => "distributed",
            Self::Central => "central",
        }
    }
}

/// Resolve the on-disk thumbnail path for `image` under the chosen storage mode.
pub fn thumbnail_path_for(image: &Path, mode: ThumbnailMode, central_root: &Path) -> PathBuf {
    match mode {
        ThumbnailMode::Distributed => {
            let name = image
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".into());
            image
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(".thumbnails")
                .join(format!("{name}.png"))
        }
        ThumbnailMode::Central => {
            let file_name = image
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".into());
            let (mtime, size) = source_stamp(image).unwrap_or((0, 0));
            let id = image_record_id(&file_name, mtime, size);
            // Two-level fan-out: ab/cd/<full-id>.png
            let a = id.get(0..2).unwrap_or("00");
            let b = id.get(2..4).unwrap_or("00");
            central_root.join(a).join(b).join(format!("{id}.png"))
        }
    }
}

/// Stable id: SHA-256 hex of `file_name \\0 mtime_le \\0 size_le` (no directory).
pub fn image_record_id(file_name: &str, mtime: u64, size: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file_name.as_bytes());
    hasher.update([0]);
    hasher.update(mtime.to_le_bytes());
    hasher.update([0]);
    hasher.update(size.to_le_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

fn file_name_of(image: &Path) -> String {
    image
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into())
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

enum Backend {
    Files,
    Db { conn: Mutex<Connection> },
}

/// Process-wide metadata cache (files or SQLite).
pub struct MetaStore {
    backend: Backend,
    mode: MetadataMode,
    db_path: PathBuf,
}

impl MetaStore {
    pub fn open(mode: MetadataMode, db_path: PathBuf) -> Result<Self> {
        let backend = match mode {
            MetadataMode::Files => Backend::Files,
            MetadataMode::Db => {
                if let Some(parent) = db_path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                let conn = Connection::open(&db_path)
                    .with_context(|| format!("opening metadata db {}", db_path.display()))?;
                conn.execute_batch(
                    "
                    PRAGMA journal_mode=WAL;
                    PRAGMA foreign_keys=ON;
                    ",
                )?;
                migrate(&conn)?;
                Backend::Db {
                    conn: Mutex::new(conn),
                }
            }
        };
        Ok(Self {
            backend,
            mode,
            db_path,
        })
    }

    pub fn mode(&self) -> MetadataMode {
        self.mode
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn is_stale(&self, image: &Path) -> bool {
        match &self.backend {
            Backend::Files => crate::meta_parsers::metadata_cache_is_stale_file(image),
            Backend::Db { conn } => match source_stamp(image) {
                Ok((mtime, size)) => {
                    let id = image_record_id(&file_name_of(image), mtime, size);
                    let Ok(conn) = conn.lock() else { return true; };
                    match conn
                        .query_row(
                            "SELECT cache_version, source_mtime, source_size FROM images WHERE id = ?1",
                            params![id],
                            |row| {
                                Ok((
                                    row.get::<_, i64>(0)?,
                                    row.get::<_, i64>(1)? as u64,
                                    row.get::<_, i64>(2)? as u64,
                                ))
                            },
                        )
                        .optional()
                    {
                        Ok(Some((version, cached_mtime, cached_size))) => {
                            version as u32 != METADATA_CACHE_VERSION
                                || cached_mtime != mtime
                                || cached_size != size
                        }
                        _ => true,
                    }
                }
                Err(_) => true,
            },
        }
    }

    pub fn load(&self, image: &Path) -> Option<CachedImageMeta> {
        match &self.backend {
            Backend::Files => load_cached_image_meta_file(image),
            Backend::Db { conn } => {
                let (mtime, size) = source_stamp(image).ok()?;
                let id = image_record_id(&file_name_of(image), mtime, size);
                let conn = conn.lock().ok()?;
                load_db_row(&conn, &id)
            }
        }
    }

    pub fn write(&self, image: &Path, cached: &CachedImageMeta) -> Result<()> {
        match &self.backend {
            Backend::Files => {
                write_cached_image_meta_file(image, cached)?;
                Ok(())
            }
            Backend::Db { conn } => {
                let conn = conn.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;
                upsert_db_row(&conn, image, cached)
            }
        }
    }

    /// Parse translators and persist cache for `image`.
    pub fn write_image_cache(&self, image: &Path, parsers: &[ParserConfig]) -> Result<()> {
        match &self.backend {
            Backend::Files => {
                write_image_metadata_cache_file(image, parsers)?;
                Ok(())
            }
            Backend::Db { .. } => {
                let (mtime, size) = source_stamp(image)?;
                let orientation = self
                    .load(image)
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
                };
                self.write(image, &cached)
            }
        }
    }

    pub fn display_orientation(&self, image: &Path) -> u8 {
        if let Some(cached) = self.load(image) {
            if cached.version >= METADATA_CACHE_VERSION {
                return cached.orientation % 4;
            }
        }
        exif_to_quarter_turns(exif_orientation(image))
    }

    pub fn set_orientation(&self, image: &Path, orientation: u8) -> Result<()> {
        let orientation = orientation % 4;
        let (mtime, size) = source_stamp(image)?;
        if let Some(mut cached) = self.load(image) {
            cached.version = METADATA_CACHE_VERSION;
            cached.source_mtime = mtime;
            cached.source_size = size;
            cached.orientation = orientation;
            self.write(image, &cached)
        } else {
            let cached = CachedImageMeta {
                version: METADATA_CACHE_VERSION,
                source_mtime: mtime,
                source_size: size,
                parser: None,
                fields: Vec::new(),
                positive: None,
                negative: None,
                loras: Vec::new(),
                loras_active: Vec::new(),
                orientation,
            };
            self.write(image, &cached)
        }
    }

    pub fn matches_volltext(&self, image: &Path, file_name: &str, needle_lower: &str) -> bool {
        if needle_lower.is_empty() {
            return true;
        }
        if file_name.to_ascii_lowercase().contains(needle_lower) {
            return true;
        }
        match &self.backend {
            Backend::Files => self
                .load(image)
                .map(|cached| cached.search_blob_lower().contains(needle_lower))
                .unwrap_or(false),
            Backend::Db { conn } => {
                let Ok((mtime, size)) = source_stamp(image) else {
                    return false;
                };
                let id = image_record_id(&file_name_of(image), mtime, size);
                let Ok(conn) = conn.lock() else {
                    return false;
                };
                conn.query_row(
                    "SELECT 1 FROM images WHERE id = ?1 AND instr(search_blob, ?2) > 0",
                    params![id, needle_lower],
                    |_| Ok(()),
                )
                .optional()
                .ok()
                .flatten()
                .is_some()
            }
        }
    }

    pub fn has_active_lora(&self, image: &Path, lora_name: &str) -> bool {
        match &self.backend {
            Backend::Files => {
                let Some(cached) = self.load(image) else {
                    return false;
                };
                cached
                    .active_lora_names()
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(lora_name))
            }
            Backend::Db { conn } => {
                let Ok((mtime, size)) = source_stamp(image) else {
                    return false;
                };
                let id = image_record_id(&file_name_of(image), mtime, size);
                let Ok(conn) = conn.lock() else {
                    return false;
                };
                conn.query_row(
                    "SELECT 1 FROM image_loras WHERE image_id = ?1 AND active = 1 AND lower(name) = lower(?2)",
                    params![id, lora_name],
                    |_| Ok(()),
                )
                .optional()
                .ok()
                .flatten()
                .is_some()
            }
        }
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meta_schema (
            version INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS images (
            id TEXT PRIMARY KEY NOT NULL,
            file_name TEXT NOT NULL,
            source_mtime INTEGER NOT NULL,
            source_size INTEGER NOT NULL,
            cache_version INTEGER NOT NULL,
            parser TEXT,
            positive TEXT,
            negative TEXT,
            orientation INTEGER NOT NULL DEFAULT 0,
            fields_json TEXT NOT NULL,
            search_blob TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_images_file_name ON images(file_name);
        CREATE INDEX IF NOT EXISTS idx_images_stamp ON images(file_name, source_mtime, source_size);
        CREATE TABLE IF NOT EXISTS image_loras (
            image_id TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY (image_id, name)
        );
        CREATE INDEX IF NOT EXISTS idx_loras_active_name ON image_loras(active, name);
        ",
    )?;
    let version: Option<i32> = conn
        .query_row("SELECT version FROM meta_schema LIMIT 1", [], |row| row.get(0))
        .optional()?;
    if version.is_none() {
        conn.execute(
            "INSERT INTO meta_schema (version) VALUES (?1)",
            params![SCHEMA_VERSION],
        )?;
    }
    Ok(())
}

fn load_db_row(conn: &Connection, id: &str) -> Option<CachedImageMeta> {
    let row = conn
        .query_row(
            "SELECT cache_version, source_mtime, source_size, parser, positive, negative,
                    orientation, fields_json
             FROM images WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u32,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)? as u8,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .ok()??;

    let (version, source_mtime, source_size, parser, positive, negative, orientation, fields_json) =
        row;
    let fields: Vec<(String, String)> = serde_json::from_str(&fields_json).ok()?;

    let mut loras = Vec::new();
    let mut loras_active = Vec::new();
    let mut stmt = conn
        .prepare("SELECT name, active FROM image_loras WHERE image_id = ?1")
        .ok()?;
    let rows = stmt
        .query_map(params![id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .ok()?;
    for item in rows.flatten() {
        let (name, active) = item;
        loras.push(name.clone());
        if active != 0 {
            loras_active.push(name);
        }
    }

    Some(CachedImageMeta {
        version,
        source_mtime,
        source_size,
        parser,
        fields,
        positive,
        negative,
        loras,
        loras_active,
        orientation,
    })
}

fn upsert_db_row(conn: &Connection, image: &Path, cached: &CachedImageMeta) -> Result<()> {
    let file_name = file_name_of(image);
    let id = image_record_id(&file_name, cached.source_mtime, cached.source_size);
    let fields_json = serde_json::to_string(&cached.fields).context("serialize fields_json")?;
    let search_blob = cached.search_blob_lower();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO images (
            id, file_name, source_mtime, source_size, cache_version,
            parser, positive, negative, orientation, fields_json, search_blob, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(id) DO UPDATE SET
            file_name=excluded.file_name,
            source_mtime=excluded.source_mtime,
            source_size=excluded.source_size,
            cache_version=excluded.cache_version,
            parser=excluded.parser,
            positive=excluded.positive,
            negative=excluded.negative,
            orientation=excluded.orientation,
            fields_json=excluded.fields_json,
            search_blob=excluded.search_blob,
            updated_at=excluded.updated_at",
        params![
            id,
            file_name,
            cached.source_mtime as i64,
            cached.source_size as i64,
            cached.version as i64,
            cached.parser,
            cached.positive,
            cached.negative,
            cached.orientation as i64,
            fields_json,
            search_blob,
            now_unix(),
        ],
    )?;
    tx.execute("DELETE FROM image_loras WHERE image_id = ?1", params![id])?;
    // Prefer explicit active list; also store inactive-only names from `loras`.
    let mut seen = std::collections::BTreeSet::new();
    for name in &cached.loras_active {
        if seen.insert(name.to_ascii_lowercase()) {
            tx.execute(
                "INSERT INTO image_loras (image_id, name, active) VALUES (?1, ?2, 1)",
                params![id, name],
            )?;
        }
    }
    for name in &cached.loras {
        if seen.insert(name.to_ascii_lowercase()) {
            tx.execute(
                "INSERT INTO image_loras (image_id, name, active) VALUES (?1, ?2, 0)",
                params![id, name],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_id_ignores_directory() {
        let a = image_record_id("photo.png", 100, 2000);
        let b = image_record_id("photo.png", 100, 2000);
        let c = image_record_id("photo.png", 101, 2000);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn central_thumbnail_layout_and_mode_parse() {
        let id = image_record_id("photo.png", 100, 2000);
        let root = PathBuf::from("thumbs");
        let expected = root
            .join(&id[0..2])
            .join(&id[2..4])
            .join(format!("{id}.png"));
        assert_eq!(
            expected.file_name().and_then(|n| n.to_str()),
            Some(format!("{id}.png").as_str())
        );
        assert_eq!(
            expected.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()),
            Some(&id[2..4])
        );
        assert_eq!(ThumbnailMode::parse("ditributed"), ThumbnailMode::Distributed);
        assert_eq!(ThumbnailMode::parse("central"), ThumbnailMode::Central);
    }

    #[test]
    fn sqlite_roundtrip_orientation() {
        let dir = std::env::temp_dir().join(format!("comruvi-meta-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("metadata.sqlite");
        let store = MetaStore::open(MetadataMode::Db, db).unwrap();
        let image = dir.join("moved.png");
        fs::write(&image, b"not-a-real-image").unwrap();
        // source_stamp works on any file
        let (mtime, size) = source_stamp(&image).unwrap();
        let cached = CachedImageMeta {
            version: METADATA_CACHE_VERSION,
            source_mtime: mtime,
            source_size: size,
            parser: Some("test".into()),
            fields: vec![("Positive Prompt".into(), "hello world".into())],
            positive: Some("hello world".into()),
            negative: None,
            loras: vec!["Foo".into()],
            loras_active: vec!["Foo".into()],
            orientation: 2,
        };
        store.write(&image, &cached).unwrap();
        assert!(!store.is_stale(&image));
        assert_eq!(store.display_orientation(&image), 2);
        assert!(store.matches_volltext(&image, "moved.png", "hello"));
        assert!(store.has_active_lora(&image, "foo"));

        // Simulate move to another folder with same name/mtime/size.
        let other = dir.join("other");
        fs::create_dir_all(&other).unwrap();
        let moved = other.join("moved.png");
        fs::rename(&image, &moved).unwrap();
        // Preserve stamp by copying bytes with same content — rename keeps mtime on Windows usually.
        assert!(!store.is_stale(&moved));
        assert_eq!(store.display_orientation(&moved), 2);

        let _ = fs::remove_dir_all(&dir);
    }
}
