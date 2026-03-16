# VDM

VDM is a Windows-first Tauri v2 download manager built around an IDM-style backend philosophy: reserve disk space early, write into the final file at exact offsets, scale connections only when the host and disk can sustain it, and fall back conservatively when the request shape is not safe for segmentation.

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
- `src/`: React UI for queue management, monitoring, and diagnostics.
- `extensions/vdm-catcher/`: Chrome MV3 capture extension.
- `src/types/generated/backend/`: `ts-rs` generated frontend bindings.
- `new_features.md`: ship tracker for product work.
- `progress.md`: compact technical baseline and next-session context.

## Prerequisites

- Bun
- Rust stable with `clippy` and `rustfmt`
- Visual Studio C++ build tools on Windows
- Microsoft Edge WebView2 runtime
- Chrome or Chromium if you are working on the extension

## Local development

```powershell
bun install
cargo tauri dev
```

Frontend-only iteration is available through `bun run dev`.

## Validation

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test
cargo clippy --manifest-path src-tauri/Cargo.toml --target-dir target-clippy --all-targets --all-features -- -D warnings
bun run lint
bun run test:extension
bun run build
```

Rust test runs are also the authoritative path for refreshing generated backend bindings. If you change a Rust IPC or event payload, commit the updated files under `src/types/generated/backend`.