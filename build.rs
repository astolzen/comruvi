//! Single source of truth: repo-root `VERSION` (e.g. `0.0.6`).
//! Emits `COMRUVI_VERSION` and `COMRUVI_CONFIG_VERSION` (`0.06`) for the app,
//! keeps `[package].version` in Cargo.toml in sync, and on Windows embeds
//! `comruvi.ico` into the PE (Explorer / taskbar pin icon).

use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let version_path = manifest_dir.join("VERSION");
    println!("cargo:rerun-if-changed=VERSION");
    println!("cargo:rerun-if-changed=comruvi.ico");

    let version = fs::read_to_string(&version_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", version_path.display()))
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if version.is_empty() {
        panic!("{} is empty", version_path.display());
    }
    if !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        panic!("VERSION must be a dotted numeric version, got {version:?}");
    }

    sync_cargo_toml(manifest_dir, &version);

    let config_version = config_version_from_semver(&version);
    println!("cargo:rustc-env=COMRUVI_VERSION={version}");
    println!("cargo:rustc-env=COMRUVI_CONFIG_VERSION={config_version}");

    embed_windows_icon(manifest_dir);
}

/// `0.0.6` → `0.06` (same rule as installer `Format-ConfigVersion`).
fn config_version_from_semver(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() == 3 {
        format!("{}.{}{}", parts[0], parts[1], parts[2])
    } else {
        version.to_string()
    }
}

fn sync_cargo_toml(manifest_dir: &Path, version: &str) {
    let cargo_path = manifest_dir.join("Cargo.toml");
    let text = fs::read_to_string(&cargo_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", cargo_path.display()));

    let current = text.lines().find_map(|line| {
        line.strip_prefix("version = \"")
            .and_then(|rest| rest.strip_suffix('"'))
            .map(str::to_string)
    });
    match current.as_deref() {
        Some(v) if v == version => return,
        None => panic!("could not find package version = \"...\" in Cargo.toml"),
        Some(_) => {}
    }

    let mut out = String::with_capacity(text.len() + 8);
    let mut replaced = false;
    for line in text.lines() {
        if !replaced && line.starts_with("version = \"") {
            out.push_str("version = \"");
            out.push_str(version);
            out.push('"');
            replaced = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !text.ends_with('\n') {
        out.pop();
    }

    fs::write(&cargo_path, out.as_bytes())
        .unwrap_or_else(|e| panic!("failed to sync {}: {e}", cargo_path.display()));
    println!("cargo:warning=synced Cargo.toml version to {version} from VERSION file");
}

fn embed_windows_icon(manifest_dir: &Path) {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    let icon = manifest_dir.join("comruvi.ico");
    if !icon.is_file() {
        // Bootstrap: `export-icon` creates this file; rebuild picks it up via rerun-if-changed.
        println!(
            "cargo:warning=missing comruvi.ico — run `cargo run --release --bin export-icon` then rebuild"
        );
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("comruvi.ico");
    res.set("FileDescription", "comruvi — ComfyUI Rust Viewer");
    res.set("ProductName", "comruvi");
    res.set("LegalCopyright", "AGPL-3.0-only");
    res.compile()
        .unwrap_or_else(|e| panic!("failed to embed Windows resources: {e}"));
}
