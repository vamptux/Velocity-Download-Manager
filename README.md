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
- The app checks `https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest.json` for updates.
- `TAURI_SIGNING_PRIVATE_KEY` must contain the full contents of `%USERPROFILE%\.tauri\velocity-download-manager.key`, not the password or a placeholder string.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is only the passphrase for a password-protected key.
- The current local updater key for this repo was generated without a password, so `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` should be omitted in GitHub or left empty.
- The current public updater key is embedded in `src-tauri/tauri.conf.json`.

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