# Product Roadmap

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
- [x] Check duplicates by validators (`etag`, `last-modified`, content length) when available.
- [x] Check duplicates by target path / final path.
- [x] Detect duplicates in compact capture window before add.
- [x] Detect duplicates in in-app add flow before add.
- [ ] Add actions: `Resume existing`, `Replace existing`, `Keep both`, `Merge source list`.
- [ ] Decide safe behavior when a duplicate is partial vs finished.

Notes:
- Current gap: same file can be added twice without clear duplicate warning in capture popup and in-app add flow.
- Identity decisions: backend add rejects normalized URL matches and target-path collisions; URL matches now take precedence over path-only matches so the UI does not suggest a rename that the backend would still block.
- Validator decisions: validator matches now require equal content length plus a matching `etag` or `last-modified`, which keeps the rule conservative enough for production while still catching same-file re-adds across wrapper or redirect URL changes.
- Current UI actions: main add dialog now routes duplicates to `Keep both`, `Select existing`, `Resume existing`, `Restart existing`, or `Open folder`; compact capture offers the same contextual flow before submit and can keep both for target-path-only collisions without leaving the pane.
- Partial duplicate policy: paused and resumable partials prefer `Resume existing`; guarded or restart-required partials prefer `Restart existing`; finished duplicates prefer `Open folder`.
- Remaining duplicate-action gap: `Replace existing` and `Merge source list` are still intentionally deferred until destructive semantics and future mirror support are specified.

---

## Phase 5 - Smarter batch import
- [x] Parse plain newline URL lists.
- [x] Add CSV paste support.
- [x] Add TSV paste support.
- [x] Support columns: `url`, `folder`, `filename`, `category`, `start mode`.
- [x] Validate rows before enqueue.
- [x] Show import preview with parsed fields and row errors.
- [x] Allow partial success with per-row failure reporting.
- [x] Allow editing parsed rows before enqueue.
- [ ] Decide Metalink support scope and parser choice.

Notes:
- Current implementation is plain one-URL-per-line and hardcodes category/save behavior in `src/components/BatchDownloadDialog.tsx`.
- Column map: `folder|directory|save path`, `filename|file|name`, `category|type`, `start mode|start|mode`.
- Metalink scope:

---

## Phase 6 - Updater safety
- [x] Keep update channel model stable-only.
- [x] Add skip-this-version state.
- [x] Preserve engine settings across update transitions.
- [x] Record enough update metadata to diff settings / behavior changes.
- [x] Define startup health check after install.
- [x] Define rollback trigger if post-update startup health check fails.
- [x] Show changelog highlights tied to engine behavior changes.

Notes:
- Current updater flow is straightforward and all-or-nothing in `src-tauri/src/app_update.rs`.
- Channel config decision: `EngineSettings.updateChannel` stays `stable`; the app does not expose preview or staging rings.
- Rollback trigger: if the first restart after install comes up on the wrong version or the updated build fails engine bootstrap, VDM marks that target version as skipped until a newer release appears.
- Skip-version storage: persisted in `EngineSettings.skippedUpdateVersion` and filtered in backend update checks.
- Update alerts now show current-to-target version metadata plus up to three prioritized highlights from release notes, with engine/backend items ranked ahead of general notes.
- Update transition metadata: install now writes a pending transition record with source/target versions, release notes, and the pre-install `EngineSettings` snapshot before download begins.
- Startup health check: first restart after install now reports `pending`, `healthy`, `restoredSettings`, `rollbackTriggered`, or `failed`; if the updated build comes up with default engine settings, VDM restores the previous transfer profile before continuing.

---

## Open questions
- [ ] Should runtime checkpoint data stay persisted by default, or only for active downloads?
- [ ] Should duplicate detection prefer validator matches over URL matches when they disagree?
- [ ] What exactly should `Replace existing` mean for finished vs partial downloads?
- [ ] Should `Merge source list` be implemented now or deferred until mirror support lands?
- [x] Should batch import preview allow editing parsed rows before enqueue?
- [x] Should updater expose anything beyond the stable release path?

Notes:
- 

---

## Session handoff
- Next recommended step: decide whether validator-based duplicate matches should auto-link into the same contextual action flow before adding any destructive replace semantics.
- Files most relevant next session: `src/components/BatchDownloadDialog.tsx`, `src/lib/batchImport.ts`, `src/App.tsx`, `src/hooks/useEngineBridge.ts`, `src-tauri/src/app_update.rs`, `src-tauri/src/engine/mod.rs`.
- Decisions that must not be forgotten: skip-version is backend-persisted as a single stable-only value, duplicate URL matches outrank path-only matches, target-path-only collisions now default to `Keep both` while still exposing the existing download action, batch import rows are now editable and revalidated live before enqueue, and updater startup health should not leak a failed install into future stable checks.
- Current thresholds / limits: batch import preview now keeps all parsed rows editable in a scrollable review pane and still limits the failure summary to the first 5 import failures.
- Known bugs / risks: CSV/TSV parsing supports quoted fields but not multiline quoted cells; validator duplicate matching intentionally refuses content-length-only matches and will not fire until the probe has both size and either `etag` or `last-modified`; extension auto-capture now intentionally avoids aggressive image/audio/video hijacking unless a stronger download signal exists.
- Quick summary for next agent: this pass added editable batch-import review with live revalidation, first-class `Keep both` duplicate handling for target-path-only collisions, stable-only updater hardening with rollback-trigger guardrails, and more conservative browser auto-capture for media navigation. Pending work is destructive duplicate semantics and mirror/source merging.