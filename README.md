# Velocity Download Manager (VDM)

Velocity Is a Rust Based Download Manager in rust focused on speed, stability and seamless UX.

## 👻 Features

- ⚡️ Faster Downloads
- 🧩 Queue control and scheduler-ready workflow foundations for managing long download sessions.
- 🛟 Browser capture via a Chrome MV3 extension and local authenticated bridge.
- 🎨 Multiple built-in themes, including Graphite, Midnight, Carbon, Slate, and Dusk.
- 🔱 Free and open source.

## 🌀 Engine highlights

- 📍 Early disk reservation, direct offset writes, and batched I/O to keep transfers stable under load.
- 📍 Adaptive segmentation with work stealing, slow-peer recovery, and conservative single-stream fallback when segmentation is unsafe.
- 📍 SQLite checkpoints, checksum verification, runtime diagnostics, and structured engine logs.
- 📍 Resume and replay guard rails for auth-gated, POST-backed, wrapper-page, unknown-size, and no-range downloads.

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

## Versioning

```powershell
bun run check:version-sync
```

Run that before tagging a release if you changed any version metadata by hand.
