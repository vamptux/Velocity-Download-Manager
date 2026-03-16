# VDM Technical Progress Tracker

## Engine Baseline

- Phase 1 I/O is complete: temp files are reserved early, concurrent offset writes are offloaded, and completion waits for disk drain plus sync before finalization.
- Phase 2 networking is complete: browser-like probing, warmed HTTP pools, explicit range proofing, and host telemetry/backoff are all in place.
- Phase 3 segmentation is complete: dynamic splitting, work stealing, slow-peer racing, minimum-segment guards, and disk-pressure suppression are implemented.
- Phase 4 UI sync is complete: throttled diff-based progress events, smoothed rendering, and bounded history rendering keep the interface responsive.
- Phase 5 resiliency is complete: SQLite-backed checkpoints, wake locks, guarded single-stream fallbacks, checksum verification, and resume-validator checks are live.

## Current Product Baseline

- Authenticated Chrome MV3 capture bridge with persisted pairing secret, per-launch nonce signing, and extension-side status reporting.
- Shared extension classification rules with response-aware enrichment and Bun fixture coverage.
- Generated Rust-to-frontend IPC bindings committed under `src/types/generated/backend`.
- Host-fair queue dispatch with same-host connection rebalancing across active downloads.
- Structured engine logs, restart-only UX, and large-history UI guard rails.

## Latest Session

- Pairing now has an explicit first-run path across the VDM settings dialog, the popup, and the extension options page. The popup status flow was also corrected so transient states no longer pass the wrong arguments.
- Added scheduler and integrity regression coverage for three-download same-host fair-share rebalancing and checksum mismatch-reset recovery.
- Added the missing OSS repo surface: README, MIT license, CONTRIBUTING, SECURITY, and Windows-first GitHub Actions CI.
- Validation tooling is now more truthful: `bun run lint` covers both `src/` and `extensions/`, and CI verifies generated backend bindings remain committed after Rust tests.

## Validation Commands

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test
cargo clippy --manifest-path src-tauri/Cargo.toml --target-dir target-clippy --all-targets --all-features -- -D warnings
bun run lint
bun run test:extension
bun run build
```

## Next Session Tasks

1. Extension pairing convenience: investigate a lower-friction handoff, such as clipboard-assisted paste or app-driven deep linking, without weakening the capture-bridge trust model.
2. Scheduler stress coverage: add mixed-host and mixed-capability fixtures around cooldowns, manual-start queue edges, and disk-pressure suppression.
3. Release pipeline: add Windows packaging and release automation that matches the new CI baseline.
4. Public repo polish: add screenshots and release/install notes once the Windows build and packaging story are stable.

