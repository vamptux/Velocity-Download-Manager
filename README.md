# Velocity Download Manager (VDM)

Velocity Download Manager is a Windows-first Tauri v2 download manager built around an IDM-style backend philosophy: reserve disk space early, write into the final file at exact offsets, scale connections only when the host and disk can sustain it, and fall back conservatively when the request shape is not safe for segmentation.

## Current baseline

- Rust download engine with zero-cost temp-file reservation, concurrent offset writes, batched disk I/O, and guarded finalization.
- Dynamic segmentation, work stealing, slow-peer race steals, and host-adaptive connection planning.
- Conservative replay and resume handling for POST/form-backed, auth-gated, wrapper-page, unknown-size, and no-range downloads.
- SQLite-backed checkpoints, checksum verification, structured engine logs, and host diagnostics surfaced in the UI.
- Chrome MV3 capture extension with response-aware classification and an authenticated loopback bridge.
- Generated Rust-to-frontend IPC bindings committed under `src/types/generated/backend`.

## Platform support

- Primary target: Windows 10/11.
- Secondary platforms may compile, but Windows behavior drives performance tuning, file-allocation policy, and release expectations.

## Repository layout

- `src-tauri/`: Rust engine, persistence, Tauri commands, capture bridge, runtime planning.
- `src/`: React UI for queue management, monitoring, diagnostics, and update surfaces.
- `extensions/vdm-catcher/`: Chrome MV3 capture extension.
- `src/types/generated/backend/`: `ts-rs` generated frontend bindings.

## Prerequisites

- Bun
- Rust stable with `clippy` and `rustfmt`
- Visual Studio C++ build tools on Windows
- Microsoft Edge WebView2 runtime
- Chrome or Chromium if you are working on the extension

## Local development

```powershell
bun install
bun run tauri dev
```

Frontend-only iteration is available through `bun run dev`.

## Windows installer

```powershell
bun run build:windows
```

The signed NSIS installer is emitted under `src-tauri/target/release/bundle/nsis/` when built locally on Windows.

## GitHub releases and auto-updates

- Tauri updater artifacts and `latest.json` are produced by `.github/workflows/release.yml`.
- Push a tag in the form `vX.Y.Z` to publish a Windows installer release on GitHub.
- Stable builds check `https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest.json` for updates.
- Preview builds check `https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest-preview.json` first and fall back to the stable manifest when no preview manifest is published.
- Manual update checks now fall back to GitHub release metadata when an updater manifest is missing or delayed, so the app can say "up to date" or "release published but not ready for in-app install yet" instead of surfacing a raw JSON fetch failure.
- The update channel is persisted in engine settings and can be switched in the app under Browser Integration -> App Updates.
- After install, VDM records a pending startup-health transition. If the first restart boots the wrong version or the updated build fails engine bootstrap, the failed version is marked as skipped for future checks and preview users are moved back to the stable channel.
- `TAURI_SIGNING_PRIVATE_KEY` must contain the full contents of `%USERPROFILE%\.tauri\velocity-download-manager.key`, not the password or a placeholder string.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is only the passphrase for a password-protected key.
- The current local updater key for this repo was generated without a password, so `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` should be omitted in GitHub or left empty.
- The current public updater key is embedded in `src-tauri/tauri.conf.json`.
- The release workflow now hard-fails if `latest.json` is not attached to the tagged GitHub release, which prevents shipping a build that would break the in-app updater.

## Validation

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
bun run build
cargo test --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target-test
cargo clippy --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target-clippy --all-targets --all-features -- -D warnings
bun run lint
bun run test:extension
bun run validate
```

Rust test runs are also the authoritative path for refreshing generated backend bindings. If you change a Rust IPC or event payload, commit the updated files under `src/types/generated/backend`.