# Velocity Download Manager (VDM)

Velocity Download Manager is a Windows-first desktop downloader built with Rust, Tauri v2, and React. It is designed for high sustained throughput, careful resume handling, and a cleaner modern UI without the usual clutter.

## Features

- Faster multi-connection downloads with host-aware scaling and guarded fallbacks.
- Queue control and scheduler-ready workflow foundations for managing long download sessions.
- Browser capture via a Chrome MV3 extension and local authenticated bridge.
- Multiple built-in themes, including Graphite, Midnight, Carbon, Slate, and Dusk.
- Free and open source.

## Engine highlights

- Early disk reservation, direct offset writes, and batched I/O to keep transfers stable under load.
- Adaptive segmentation with work stealing, slow-peer recovery, and conservative single-stream fallback when segmentation is unsafe.
- SQLite checkpoints, checksum verification, runtime diagnostics, and structured engine logs.
- Resume and replay guard rails for auth-gated, POST-backed, wrapper-page, unknown-size, and no-range downloads.

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
- `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` must stay on the same version; `bun run check:version-sync` enforces that locally and in CI.
- Stable builds check `https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest.json` for updates.
- Preview builds check `https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest-preview.json` first and fall back to the stable manifest when no preview manifest is published.
- Manual update checks now fall back to GitHub release metadata when an updater manifest is missing or delayed, so the app can say "up to date" or "release published but not ready for in-app install yet" instead of surfacing a raw JSON fetch failure.
- The update channel is persisted in engine settings and can be switched in the app under Browser Integration -> App Updates.
- VDM records the pending startup-health transition only when the user chooses to restart and apply the staged update, which avoids falsely marking a downloaded-but-not-yet-applied build as failed.
- Startup maintenance prunes stale updater temp artifacts and legacy app-local update directories so repeated upgrades do not keep growing disk usage.
- `TAURI_SIGNING_PRIVATE_KEY` must contain the full contents of `%USERPROFILE%\.tauri\velocity-download-manager.key`, not the password or a placeholder string.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is only the passphrase for a password-protected key.
- The current local updater key for this repo was generated without a password, so `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` should be omitted in GitHub or left empty.
- The current public updater key is embedded in `src-tauri/tauri.conf.json`.
- The release workflow now hard-fails if `latest.json` or updater signature assets are missing from the tagged GitHub release, which prevents shipping a build that would break in-app verification.

## Versioning

```powershell
bun run check:version-sync
```

Run that before tagging a release if you changed any version metadata by hand.

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