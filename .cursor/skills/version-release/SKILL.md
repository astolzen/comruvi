---
name: version-release
description: >-
  Build, package, document, version-bump, commit, and publish a comruvi GitHub
  Release. Use when the user asks to cut a release, bump the version, publish an
  MSI, or run the version-release workflow for this project.
disable-model-invocation: true
---

# comruvi version-release

Cut a release for **comruvi** from this repository. The user supplies the target
version (for example `0.1.1` or `0.2.0`). Keep the existing numeric `VERSION`
convention (dotted digits only — no `beta` suffix in the file; put channel words
in CHANGELOG / About / release title).

## Preconditions

- Working tree should be on `master` (or the user’s release branch).
- Ask for the **exact version string** if not already given.
- Do **not** change git config. Do **not** force-push. Do **not** skip hooks.

## Steps (in order)

### 1. Set the version

1. Write the version to the repo-root `VERSION` file (single line, e.g. `0.1.1`).
2. Run a cargo build so `build.rs` syncs `Cargo.toml` / `Cargo.lock`.
3. Confirm `CONFIG_VERSION` mapping (`0.1.1` → config `version = 0.11`).

### 2. Compile and check

```powershell
cargo test --bin comruvi
cargo build --release --bin comruvi
```

Fix compile/test failures before continuing.

### 3. Update documentation

1. Move `## Unreleased` notes in `CHANGELOG.md` into a dated section for this
   version (keep the project’s existing changelog style).
2. Update `README.md` version examples (MSI file names, `version = …` in config
   docs, “Current status”) when they still mention the previous release.
3. About dialog already reads `VERSION` via `build.rs`; only adjust channel text
   (e.g. `beta`) if the user asked for it.

### 4. Build the Windows installer

From the repo root:

```powershell
powershell -ExecutionPolicy Bypass -File .\installer\pack-windows.ps1
```

Expect:

- `dist/comruvi-<version>-windows-x64.msi`
- `dist/comruvi-<version>-windows-x64.zip`

`dist/` is gitignored — do not commit installer artifacts.

### 5. Git commit

Stage source/docs/version files only (not `dist/`, not generated WiX temps under
`installer/wix/` that are gitignored).

Commit message style (match recent history):

```text
Release <version>: <one-line summary of headline changes>.
```

Use a PowerShell here-string for the message on Windows (bash heredoc is not
available). Then:

```powershell
git status
git push
```

Only push when the user asked to publish / push / release.

### 6. Publish the GitHub Release

Create the tag and release with assets (prerelease only if the user said alpha/beta):

```powershell
gh release create "v<version>" --title "v<version>" --notes-file - --latest `
  "dist/comruvi-<version>-windows-x64.msi" `
  "dist/comruvi-<version>-windows-x64.zip"
```

Release notes body: summarize the new CHANGELOG section (Added / Fixed / Changed).

Tag naming: prefer `v0.1.1` or `v0.1.0-beta` to match prior tags (`v0.091-alpha`,
`v0.1.0-beta`). Align the tag with what the user requested.

Return the release URL when done.

## Checklist

- [ ] `VERSION` + `Cargo.toml` match
- [ ] `cargo test` / release build OK
- [ ] CHANGELOG + README updated
- [ ] MSI + zip built under `dist/`
- [ ] Commit pushed (if requested)
- [ ] `gh release` published with MSI and zip
