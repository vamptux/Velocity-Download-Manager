# new_features.md

## How to use this file
- Keep entries short and current.
- Change `[ ]` to `[x]` when completed.
- Add notes only where they help the next session.
- Prefer decisions, IDs, schema changes, blockers, and thresholds over logs.
- Scope for this file only: persistence normalization, hot/cold state split, duplicate handling, smarter batch import, updater safety.

## Project status
- Current phase: Phase 4 / Phase 5 / Phase 6 incremental hardening
- Current focus: duplicate prevention, smarter batch import, updater safety state
- Main blocker: persistence normalization is still the larger backend migration
- Last updated by: GitHub Copilot
- Last updated at: 2026-03-22

---

## Phase 1 - Persistence redesign
- [ ] Confirm normalized storage boundaries.
- [ ] Define migration strategy from snapshot DB to normalized tables.
- [ ] Decide whether runtime checkpoint stays persisted or becomes volatile cache.
- [ ] Define rollback behavior if normalized migration fails.

Notes:
- Current snapshot source: `src-tauri/src/engine/persistence.rs`
- Current row/detail split in memory only: `src-tauri/src/engine/mod.rs`, `src-tauri/src/engine/operations.rs`

---

## Phase 2 - Schema and storage primitives
- [ ] Create `settings` table.
- [ ] Create `downloads` table for hot summary state.
- [ ] Create `download_segments` table.
- [ ] Create `host_profiles` table.
- [ ] Create `download_logs` table.
- [ ] Create `runtime_checkpoint` table or finalize volatile-cache path.
- [ ] Add indexes for queue ordering, host lookups, duplicate checks, and detail fetches.
- [ ] Add schema versioning and migrations.

Notes:
- Hot fields expected in `downloads`: status, progress, target path, URL identity, queue position, speed, target/max connections, key diagnostics summary.
- Cold fields expected off-row: logs, detailed checkpoint state, future source history / torrent state.
- Important indexes:

---

## Phase 3 - Engine persistence refactor
- [ ] Replace snapshot load path with normalized bootstrap load.
- [ ] Replace snapshot flush path with row-level writes.
- [ ] Separate write paths for hot updates vs cold detail writes.
- [ ] Keep startup fast for large queues.
- [ ] Keep corruption blast radius limited to affected rows/tables.
- [ ] Preserve existing runtime guarantees for segmented partials.

Notes:
- Must not break segmented partial recovery semantics.
- Must not regress host-profile learning or queue dispatch.

---

## Phase 4 - Duplicate handling
- [ ] Define normalized duplicate identity inputs.
- [x] Check duplicates by normalized URL.
- [ ] Check duplicates by validators (`etag`, `last-modified`, content length) when available.
- [x] Check duplicates by target path / final path.
- [ ] Detect duplicates in compact capture window before add.
- [x] Detect duplicates in in-app add flow before add.
- [ ] Add actions: `Resume existing`, `Replace existing`, `Keep both`, `Merge source list`.
- [ ] Decide safe behavior when a duplicate is partial vs finished.

Notes:
- Current gap: same file can be added twice without clear duplicate warning in capture popup and in-app add flow.
- Identity decisions: backend add now rejects normalized URL matches and target-path collisions; the main add dialog warns before submit.
- Partial duplicate policy:

---

## Phase 5 - Smarter batch import
- [x] Parse plain newline URL lists.
- [x] Add CSV paste support.
- [x] Add TSV paste support.
- [x] Support columns: `url`, `folder`, `filename`, `checksum`, `category`, `start mode`.
- [x] Validate rows before enqueue.
- [x] Show import preview with parsed fields and row errors.
- [x] Allow partial success with per-row failure reporting.
- [ ] Decide Metalink support scope and parser choice.

Notes:
- Current implementation is plain one-URL-per-line and hardcodes category/save behavior in `src/components/BatchDownloadDialog.tsx`.
- Column map: `folder|directory|save path`, `filename|file|name`, `checksum|hash`, `category|type`, `start mode|start|mode`.
- Metalink scope:

---

## Phase 6 - Updater safety
- [ ] Add update channel model: `stable` and `preview`.
- [x] Add skip-this-version state.
- [ ] Preserve engine settings across update transitions.
- [ ] Record enough update metadata to diff settings / behavior changes.
- [ ] Define startup health check after install.
- [ ] Define rollback trigger if post-update startup health check fails.
- [ ] Show changelog highlights tied to engine behavior changes.

Notes:
- Current updater flow is straightforward and all-or-nothing in `src-tauri/src/app_update.rs`.
- Channel config decision:
- Rollback trigger:
- Skip-version storage: persisted in `EngineSettings.skippedUpdateVersion` and filtered in backend update checks.

---

## Open questions
- [ ] Should runtime checkpoint data stay persisted by default, or only for active downloads?
- [ ] Should duplicate detection prefer validator matches over URL matches when they disagree?
- [ ] What exactly should `Replace existing` mean for finished vs partial downloads?
- [ ] Should `Merge source list` be implemented now or deferred until mirror support lands?
- [ ] Should batch import preview allow editing parsed rows before enqueue?
- [ ] Should preview update channel use a separate settings keyspace or share the same one?

Notes:
- 

---

## Session handoff
- Next recommended step: extend duplicate prevention into compact capture and decide replace/resume semantics before allowing duplicate override actions.
- Files most relevant next session: `src-tauri/src/engine/operations.rs`, `src/components/BatchDownloadDialog.tsx`, `src/components/NewDownloadDialog.tsx`, `src/App.tsx`, `src/lib/batchImport.ts`, `src/lib/downloadDuplicates.ts`.
- Decisions that must not be forgotten: skip-version is now backend-persisted, batch import only submits rows with zero validation errors, duplicate detection currently blocks on normalized URL or target path.
- Current thresholds / limits: batch preview renders the first 8 rows inline and the first 5 import failures in the summary.
- Known bugs / risks: CSV/TSV parsing supports quoted fields but not multiline quoted cells; compact capture still depends on backend duplicate rejection rather than proactive list inspection.
- Quick summary for next agent: this pass shipped backend duplicate guards, a proactive duplicate warning in the main add dialog, a parsed batch import preview with partial success reporting, and durable updater skip-version state. Verified with `bun run lint`, `bun run build`, and `cargo clippy --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target-clippy --all-targets --all-features -- -D warnings`.