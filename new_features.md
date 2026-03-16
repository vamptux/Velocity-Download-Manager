# VDM Necessary Features Tracker

Use this file as the active multi-session tracker for unshipped work.

- Mark `[x]` only when code, tests, and user-visible behavior are complete.
- Keep notes short and dated.
- Prefer closing `P0` items before adding new polish work.

## P0 Ship-Critical

- [x] Fast-launch bootstrap and history compaction
  Context: `persistence.rs` stores the full `RegistrySnapshot` as one JSON payload, `operations.rs::get_app_state()` waits for bootstrap, and `App.tsx` only hydrates real state after `engine://bootstrap`; large histories therefore delay first useful UI state.
  Exit: the main shell paints immediately with queue/settings/bootstrap status, active downloads hydrate from a compact startup snapshot, and archived history/details/logs load lazily or in pages instead of blocking launch.

- [x] Lightweight row payloads and detail-on-demand IPC
  Context: `DownloadRecord` currently carries segments, diagnostics, engine logs, and runtime checkpoint data for every row; `get_app_state()` and `downloads://upsert` send that full shape even when the list only needs summary fields.
  Exit: list rows use a compact summary DTO, detail panes fetch or subscribe to heavyweight fields on demand, and high-history profiles no longer pay full IPC/deserialization cost at startup.

- [x] Non-blocking registry and checkpoint persistence
  Context: `persist_registry()` writes blocking `rusqlite` transactions directly from async runtime and operations paths, and those writes currently include volatile checkpoint/probe state alongside durable downloads.
  Exit: registry persistence runs through a coalesced background writer with priority flushes for pause/remove/finish/crash recovery, so resume safety stays intact without injecting SQLite latency into transfer orchestration.

- [x] Slow-peer race-steal hardening and observability
  Context: `scheduler.rs` and `runtime.rs` already launch challenger segments against slow peers, but race launches/winners are lightly surfaced and the restart/persistence path still needs clearer production guard rails.
  Exit: race starts and winners are logged and diagnosed explicitly, checkpoint restore keeps active races truthful across restart, and slow-peer recovery never double-counts or silently hides the losing segment.

## P1 Product Cleanup

- [x] Runtime scheduler and dispatch extraction
  Context: `engine/mod.rs` still carries large runtime-dispatch and rebalancing responsibilities alongside bootstrap, events, persistence, integrity, and limiter wiring, which makes host-budget changes harder to reason about and easier to regress.
  Exit: dispatch planning/rebalancing logic lives in a dedicated scheduler module with smaller helpers and explicit delta-application boundaries, while behavior stays protected by the existing fairness and runtime fixtures.

- [x] Metadata planning pipeline deduplication
  Context: `operations.rs::probe_download()` and `add_download()` repeat the same request sanitization, recent-probe reuse, live-probe fallback, host-profile refresh, guarded-segmentation checks, and filename/size planning.
  Exit: one shared metadata-planning helper feeds both probe and add flows so request-shape policy, warnings, and capability learning cannot drift.

- [x] Probe state-machine extraction
  Context: `probe.rs::probe_download_headers_internal()` still owns HEAD/Range/GET staging, wrapper follow-up requests, app-API fallback, same-URL retries, and warning synthesis in one large loop.
  Exit: metadata discovery is split into smaller typed phases with focused helpers so wrapper-host fixes and range-detection changes can be tested independently.

- [x] Runtime transfer orchestration extraction
  Context: `runtime.rs` still mixes bootstrap refresh, guarded fallback, adaptive refill, slow-peer race management, worker lifecycle, and completion/error reconciliation in one large module.
  Exit: bootstrap, adaptive scheduling, race management, and completion/error reconciliation move into focused runtime modules while preserving current throughput behavior and regression coverage.

- [x] App shell state and action decomposition
  Context: `App.tsx` owns bootstrap subscriptions, dialogs, completion notices, queue actions, selection state, keyboard shortcuts, and refresh logic in one large component with many callbacks.
  Exit: bootstrap/event wiring, transfer actions, and modal state are split into focused hooks/modules so first-paint logic is easier to optimize and list/detail code no longer pays root-level orchestration complexity.

- [x] Truthful stall diagnostics in the UI
  Context: the backend already tracks writer backpressure, host cooldowns, checkpoint pressure, and resume-guard reasons, but the list/details surfaces still make many stalls feel like generic "paused" or "slow" states.
  Exit: downloads can explicitly show disk-pressure, host-throttled, validator-mismatch, and slow-peer-recovery reasons in row/detail UI so users know whether the bottleneck is disk, server policy, or resume safety.

- [x] Segment telemetry needed for adaptive recovery
  Context: the details UI shows segment progress, but not segment throughput, retry health, or last failure reason, which limits both future race-steal logic and support diagnostics when one connection drags a job to 99%.
  Exit: per-segment speed, retry count, and terminal failure reason are tracked in runtime state and surfaced in details/logs with bounded IPC updates.

- [ ] Adaptive retry jitter and throttle-aware stream recovery
  Context: `runtime_transfer.rs` uses deterministic exponential retry delays for HTTP/range retries and a fixed `backoff_base_ms` delay for mid-stream drops, which can synchronize retries across active segments against the same host.
  Exit: range and stream retries include bounded jitter and `Retry-After`-aware behavior where applicable, so parallel workers de-phase naturally under throttling without regressing steady-state throughput on healthy hosts.

- [ ] HTTP client pool lifecycle tightening for high-host sessions
  Context: `http_pool.rs` retains one client per origin in a single global `Mutex<BTreeMap<...>>` with no eviction/aging policy, which is simple but can hold stale hosts indefinitely during long mixed-host sessions.
  Exit: pooled clients keep current reuse behavior while adding lightweight aging or size bounds and low-contention access patterns, preserving connection reuse gains without unbounded pool growth.

- [ ] Protocol label and telemetry normalization for modern transports
  Context: runtime protocol labeling currently maps unknown HTTP versions to `http/1.x`, which can hide transport-level behavior differences in host telemetry and adaptive connection planning as newer transports appear.
  Exit: telemetry protocol labels stay normalized and explicit for modern negotiated versions so host-profile adaptation logic and diagnostics remain truthful across protocol upgrades.

- [ ] Rust 2024 migration and toolchain policy hardening
  Context: `src-tauri/Cargo.toml` remains on edition `2021` and does not declare an explicit `rust-version` or `resolver = "3"` policy, leaving modern Cargo/MSRV behavior implicit.
  Exit: run an edition-2024 migration pass (`cargo fix --edition`) and lock in explicit toolchain policy (`edition = "2024"`, `rust-version`, resolver strategy, CI validation), keeping performance-focused runtime code current with 2026 Rust/Cargo defaults.

- [ ] Edition-2024 semantic audit for lock lifetimes and async temporaries
  Context: Rust 2024 changes temporary drop scope rules (`if let` and tail-expression temporaries), which can subtly change mutex/rwlock guard lifetimes and borrow behavior in high-concurrency scheduling/recovery paths.
  Exit: audit runtime/dispatch/recovery hot paths for temporary-scope assumptions and make guard lifetimes explicit where needed so post-migration behavior is stable and deadlock-resistant.

- [ ] Latest-stable lint hardening pass after toolchain upgrade
  Context: Rust 1.91–1.94 introduced additional and promoted lints/future-compat warnings that can hide correctness drift until later compiler updates if not normalized now.
  Exit: run compiler+clippy on latest stable, triage new warnings into fixed code or explicit policy decisions, and keep the backend on a warning-clean baseline for forward upgrades.

- [ ] Modern std API adoption sweep in parser and queue hot paths
  Context: newer stable APIs (for example `array_windows` and conditional `VecDeque` pops) can reduce boilerplate and tighten bounds/branch behavior in packet/header scanning and bounded nonce/capture queues.
  Exit: replace equivalent legacy patterns in targeted hotspots with stable modern APIs where behavior is unchanged, improving readability and reducing foot-gun surface in critical paths.

- [ ] Toolchain reproducibility and resolver-policy enforcement
  Context: without a pinned Rust toolchain policy and explicit resolver behavior, CI/dev environments can drift and produce inconsistent dependency resolution or diagnostics as Cargo evolves.
  Exit: define and enforce a reproducible toolchain policy (stable channel floor, rust-version, resolver strategy, and CI checks) so performance/stability work remains deterministic across machines.

## P2 Release Baseline

- [x] Comment-density and redundant structure cleanup
  Context: files such as `DownloadDetailsPanel.tsx`, `Sidebar.tsx`, `NewDownloadDialog.tsx`, and `capture_bridge.rs` carry decorative section banners or explanatory scaffolding that no longer protects complex logic.
  Exit: ornamental banners and redundant comments are removed, only protocol/security/invariant comments stay, and module/function naming does the structural work.

## Session Notes

- 2026-03-16: completed items were removed from this tracker to keep only unshipped backlog work.
- 2026-03-16: ephemeral probe-cache slimming and probe transparency/guarded-mode UX were implemented and validated with lint/build checks.
- 2026-03-16: metadata planning was centralized for probe/add flows and row/detail diagnostics now surface disk-pressure and host-throttling reasons more truthfully.
- 2026-03-16: startup snapshot bootstrap and row/detail IPC split were shipped to reduce first-paint and high-history hydration overhead.
- 2026-03-16: registry persistence now runs through a coalesced background writer with flush priority on pause/remove/finish paths, and slow-peer race launches/winners plus checkpoint race pruning are surfaced in runtime logs.
- 2026-03-16: runtime dispatch application/testing was isolated in `runtime_dispatch.rs` and App shell bootstrap wiring plus batch transfer actions were extracted for cleaner UI orchestration.
- 2026-03-16: segment telemetry now persists per-segment throughput, ETA, retry attempts, and terminal failure reason in runtime checkpoint state and is shown in details UI.
- 2026-03-16: runtime error reconciliation/fallback policy was extracted to `runtime_recovery.rs` and slow-peer race/work-steal queue expansion logic moved to `runtime_race.rs` for cleaner transfer orchestration boundaries.
- 2026-03-16: probe metadata discovery was split into typed request-state and attempt phases, and capture-bridge/comment density was reduced by removing decorative scaffolding comments.
