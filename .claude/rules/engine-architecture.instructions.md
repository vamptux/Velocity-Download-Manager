---
description: SOTA High-Performance Download Engine Architecture (IDM / AB-Downloader Style)
---

# VDM High-Performance Engine Architecture

This rule file dictates the core engineering principles for achieving top-tier, SOTA (State OF The Art) download speeds matching engines like IDM, AB-Downloader, and FDM. It incorporates VDM's specific `HostProfile`, `EngineSettings`, and `DownloadSegment` data models.

## 1. Zero-Cost File Pre-Allocation
- **Zero OS Zeroing:** Before downloading begins, read the `Content-Length` (from validation/probes). Instantly allocate the full file size on disk using OS-specific fast allocation methods.
  - *Windows:* `fs2::FileExt::allocate` or ideally `SetFileValidData` (if privileges allow) / sparse files.
  - *Unix/macOS:* `posix_fallocate` via `rustix` or `libc`.
- **Purpose:** Prevents disk fragmentation and avoids the OS bottleneck of writing gigabytes of zeros to disk before real data arrives.

## 2. Lock-Free Concurrent Writes & Zero Final Assembly
- **No Mutex Contentions:** Avoid trapping workers behind a `Mutex<File>` to write chunks. This causes severe thread contention at high speeds.
- **Concurrent APIs:** Instead, use `std::os::windows::fs::FileExt::seek_write` for Windows and `std::os::unix::fs::FileExt::write_at` for Unix/macOS. This allows multiple threads to concurrently stream data into the exact same file descriptor without locks.
- **In-Place Assembly:** Because every thread directly writes to its calculated byte offset in the pre-allocated final file, the exact moment the last byte is downloaded, the file is 100% complete. **Zero final assembly or merging is required.**

## 3. Dynamic Segmentation & Work Stealing Algorithm
The core of IDM/AB-Downloader speeds is uninterrupted dynamic segmentation.
- **Initial Scatter:** Divide the total file into `N` large segments (e.g., `maxActiveDownloads` or `defaultMaxConnections` from `EngineSettings`) and assign workers.
- **Idle Stealing:** When a thread completes its segment, it asks the Segment Scheduler for work. 
- **The Split:** The Scheduler locates the active segment with the largest remaining un-downloaded gap. It modifies the active segment's `end` boundary down, and creates a new `DownloadSegment` from the split point to the old end.
- **Uninterrupted Flow:** The original thread continuously downloads without knowing it was split—it simply hits its new `end` boundary earlier. NEVER interrupt, pause, or reset an active string to split it.

## 4. Hyper-Segmentation Prevention (Edge-Case Handling)
Do not aggressively steal work if the TCP/Http overhead will out-cost the download itself.
- **Minimum Thresholds:** Strictly adhere to `EngineSettings::minSegmentSizeBytes` (e.g., 2MB - 5MB). If the largest gap is smaller than this threshold, **do not steal**. Let the original thread finish it.
- **No Cold Steals:** Do not open completely new TCP connections purely to steal tiny chunks at the end of a file. Only let idle threads that have finished their previous segments (reusing a warmed-up Keep-Alive connection) initiate a steal.
- **Late Segment Ratio:** Observe `lateSegmentRatioPercent` to throttle new connection generation near the final 5% of a file.

## 5. Host Adaptive Concurrency & Rate Limiting
Reference VDM's `HostProfile` to automatically shape traffic and respect server limitations.
- **Monitor TFBB & Throughput:** Track `averageTtfbMs` and `averageThroughputBytesPerSecond`. 
- **Ramp Constraints:** If adding more connections to a host does not increase throughput (track via `rampAttemptsWithoutGain`), set `concurrencyLocked = true` and lock the connection count. 
- **Backoff Profiles:** If `throttleEvents` (HTTP 429) or `timeoutEvents` grow, trigger an exponential backoff. Do not immediately open 8 connections to a host that just served a 429. Use a single warm `reqwest::Client` pool per host.

## 6. Real-World Bottleneck Defeats (Disk I/O & Tokio Blocking)
Achieving 1Gbps+ exposes extreme bottlenecks if network logic mixes with disk I/O.
- **Batched I/O Buffering:** Do not call `seek_write` for every 8KB TCP payload. The sheer volume of Syscalls will cripple CPU performance. Buffer incoming bytes in-memory per-segment (e.g., 1MB to 4MB capacity) and flush to disk in bulk.
- **Async Non-Blocking Writes:** Standard `std::fs` operations are blocking. Doing them directly inside a `tokio::spawn` network loop will starve the Tokio executor. All `seek_write` operations MUST be offloaded via `tokio::task::spawn_blocking` or handled by a dedicated standard thread pool using channels.
- **Antivirus Evasion (Exclusive Locks):** Continuously writing file chunks triggers exhaustive real-time scans from Windows Defender, strangling SSD I/O. Keep an exclusive/shared write lock on the `.vdm` temporary file during download. Only release it upon final completion to trigger the final AV scan.

## 7. Slow-Peer Dropping & Race-Stealing
The most common IDM/Aria2 edge case: the entire download hits 99%, but one thread mysteriously throttles to 10 KB/s, hanging the job.
- **Monitor Thread Health:** Track metrics per segment thread. If a segment's calculated ETA far exceeds the median ETA of other segments, flag it as a "slow peer".
- **Race Condition:** Steal the remaining bytes from the *slow peer* using a fast, idle thread. Let both the slow connection and the new fast connection race to download that exact same byte range. Whichever finishes first commits to disk, and forcefully terminates the loser.

## 8. IPC State Sync & UI Performance
VDM's `TransferSegmentStrip.tsx` heavily relies on accurate byte reporting per segment.
- **Batched Tracking:** A download at 5 Gbps will process thousands of chunks per second. NEVER emit a Tauri IPC event per chunk.
- **Interval Flushes:** Use `segmentCheckpointMinIntervalMs` to batch payload updates (e.g., 100ms-150ms). Update `downloaded` in atomic counters internally, and only read them during the sync interval.

## 9. Resume Resiliency & Hash Validation
- **State Hydration:** Segment progress must be flushed to disk/SQLite rapidly but asynchronously (`EngineSettings::segmentCheckpointMaxIntervalMs`). 
- **Validator Checks:** On resume, send an HTTP HEAD/GET request to match the `ResumeValidators` (`ETag`, `Last-Modified`, `Content-Length`). If the server returns 200 OK without matching validators (meaning the file changed), alert the user or restart the job from byte 0.

## 10. Ultimate OS & Network Polishes
- **Power Management (Wake Locks):** Huge multi-gigabyte downloads are susceptible to the OS entering idle/sleep mode. Strictly acquire OS-level power/wake locks (via `tauri-plugin-power` or Rust crates) the moment a segment starts and release them when all downloads queue finish or pause.
- **TCP Socket Tuning:** Ensure your `reqwest::Client` pool explicitly enables `tcp_nodelay(true)` to disable Nagle's algorithm, reducing packet latency. Utilize `reqwest`'s connection keep-alive timeout matching VDM's `cooldownUntil` logic.
- **UI Render Thrashing Prevention:** Sending huge payloads of 32 active segments 10 times a second will freeze React. Alongside interval batching (`segmentCheckpointMinIntervalMs`), the Tauri IPC should ideally send *diff-based* payloads (only segment IDs that moved). In React, heavily utilize `useMemo` or atomic state (Zustand/Jotai) bound specifically to the localized `Tooltip` and `Bar` to prevent the parent `DownloadRow` from re-rendering every frame.