# Data-upload support M2: upload file discovery + readiness

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** MUST be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench4/repos/agent` (the Rust device agent — the only repo edited) | read-write | Add a new background worker `agent/src/workers/uploads.rs` that, for each cached `UploadRule`, watches the filesystem (per-rule `poll_interval_secs` cadence), glob-matches `source.glob`, and decides which matched files are "ready" (quiescent) via `source.stability_window_secs`. Newly-ready files are LOGGED (placeholder sink) and tracked to avoid re-logging. Add a `glob` dependency. Wire the worker into `agent/src/app/run.rs` behind an enable flag, with its `JoinHandle` tracked by the shutdown manager. Add worker unit tests under `agent/tests/workers/uploads.rs`. |

Branch: `feat/uploads-file-discovery`. Base: `main`. This plan lives in `plans/backlog/` per the agent repo's plan conventions (`plans/{active,archived,backlog,completed}/`). Commit all changes from inside the agent repo's own git context (see workbench `CLAUDE.md`), never from the workbench root.

**This plan covers ONLY M2 (file discovery + readiness).** It deliberately ENDS at "for each cached upload rule, the set of currently-ready files is identified and emitted to a placeholder log sink (deduped in memory)." Nothing computes digests, contacts the backend, or persists anything.

### Explicitly OUT OF SCOPE (deferred to M3–M5)

These are intentionally NOT in this plan and MUST NOT be implemented here:

- Streaming sha256 digest + size computation over ready files (M3).
- `POST /uploads` (`createUpload`) + the presigned `PUT` to the customer bucket with `required_headers` (M3).
- `POST /uploads/{upload_id}/confirm` (`confirmUpload`) (M3).
- The local uploads ledger / idempotency / retry / durable already-uploaded state (M4). M2's "already-reported" set is **in-memory only** and resets on restart — that is intentional and sufficient for the log sink.
- `delete_policy` enforcement (deleting local source files) (M5).
- Finalization-marker detection (MCAP footer / parquet magic bytes). M2 marks a code HOOK for it but does NOT implement it; size+mtime stability is the sole readiness signal.
- Any outbound HTTP.

The read path (M0/M1) is already landed: `storage::UploadRules` is populated during sync and persisted at `/var/lib/miru/resources/upload_rules.json`. See `plans/completed/20260626-agent-uploads-read-path.md`. M2 is the first *consumer* of those cached rules.

## Purpose / Big Picture

The agent's full data-upload feature is: discover upload rules from the deployed release (M1, done) → **watch the filesystem for matching, quiesced files (M2, this plan)** → mint a presigned URL and `PUT` the file (M3) → confirm + ledger (M4) → enforce `delete_policy` (M5). Research: `/home/ben/miru/workbench4/research/20260626-agent-data-upload-implementation.md`.

M2 delivers the discovery + readiness slice as a standalone, independently-verifiable background worker that mirrors the existing `poller` worker's shape (`Options` + injected `sleep_fn` + `run_impl` loop + `tokio::select!` on shutdown). It is the lowest-risk consumer of the cached rules: it performs only read-only filesystem stats and emits a log line per newly-ready file. M3 will replace the log sink with the digest+upload pipeline; M2's readiness state machine and per-rule cadence are the foundation M3 builds on.

**Observable outcome at completion:** with the worker enabled and at least one cached upload rule whose `glob` matches a file that has been size/mtime-stable for `>= stability_window_secs`, the agent logs an `info!` line carrying `{ file_path, file_modified_at }` exactly once per newly-ready file (re-scans do not re-log). A worker unit-test suite (temp filesystem + injected clock + injected `sleep_fn`, mirroring `agent/tests/workers/poller.rs`) covers glob matching, the stability state machine, per-rule cadence, already-reported dedupe, and the empty/no-rules case. `scripts/preflight.sh` prints `Preflight clean`.

## Progress

- [ ] **M2.1** Add the `glob` dependency: `glob = "0.3.2"` in `[workspace.dependencies]` (workspace `Cargo.toml`) and `glob = { workspace = true }` in `agent/Cargo.toml` `[dependencies]`.
- [ ] **M2.2** Add `agent/src/workers/uploads.rs` (`Options` + `Default`, `run(...)` with injected `sleep_fn` + injected `now_fn` clock + `shutdown_signal`, `run_impl` loop). Register `pub mod uploads;` in `agent/src/workers/mod.rs`.
- [ ] **M2.3** Implement per-rule poll cadence (per-rule next-scan time map keyed by `UploadRuleID`), reading rules each loop via `storage.upload_rules.values().await`.
- [ ] **M2.4** Implement glob matching (`glob::glob(&rule.source.glob)`) + the readiness state machine (per-file `last_size`/`last_mtime`/`stable_since`; reset on change; ready when unchanged and `now - stable_since >= stability_window_secs`). Mark the finalization-marker HOOK (comment only).
- [ ] **M2.5** Implement the log sink: on a newly-ready file, `info!` `{ file_path, file_modified_at }` and insert into an in-memory `already_reported` set so subsequent scans do not re-log.
- [ ] **M2.6** Wire into `agent/src/app/run.rs`: add `enable_uploads_worker` + `uploads: uploads::Options` to `AppOptions` (default-on, mirroring `enable_poller`/`enable_mqtt_worker`); add `init_uploads_worker`; add `uploads_worker_handle` to `ShutdownManager` (new + register + shutdown step).
- [ ] **M2.7** Tests — `agent/tests/workers/uploads.rs` (registered in `agent/tests/workers/mod.rs`): glob match/miss, stability state machine, per-rule cadence, dedupe, empty/no-rules. Add an `app/run.rs` duplicate-handle test mirroring the existing `register_handle_rejects_*` tests.
- [ ] **V** `scripts/update-deps.sh` then `scripts/preflight.sh` → final line `Preflight clean` (lint, clippy `-D warnings`, fmt, cargo-diet deadcode, machete, audit, full tests, covgate). MUST pass before publishing.

Use timestamps when completing steps. Split partially-completed work into "done" / "remaining".

## Surprises & Discoveries

(Add entries as work proceeds. Seed findings from the verified context below.)

- **`models::UploadRuleSource` already stores durations as `i32` seconds** — `poll_interval_secs` and `stability_window_secs`, NOT the string form the M1 plan originally sketched. Verified in `agent/src/models/upload_rule.rs:39-44` and the generated `libs/backend-api/src/models/upload_rule_source.rs:14-22`. The `glob` field doc string confirms it is "An absolute glob pattern (must start with `/`)". No duration parsing is needed in M2.
- **Workers obtain time via `chrono::Utc::now()` directly and inject only `sleep_fn`** (`agent/src/workers/poller.rs:86,94,99,123`; `token_refresh.rs:93,119`). The poller tests achieve determinism by setting syncer state relative to the real `Utc::now()` and stepping the injected `SleepController`. M2's readiness compares file mtimes against "now" at sub-poll precision, so M2 ADDS an injected clock (`now_fn`) alongside `sleep_fn` (see Decision Log) — this is an extension of, not a departure from, the inject-the-time-source convention.
- **`covgate.sh` discovers modules by `.covgate` *directories*, not files** (`scripts/lib/covgate.sh:67-74` — `find "$SRC_DIR" -name '.covgate'`, threshold applies to the containing directory's aggregate coverage). `agent/src/workers/.covgate` = **83.21** and `poller.rs` is a single file aggregated under it. So `uploads.rs` (a sibling file, NOT a new directory) is covered under the SAME `workers` aggregate threshold of 83.21 — it does NOT get its own threshold. Adding under-tested code drags the whole `workers` module below 83.21. Cover `uploads.rs` thoroughly. (If a future milestone splits uploads into `workers/uploads/`, that directory could carry its own `.covgate`.)

## Decision Log

- **Decision: use the `glob` crate (`glob = "0.3.2"`), NOT `globset` + `walkdir`.** Rationale: each rule supplies a single ABSOLUTE glob pattern (`source.glob` starts with `/`). `glob::glob(pattern)` walks the filesystem AND matches in one call, returning an iterator of matched `PathBuf`s — exactly the "absolute pattern → matching files" operation M2 needs. `globset` only matches already-enumerated paths against compiled patterns; pairing it with `walkdir` would require deriving a walk-root from the absolute pattern (fiddly: split off the literal prefix before the first wildcard) and re-implementing the traversal `glob` already does. `glob` is the simplest fit, is a `rust-lang-nursery`/`rust-lang` maintained crate with no open RUSTSEC advisory, and adds no transitive bloat. `glob::glob` does BLOCKING fs I/O; since scans run at most once per `poll_interval_secs` per rule, wrap the match in `tokio::task::spawn_blocking` (or accept the brief blocking call) — finalize during implementation; prefer `spawn_blocking` to keep the runtime responsive. Pin style mirrors the workspace convention (e.g. `base64 = "0.22.1"`): add `glob = "0.3.2"` to `[workspace.dependencies]` and reference it as `glob = { workspace = true }` in `agent/Cargo.toml`.
  Date/Author: 2026-06-28 / plan author.
- **Decision: inject a clock `now_fn: Fn() -> DateTime<Utc>` into `uploads::run`, alongside the injected `sleep_fn`.** Production wiring passes `chrono::Utc::now` (`agent/src/app/run.rs`); tests pass a controllable clock (a shared `Arc<AtomicI64>` of epoch seconds → `DateTime::from_timestamp`). Rationale: the readiness state machine compares `now - stable_since >= stability_window_secs`; with a real clock this is timing-flaky in tests. The poller injects only `sleep_fn` because its time math tolerates real-clock slop (it asserts sleep durations within `±1s`); M2's stability boundary does not. Injecting `now_fn` mirrors the established "inject the time-dependent function" pattern (`sleep_fn`) rather than reaching for `tokio::time::pause` (the poller does not use it). File mtimes are read as `SystemTime` from `std::fs::Metadata::modified()` and converted to `DateTime<Utc>` for the `file_modified_at` log field.
  Date/Author: 2026-06-28 / plan author.
- **Decision: `enable_uploads_worker` defaults to `true`, mirroring `enable_poller` and `enable_mqtt_worker`.** Rationale: matches the sibling-worker convention in `AppOptions::default()` (`agent/src/app/options.rs:69-76`), and the M2 worker's only effect is read-only filesystem stats + a deduped `info!` log line — harmless in production. With no cached upload rules (the common case until the feature ships fleet-wide) the worker idles. ALTERNATIVE considered: default `false` until the M3–M5 pipeline lands, so a half-built feature emits no logs. Rejected to honor the "mirror siblings" convention and because the dedupe set keeps the log volume bounded (one line per file, once). Revisit if the placeholder log proves noisy before M3.
  Date/Author: 2026-06-28 / plan author.
- **Decision: the placeholder sink is a real, wired-in `info!` + `already_reported` set mutation — not a stub — so the readiness computation is genuinely consumed (no dead code).** Rationale: the deadcode gate (`cargo-diet`) + clippy `dead_code` would flag a readiness result that drives nothing. Logging the result AND mutating the in-memory `already_reported: HashSet<PathBuf>` (which in turn gates future logging) makes "ready" an observable side effect. M3 swaps the `info!`/set for the digest+`POST /uploads` pipeline. The finalization-marker HOOK is a COMMENT in the readiness path (not a stub function), so it adds no dead code.
  Date/Author: 2026-06-28 / plan author.

## Outcomes & Retrospective

(Fill in during/after implementation: final `Options` fields, whether `glob` runs under `spawn_blocking`, the exact `now_fn` test-clock helper location, measured `workers` covgate %, and the final `Preflight clean` confirmation.)

## Context and Orientation

The agent is a Rust workspace rooted at `repos/agent/Cargo.toml`: `agent/` (binary crate — all logic), `libs/backend-api/` + `libs/device-api/` (OpenAPI-generated; do NOT hand-edit). Repo conventions: `repos/agent/AGENTS.md` (import ordering with `// standard crates` / `// internal crates` / `// external crates` group comments; `thiserror` errors; `#[cfg(feature = "test")]` gating; `scripts/test.sh` runs `RUST_LOG=off cargo test --features test`; per-module `.covgate`).

### Verified inputs (re-verify before finalizing)

- **Cached rules store** — `agent/src/storage/upload_rules.rs`:
  ```rust
  pub type UploadRules = cache::FileCache<models::UploadRuleID, models::UploadRule>;
  ```
  Read all rules via `storage.upload_rules.values().await` → `Result<Vec<models::UploadRule>, cache::CacheErr>` (exact signature: `agent/src/cache/concurrent.rs:491` — `pub async fn values(&self) -> Result<Vec<V>, CacheErr>`; `entries()` at `:486`). `storage::Storage` exposes `pub upload_rules: Arc<UploadRules>` (`agent/src/storage/mod.rs:85`); `AppState` exposes `pub storage: Arc<storage::Storage>` (`agent/src/app/state.rs:19`).
- **Domain model** — `agent/src/models/upload_rule.rs`: `UploadRule { id, upload_collection_id, upload_collection_name, digest, source: UploadRuleSource, destination: UploadRuleDestination, created_at, updated_at }`; `UploadRuleSource { glob: String, poll_interval_secs: i32, stability_window_secs: i32 }` (lines 39-44). `glob` is absolute (starts with `/`). `UploadRuleID = String`.
- **Worker pattern to MIRROR** — `agent/src/workers/poller.rs`:
  - `#[derive(Debug, Clone)] pub struct Options { ... }` + `impl Default`.
  - `pub async fn run<F, Fut, ...>(options: &Options, <deps...>, sleep_fn: F, mut shutdown_signal: Pin<Box<impl Future<Output=()> + Send + 'static>>) where F: Fn(Duration) -> Fut, Fut: Future<Output=()> + Send` (lines 34-56).
  - `tokio::select!` over `shutdown_signal.as_mut()` vs `run_impl(...)`; `run_impl` is the non-returning loop that calls `sleep_fn(Duration::from_secs(...))` inside its own `tokio::select!` (lines 44-132).
  - `mqtt::run` (lines 48-57) confirms the multi-dep + multi-generic form.
- **Spawn / handle registration** — `agent/src/app/run.rs`:
  - `init()` (lines 110-157) gates each worker on an `options.enable_*` flag and calls an `init_*_worker` fn. `init_poller_worker` (217-246) and `init_mqtt_worker` (248-279) are the templates: clone the needed `app_state` pieces, `tokio::spawn(async move { <worker>::run(&options, deps..., tokio::time::sleep, Box::pin(async move { let _ = shutdown_rx.recv().await; })).await; })`, then `shutdown_manager.register_handle(|mgr| &mut mgr.<worker>_worker_handle, "<name>_handle", handle)?`.
  - `ShutdownManager` (315-339) holds `poller_worker_handle`/`mqtt_worker_handle: Option<JoinHandle<()>>`, initialized to `None` in `new`. `register_handle` (361-381) rejects duplicates. `shutdown_impl` (420-485) awaits each handle in order (token_refresh → poller → mqtt → server → app_state).
- **App options / enable flags** — `agent/src/app/options.rs`: `AppOptions` (38-56) carries `enable_poller: bool` + `poller: poller::Options` and `enable_mqtt_worker: bool` + `mqtt_worker: mqtt::Options`; `AppOptions::default()` (58-78) sets both enable flags `true`. `use crate::workers::{mqtt, poller, ...}` at line 9.
- **Test harness** — `agent/tests/workers/poller.rs` uses `crate::mocks::error::SleepController` (`sleep_ctrl.sleep_fn()`, `.await_sleep()`, `.release()`, `.get_last_attempted_sleep()`; defined `agent/tests/mocks/error.rs:29-110`) + `filesys::Dir::create_temp_dir("testing")` + `tokio::spawn(run(...))` with a `std::future::pending()` shutdown. `agent/tests/workers/mod.rs` registers `pub mod {mqtt, poller, token_refresh};`.

### Validation tooling

- `scripts/preflight.sh` runs, in parallel: `scripts/lint.sh` (custom import linter, `cargo fmt`, `cargo machete` + diet unused-dep/deadcode, `rustsec` audit, `cargo clippy -D warnings`) and `scripts/covgate.sh` (`cargo test --features test` with coverage + per-module `.covgate` enforcement). Prints `Preflight clean` on success.
- `scripts/update-deps.sh` refreshes `Cargo.lock` (run BEFORE linting, especially after adding `glob`).
- `scripts/test.sh` = `RUST_LOG=off cargo test --features test` (the `--features test` flag is REQUIRED — mocks are behind `#[cfg(feature = "test")]`).
- Relevant covgate thresholds: `agent/src/workers/.covgate` = **83.21** (the new `uploads.rs` aggregates here), `agent/src/app/.covgate` (the `run.rs`/`options.rs` edits aggregate here — read the file for the exact number and keep the module at/above it).

## Plan of Work

### M2.1 — Add the `glob` dependency

- Workspace `Cargo.toml` `[workspace.dependencies]`: add `glob = "0.3.2"` (alphabetical-ish, near `futures`).
- `agent/Cargo.toml` `[dependencies]`: add `glob = { workspace = true }`.
- Run `scripts/update-deps.sh` to refresh `Cargo.lock`. Confirm `cargo machete` does not flag `glob` (it will be used by `uploads.rs`) and `audit` stays clean.

### M2.2 — New worker `agent/src/workers/uploads.rs`

Mirror `poller.rs`. Imports follow the group convention:

```rust
// standard crates
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::models::{UploadRule, UploadRuleID};
use crate::storage;

// external crates
use chrono::{DateTime, Utc};
use tracing::{debug, error, info};
```

`Options`:

```rust
#[derive(Debug, Clone)]
pub struct Options {
    /// Floor for the worker's sleep between scan passes and a fallback when a
    /// rule's poll_interval_secs is <= 0. Guards against a 0/negative interval
    /// busy-looping the worker.
    pub min_poll_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self { min_poll_interval_secs: 1 }
    }
}
```

`run` (mirror poller's `run` exactly, but add the injected `now_fn`):

```rust
pub async fn run<SleepF, SleepFut, NowF>(
    options: &Options,
    upload_rules: &storage::UploadRules,
    sleep_fn: SleepF,
    now_fn: NowF,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    SleepF: Fn(Duration) -> SleepFut,
    SleepFut: Future<Output = ()> + Send,
    NowF: Fn() -> DateTime<Utc>,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => { info!("Uploads worker shutdown complete"); }
        _ = run_impl(options, upload_rules, sleep_fn, now_fn) => {}
    }
}
```

### M2.3 — Per-rule poll cadence (`run_impl` loop)

`run_impl` owns the mutable in-memory state and loops:

- `next_scan_at: HashMap<UploadRuleID, DateTime<Utc>>` — when each rule is next due.
- `observations: HashMap<PathBuf, FileObservation>` — per-file stability state (M2.4).
- `already_reported: HashSet<PathBuf>` — dedupe set (M2.5).

Loop body:
1. `let now = now_fn();`
2. Load current rules: `let rules = match upload_rules.values().await { Ok(r) => r, Err(e) => { error!(...); Vec::new() } };` (log + treat as empty on cache error — never crash the worker).
3. For each `rule` with `next_scan_at.get(&rule.id)` either absent or `<= now`: run `scan_rule(&rule, &mut observations, &mut already_reported, now)` (M2.4/M2.5), then set `next_scan_at.insert(rule.id, now + interval)` where `interval = Duration::from_secs(max(rule.source.poll_interval_secs as i64, options.min_poll_interval_secs) as u64)`.
4. Prune `next_scan_at` / `observations` / `already_reported` entries for files/rules no longer present (optional in M2 — note as a minor leak that resets on restart; keep simple, prune `next_scan_at` for removed rule ids at least).
5. Compute the sleep: `wait = min over rules of (next_scan_at[rule.id] - now)`, clamped to `>= min_poll_interval_secs`; if there are no rules, sleep `min_poll_interval_secs` (idle).
6. `debug!("uploads worker sleeping {wait:?} until next scan");`
7. `tokio::select!` is NOT needed here (shutdown is handled by the outer `run`'s select); simply `sleep_fn(wait).await;` and loop. (Match poller's structure: the loop's `await` points are cancellation-safe because the outer `run` drops the `run_impl` future on shutdown.)

### M2.4 — Readiness via `stability_window_secs`

```rust
struct FileObservation {
    size: u64,
    mtime: std::time::SystemTime,
    stable_since: DateTime<Utc>,
}
```

`scan_rule`:
- `let paths = match glob::glob(&rule.source.glob) { Ok(p) => p, Err(e) => { error!("invalid glob {:?}: {e}", rule.source.glob); return; } };` (Decision Log: prefer wrapping the blocking enumeration + stat in `tokio::task::spawn_blocking`.)
- For each `Ok(path)` that is a file:
  - `let meta = std::fs::metadata(&path)` (skip on `Err`, e.g. file deleted mid-scan).
  - `let (size, mtime) = (meta.len(), meta.modified()?);`
  - Look up `observations.get(&path)`:
    - absent OR `(size, mtime) != (obs.size, obs.mtime)` → file is new or changed: `observations.insert(path, FileObservation { size, mtime, stable_since: now });` (reset the window). Do NOT report.
    - present and unchanged → check readiness: `if now.signed_duration_since(obs.stable_since).num_seconds() >= rule.source.stability_window_secs as i64` → file is READY → M2.5.
  - `// HOOK (M3): finalization-marker detection (MCAP footer / parquet magic bytes)`
    `// would gate readiness here in addition to size+mtime stability. NOT implemented in M2.`

Edge: a `stability_window_secs <= 0` means "ready as soon as observed stable for one poll" — the `>= 0` comparison naturally yields ready on the second observation (or first if `stable_since == now` and window is 0). Confirm/clamp during implementation.

### M2.5 — Output sink (placeholder)

When a file is READY (M2.4) and `!already_reported.contains(&path)`:

```rust
let file_modified_at: DateTime<Utc> = obs.mtime.into(); // SystemTime -> DateTime<Utc>
info!(
    file_path = %path.display(),
    file_modified_at = %file_modified_at,
    "upload candidate ready (M2 placeholder sink)"
);
already_reported.insert(path.clone());
```

Subsequent scans see the file in `already_reported` and skip re-logging. (M3 replaces this block with the digest + `POST /uploads` pipeline.)

### M2.6 — Wire into `app/run.rs`

- `agent/src/app/options.rs`:
  - `use crate::workers::{mqtt, poller, uploads, token_refresh::...};` (add `uploads`).
  - `AppOptions`: add `pub enable_uploads_worker: bool,` and `pub uploads: uploads::Options,`.
  - `AppOptions::default()`: add `enable_uploads_worker: true,` and `uploads: uploads::Options::default(),`.
- `agent/src/app/run.rs`:
  - `use crate::workers::{mqtt, poller, uploads, token_refresh::...};`.
  - In `init()`, after the mqtt block: `if options.enable_uploads_worker { init_uploads_worker(options.uploads.clone(), app_state.clone(), shutdown_manager, shutdown_tx.subscribe()).await?; }`.
  - Add `init_uploads_worker` mirroring `init_poller_worker`:
    ```rust
    async fn init_uploads_worker(
        options: uploads::Options,
        app_state: Arc<AppState>,
        shutdown_manager: &mut ShutdownManager,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), ServerErr> {
        info!("Initializing uploads worker...");
        let upload_rules = app_state.storage.upload_rules.clone();
        let uploads_handle = tokio::spawn(async move {
            uploads::run(
                &options,
                upload_rules.as_ref(),
                tokio::time::sleep,
                chrono::Utc::now,
                Box::pin(async move { let _ = shutdown_rx.recv().await; }),
            )
            .await;
        });
        shutdown_manager.register_handle(
            |mgr| &mut mgr.uploads_worker_handle,
            "uploads_handle",
            uploads_handle,
        )?;
        Ok(())
    }
    ```
  - `ShutdownManager`: add `uploads_worker_handle: Option<JoinHandle<()>>` field; init `None` in `new`; await it in `shutdown_impl` (insert a step, e.g. after poller / before mqtt, with the same `JoinHandleErr` mapping + "handle not found" else-branch).

### M2.7 — Tests

See Test Steps.

## Test Steps

Tests use `--features test` (run via `scripts/test.sh`). Test files mirror `agent/src/` under `agent/tests/`. No `#[serial]` needed (each test uses its own `filesys::Dir::create_temp_dir("testing")`; no fixed OS paths).

**Test clock helper.** The injected `now_fn` needs to be controllable. Add a minimal helper (in `agent/tests/workers/uploads.rs`, or a small `agent/tests/mocks/clock.rs` registered in `agent/tests/mocks/mod.rs`): a shared `Arc<std::sync::atomic::AtomicI64>` of epoch seconds with `fn now_fn(&self) -> impl Fn() -> DateTime<Utc>` returning `move || DateTime::from_timestamp(clock.load(Ordering::SeqCst), 0).unwrap()` and an `advance(secs)` method. To make a matched file observed as stable, set its mtime deterministically with `filetime::set_file_mtime` — OR, simpler and dependency-free, write the file, capture its real mtime, and drive only the injected `now_fn` forward past `stable_since + window` (the observation records the file's real mtime; readiness depends on `now - stable_since`, both clock-driven). Prefer the clock-only approach to avoid a new `filetime` dependency.

**Worker spawn pattern** (mirror `agent/tests/workers/poller.rs`): create a temp `Layout`, spawn `storage::UploadRules` via `UploadRules::spawn(64, layout.upload_rules(), capacity).await` (confirm the exact `spawn` arity from `agent/src/cache/file.rs`/`storage/upload_rules.rs`), `write_if_absent` the test rules, then `tokio::spawn(uploads::run(&options, &rules, sleep_ctrl.sleep_fn(), clock.now_fn(), pending_shutdown))`. Drive scans by stepping `sleep_ctrl.await_sleep()/release()` and asserting via a captured-log mechanism. Register `pub mod uploads;` in `agent/tests/workers/mod.rs`.

**Asserting the log sink.** Since the sink is `info!`, assert readiness either by (a) capturing logs with `tracing_subscriber`'s test layer / `tracing-test`-style buffer, or (b) — preferred to avoid a new dev-dep — refactor the readiness emission into a small testable seam: have `scan_rule` (or a `decide_ready(rule, observations, now) -> Vec<ReadyFile>` pure fn) RETURN the newly-ready files, and have `run_impl` do the `info!` + `already_reported` insert. Unit-test `decide_ready` directly (pure, deterministic) and let the worker-loop tests assert behavior via the returned/observable state. Finalize the seam during implementation; keep the `info!` itself wired in (Decision Log: no dead code).

### T1. Glob matching — matches and misses, absolute

In a temp dir, create `<tmp>/data/a.mcap`, `<tmp>/data/b.txt`, `<tmp>/other/c.mcap`. A rule with `glob = "<tmp>/data/*.mcap"` matches only `a.mcap` (not `b.txt`, not `other/c.mcap`). Assert the candidate set after enough elapsed time is exactly `{a.mcap}`. Add a `**` recursive case (`<tmp>/**/*.mcap` matches `a.mcap` + `c.mcap`). Assert an invalid glob pattern is logged and produces no candidates (no panic).

### T2. Stability state machine

- **change resets window**: rule `stability_window_secs = 10`. Observe file at `t=0` (stable_since=0). At `t=5` modify file (size change via a second `decide_ready`/scan with a changed observation) → `stable_since` resets to 5. At `t=12` (only 7s since reset) → NOT ready. At `t=16` (>=10s since reset) → ready.
- **unchanged >= window => ready**: observe at `t=0`, unchanged at `t=10` → ready.
- **not-yet-stable**: observe at `t=0`, check at `t=9` (window 10) → NOT ready.

### T3. Per-rule cadence

Two rules with different `poll_interval_secs` (e.g. 5 and 30). Drive `sleep_ctrl`/clock and assert each rule is scanned only when its own next-scan time elapses (rule A scans ~6x while rule B scans ~1x over 30s of simulated time). Assert the worker's computed sleep equals the nearest due rule's remaining interval (clamped to `min_poll_interval_secs`).

### T4. Already-reported dedupe

A single ready file is logged/returned as newly-ready exactly ONCE across multiple consecutive scans (the `already_reported` set suppresses re-emission on scans 2..N).

### T5. Empty / no-rules

- No cached rules → the worker idles (sleeps `min_poll_interval_secs`), no candidates, no panic.
- A rule whose glob matches nothing → no candidates, no panic.
- `upload_rules.values()` returning a cache error is logged and treated as empty (worker keeps running).

### T6. app/run.rs handle registration

Mirror `register_handle_rejects_poller_duplicates` (`agent/src/app/run.rs:529-555`): add `register_handle_rejects_uploads_duplicates` asserting a second `register_handle` for `uploads_worker_handle` returns `ServerErr::ShutdownMngrDuplicateArgErr { arg_name: "uploads_handle" }`.

## Validation and Acceptance

**Changes MUST NOT be published until `scripts/preflight.sh` reports `Preflight clean`.** Run from the repo root:

    cd /home/ben/miru/workbench4/repos/agent
    scripts/update-deps.sh        # refresh Cargo.lock after adding `glob`
    scripts/preflight.sh          # lint + clippy -D warnings + fmt + machete + diet(deadcode) + audit + tests + covgate, in parallel
    # final line must be: Preflight clean

Plus the individual gates the conventions mandate (all must pass clean):

    cargo build -p miru-agent
    scripts/test.sh                                   # RUST_LOG=off cargo test --features test
    cargo fmt -p miru-agent -- --check
    cargo clippy --package miru-agent --all-features -- -D warnings
    cargo machete                                     # `glob` must register as USED (wired into uploads.rs)

**Coverage gate (`scripts/covgate.sh`, invoked by preflight) will fail the build if coverage drops below a module's `.covgate`.** `uploads.rs` aggregates into `agent/src/workers/.covgate` = **83.21** (it is a file under `workers/`, not its own directory — see Surprises). Cover the readiness/cadence/dedupe logic thoroughly (the pure `decide_ready` seam makes this straightforward) so the `workers` aggregate stays `>= 83.21`. The `app/run.rs` + `options.rs` edits aggregate into `agent/src/app/.covgate` — keep that module at/above its threshold (the duplicate-handle test T6 plus default-on options exercised by existing app tests cover the additions). Adding a dependency must keep `machete` (used) and `audit` (no `glob` advisory) clean.

Acceptance (human-verifiable):

1. `agent/src/workers/uploads.rs` exists, mirrors the poller shape (`Options` + `Default`, `run` with injected `sleep_fn` + `now_fn` + `shutdown_signal`, `run_impl` loop), and is registered in `workers/mod.rs`.
2. With the worker enabled and a cached rule whose glob matches a file stable for `>= stability_window_secs`, the agent logs `{ file_path, file_modified_at }` exactly once per file; re-scans do not re-log.
3. `cargo test --features test` runs `agent/tests/workers/uploads.rs` covering glob match/miss (T1), the stability state machine (T2), per-rule cadence (T3), dedupe (T4), and empty/no-rules (T5), plus the app/run.rs duplicate-handle test (T6).
4. No digest/HTTP/ledger/delete_policy code is present (scope boundary respected); the finalization-marker HOOK is a comment only.
5. `scripts/preflight.sh` prints `Preflight clean`.

## Idempotence and Recovery

- M2 is purely additive (one new source file + one new dep + additive wiring in `options.rs`/`run.rs` + new test file). No generated code, no spec, no migrations.
- If `cargo build -p miru-agent` fails after M2.6 with a missing-field error on `AppOptions` or `ShutdownManager`, the wiring is incomplete — finish all sites: `AppOptions` struct + `Default`, `init()` gate, `init_uploads_worker`, `ShutdownManager` field + `new` + `shutdown_impl` step + `register_handle` call.
- If `cargo machete` flags `glob` as unused, the dependency was added before `uploads.rs` references it — complete M2.2+M2.4 together.
- The worker holds only in-memory state (`observations`, `already_reported`, `next_scan_at`); a restart re-derives everything from the cache + filesystem. There is no persisted state to corrupt or migrate.
- Rollback: `git -C /home/ben/miru/workbench4/repos/agent checkout -- agent/src/workers/uploads.rs agent/src/workers/mod.rs agent/src/app/{run,options}.rs agent/Cargo.toml Cargo.toml Cargo.lock agent/tests/workers/` and `rm agent/src/workers/uploads.rs agent/tests/workers/uploads.rs` restores pre-change state.

---

Change note (2026-06-28): Initial draft. Covers M2 (upload file discovery + readiness): new `workers/uploads.rs` mirroring the poller (per-rule `poll_interval_secs` cadence; size+mtime `stability_window_secs` readiness state machine; deduped placeholder `info!` sink), a `glob = "0.3.2"` dependency, and default-on `app/run.rs` wiring with a tracked shutdown handle. Key decisions: `glob` crate over `globset`+`walkdir` (single absolute pattern); inject a `now_fn` clock alongside `sleep_fn` for deterministic readiness tests; `enable_uploads_worker` defaults `true` (mirrors siblings); the log+dedupe sink is real wired-in behavior to avoid the deadcode gate. Scope ENDS at identifying ready files; digest/POST/confirm/ledger/delete_policy are M3–M5.
