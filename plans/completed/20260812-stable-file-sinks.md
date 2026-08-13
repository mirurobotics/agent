# PR 3 — Stable-file sinks: replace the lossy broadcast bridge with injected, awaited StableFileSink handles

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench4/repos/agent | read-write | All changes land here (crate `miru-agent` under agent/) |

Base branch `main`; working branch `refactor/stable-file-sinks` (already created, clean). No backend, spec, or generated-code (libs/) changes. No wire-protocol changes — this is an internal delivery-mechanism refactor plus a permanent-data-loss bug fix.

## Purpose / Big Picture

`ScanEvent::StableFile` is today delivered over a tokio broadcast channel (capacity 256, agent/src/scan/scanner.rs) to the `workers/scan_upload_bridge.rs` subscriber, which builds an upload `Job` and enqueues it. The bridge's `RecvError::Lagged` arm logs "files will be re-observed on the next scan" — **that claim is false**. The scanner's ledger dedups: `evaluate_candidates` filters with `!state.is_latest_ledger_entry(obs)` (agent/src/scan/rule.rs:186), so a stable file is emitted exactly once per (metadata) observation. If the broadcast buffer drops that one event, the file's upload is **permanently lost** until its metadata changes. Bursts >256 stable files are realistic: an agent offline for days accumulates a backlog that all goes stable in one tick, while the uploader's per-enqueue snapshot persistence makes the bridge consumer slow.

The fix: delete the event channel and the bridge worker entirely. The scanner takes a set of **StableFileSink** handles at construction and, inside its scan tick, calls each sink per stable file and **awaits** it — backpressure replaces loss. The upload sink is today's bridge body relocated: gate on `rule.upload.is_some()`, build the `Job`, `uploader.enqueue(job)`, warn on enqueue failure without propagating. Policy lives in the sink; the scanner stays policy-unaware (it knows THAT consumers exist, not WHAT they do). A future PR (3b) adds a retention/delete sink — this design must make that a one-line wiring addition, but the delete sink itself is explicitly out of scope here.

## Progress

- [x] M1: `StableFileSink` trait in scan/, scanner takes sinks in `ScannerArgs`, tick awaits sinks; delete ScanEvent/broadcast/subscribe machinery
- [x] M2: `UploadStableFileSink` in upload/ (relocated bridge policy); delete workers/scan_upload_bridge.rs and its app wiring; flip uploader/scanner init order in app/state.rs
- [x] M3: Test surface — rework scanner in-source tests to recording sinks (incl. >256 burst no-loss test), port bridge tests to sink tests, strip MockScanner subscribe machinery
- [ ] M4: Preflight CLEAN locally, push, CI green on branch head, draft PR

## Surprises & Discoveries

- **No covgate adjustments were needed.** Deleting the bridge left workers/ at 85.82% (gate 84.67), scan at 99.24% (98.83), upload at 97.09% (96.00), app at 94.08% (90.38). Every gate passes unmodified.
- **Pre-existing flaky test**: `deploy::fsm::tests::next_action_fn::deployed_activity` failed one full-suite run with `expected wait time PT3600S is not equal to actual wait time PT3599.998972073S` — a wall-clock drift flake unrelated to this change (file untouched since commit 1053f4e). It passed in isolation and on the next full-suite run.
- **The RecordingSink yields (`tokio::task::yield_now`) before recording**, so every sink-based assertion doubles as proof that the scan tick awaits sink futures to completion rather than fire-and-forgetting them — the planned "slow-sink nice-to-have" folded into the base helper for free.

## Decision Log

Decisions below marked "(agreed with repo owner)" were settled before planning — do not relitigate them.

- **(agreed) Await, don't buffer**: the scanner calls sinks directly and awaits them inside its tick. A slow uploader slows the scan tick instead of dropping uploads. No parallel broadcast channel is kept "for observability" — a second lossy channel invites the same misuse this PR removes.
- **(agreed) Sinks are infallible from the scanner's perspective**: `on_stable_file` returns `()`. Sinks handle/log their own errors internally; the scanner must not fail its tick because a sink errored.
- **(agreed) Upload policy stays in the sink**: the `rule.upload.is_some()` gate, Job construction, and enqueue-failure warn move verbatim from the bridge into the upload sink. Retention-only rules produce no job.
- **(agreed) Delete entirely**: `ScanEvent`, the broadcast channel, `subscribe`/`Command::Subscribe`/`ScannerExt::subscribe`, `broadcast_capacity`/`DEFAULT_BROADCAST_CAPACITY`, `workers/scan_upload_bridge.rs`, its ShutdownManager handle + registration in app/run.rs, and all `Lagged`/`Closed` handling. Tests use a recording mock sink instead of subscribing.
- **Trait shape — object-safe boxed-future desugar (planner decision)**: the repo has NO dyn async traits today (`ScannerExt`/`UploaderExt`/`TokenManagerExt` all use plain `async fn` in traits, which is not object-safe, and there is no `#[async_trait]` dependency). The scanner needs a *heterogeneous* sink collection (`Vec<Arc<dyn StableFileSink>>` so PR 3b adds a sink without touching scanner generics), so we hand-desugar to a boxed future — the same pattern the repo already uses for `Pin<Box<dyn Future<Output = ()> + Send>>` shutdown signals in workers and app/run.rs:

      pub trait StableFileSink: Send + Sync {
          fn on_stable_file<'a>(
              &'a self,
              file: StableFile,
              rule: &'a FileRule,
          ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
      }

  `file` by value (each stable file goes to sinks once; today's code already clones per event), `rule` by reference (many files can share one rule per tick; sinks clone the fields they need). No new dependency, no macro.
- **Trait location scan/, impl upload/ (planner decision)**: the trait lives in a new `agent/src/scan/sink.rs` (the scanner's own extension point; scan/ must name the type it stores). The upload sink `UploadStableFileSink` lives in a new `agent/src/upload/sink.rs`: upload/ gains a dependency on scan/ (for `StableFile`, `FileRule` comes from models/), scan/ gains none on upload/ — currently neither imports the other, so this creates a clean one-way scan ← upload edge and keeps the scanner policy-unaware. workers/ was only right for the bridge because it was a spawned worker; the sink is not a worker.
- **Init-order flip in app/state.rs (planner decision)**: `AppState::init` currently creates the scanner BEFORE the uploader (state.rs lines 93-103), but sinks need the uploader handle at scanner construction. `init_uploader` has no dependency on the scanner, so the cleanest resolution is to simply swap the two calls: uploader first, then build `Vec<Arc<dyn StableFileSink>>` (upload sink present iff the uploader spawned) and pass it into `init_scanner`. No late-binding, no `OnceCell`, no Option-swap dance.
- **Shutdown ordering unchanged**: `AppState::shutdown` keeps shutting the uploader down first, then the scanner (as today). If a scan tick races shutdown and the sink enqueues into a stopped uploader, the sink logs a warn and the tick completes — acceptable by design since sinks only enqueue. The stale comment at state.rs:230-231 ("its feeder (the scan-upload bridge worker) has already been joined") must be rewritten to describe the sink reality.
- **ScannerArgs keeps `Default`**: `sinks` defaults to an empty Vec (a sink-less scanner scans and ledgers but delivers nowhere — exactly what many existing scanner unit tests want).
- **`UploadStableFileSink` is concrete over `Arc<Uploader>`, not generic (executor decision)**: the plan sketched `UploadStableFileSink<U: UploaderExt>`, but `UploaderExt` uses plain `async fn` in trait — for a generic `U` the returned futures are not provably `Send`, so the impl body cannot be boxed as `Pin<Box<dyn Future + Send>>` (the scanner worker is `tokio::spawn`ed and requires `Send`). The concrete `Uploader` gets auto-trait leakage and compiles; the tests use the real `Uploader` + `MockUploadExecutor` anyway (the house pattern), so nothing needed the generic.
- **run.rs scan-upload ShutdownManager tests deleted, not retargeted (executor decision)**: `register_handle_rejects_scan_upload_bridge_duplicates` and `shutdown_impl_maps_scan_upload_bridge_worker_join_error` were exact clones of the surviving scan-worker and sync-scan-bridge variants, which keep both the duplicate-registration and join-error paths covered (app/ covgate passes at 94.08 vs 90.38 required).
- **Covgate handling**: deleting src/workers/scan_upload_bridge.rs shifts the workers/ coverage ratio (gate 84.67) and adding scan/sink.rs + upload/sink.rs adds lines under the 98.83 scan and 96.00 upload gates. Run scripts/covgate.sh; if a gate fails purely because deletion/addition shifted ratios (not because new code is untested), adjust via scripts/update-covgates.sh and record the before/after numbers in Surprises & Discoveries. New sink code must itself be tested to the neighborhood of its module's gate.

## Context and Orientation

All paths relative to /home/ben/miru/workbench4/repos/agent. Read AGENTS.md first: three-group comment-headed imports (custom linter), thiserror + `crate::errors::Error`, `#[cfg(feature = "test")]` for test-only code, tests mirror src layout, per-directory `.covgate` files, fmt/clippy scoped `--package miru-agent` (never `cargo fmt --all`), field-by-field-assert lint (4+ `assert_eq!` on one variable's fields; suppress with `// lint:allow(field-by-field-assert)` only when genuinely needed).

### The pipeline today (what changes)

- **agent/src/scan/scanner.rs** — the whole file matters:
  - `ScanEvent` enum (lines 22-25), `DEFAULT_BROADCAST_CAPACITY` (27) — DELETE.
  - `ScannerArgs { now_fn, broadcast_capacity, snapshot_file }` (39-53) — `broadcast_capacity` becomes `sinks: Vec<Arc<dyn StableFileSink>>` (empty in `Default`).
  - `SingleThreadScanner { .., subscriber_tx, .. }` (55-61) — field becomes `sinks`.
  - `subscribe()` (114-116) and `emit_stable_files()` (118-127) — replaced by an async `dispatch_stable_files` that iterates `(file, rule)` pairs and awaits every sink.
  - `scan()` (202-254) — already collects `stable_files: Vec<(StableFile, FileRule)>` with the rule cloned in (line 221); the only change is `self.emit_stable_files(stable_files)` → `self.dispatch_stable_files(stable_files).await` and the log message ("emitted" → "delivered to sinks" or similar).
  - Actor surface: `Command::Subscribe` (283-286), its `Worker::run` arm (343-347), `ScannerExt::subscribe` (267), `Scanner::subscribe` impl (450-453) — DELETE.
  - In-source `#[cfg(test)]` module (463-1727): pervasive `subscribe`/`assert_one_stable`/`spawn_scanner_with_capacity` helpers and `ScanEvent` pattern-matches — reworked in M3 (details there).
- **agent/src/workers/scan_upload_bridge.rs** — DELETE the file. Its `run_impl` gate + `enqueue_stable_file` body (lines 46-92) relocate into the upload sink. Remove `pub mod scan_upload_bridge;` from agent/src/workers/mod.rs (line 13).
- **agent/src/app/run.rs**:
  - import of `scan_upload_bridge` (line 20), `init_scan_upload_bridge_worker` fn (368-391) and its call site (174-181) — DELETE.
  - `ShutdownManager.scan_upload_bridge_worker_handle` field (440), init (455), join arm (646-650) — DELETE.
  - In-source tests `register_handle_rejects_scan_upload_bridge_duplicates` (~885), `shutdown_impl_maps_scan_upload_bridge_worker_join_error` (~1057), and the third handle-registration test using it (~1111) — delete or retarget onto a surviving handle so ShutdownManager coverage doesn't drop (prefer retargeting the join-error tests onto another worker handle if they are the only coverage of that path).
- **agent/src/app/state.rs**: `init_scanner` (136-173) gains a `sinks` parameter forwarded into `ScannerArgs`; `AppState::init` (93-103) swaps uploader-before-scanner and builds the sink vec from the uploader Arc; stale shutdown comment (230-231) rewritten.
- **agent/src/scan/mod.rs**: remove `ScanEvent` from the re-export (line 7); add `pub mod sink;` and re-export `StableFileSink`.
- **agent/src/upload/mod.rs**: add `pub mod sink;` (+ re-export `UploadStableFileSink` alongside the existing style).
- **agent/src/scan/rule.rs:186** — `is_latest_ledger_entry` dedup filter; unchanged, but it is WHY loss was permanent: cite it in the sink-trait doc comment so the no-loss contract is written down.

### Test surface today (what changes)

- **agent/tests/workers/scan_upload_bridge.rs** — whole file (Harness over MockScanner.emit + real Uploader + MockUploadExecutor; tests: `stable_file_becomes_upload_job`, `idles_when_scanner_stream_closes`, `retention_only_stable_file_becomes_no_job`, `each_stable_file_becomes_a_job_in_order`). DELETE; behavior ports to agent/tests/upload/sink.rs (the `idles_when_scanner_stream_closes` test has no successor — there is no stream to close). Remove `mod scan_upload_bridge;` from agent/tests/workers/mod.rs.
- **agent/tests/mocks/scanner.rs** — MockScanner's `subscribe_tx`, `emit`, `subscriber_count`, `close`, and the `ScannerExt::subscribe` impl (lines 27-77, 145-153) — DELETE (the trait method is gone). The update_rules/clear_rules/scan recording surfaces stay; its remaining consumers (agent/tests/workers/sync_scan_bridge.rs, agent/tests/workers/scan.rs) do not use emit/subscribe.
- **agent/src/scan/scanner.rs `#[cfg(test)]`** — every `subscribe(&scanner)`-based assertion becomes a recording-sink-based one (M3).
- MockUploadExecutor (agent/tests/mocks/upload_executor.rs) + real `Uploader::spawn` is the house pattern for observing enqueued jobs end-to-end; reuse it in sink tests.

### Terms

- **Sink**: an injected async consumer of stable files, called and awaited by the scanner actor inside its tick.
- **Recording sink**: a test `StableFileSink` capturing `(StableFile, FileRule)` pairs into `Arc<Mutex<Vec<_>>>` for assertions.
- **Ledger dedup**: `is_latest_ledger_entry` — a file whose latest ledger entry matches the observation is never re-emitted; delivery is therefore exactly-once and must not be lossy.

## Plan of Work

### M1 — Sink trait + scanner rewiring (agent/src/scan/)

1. New `agent/src/scan/sink.rs`: the `StableFileSink` trait per the Decision Log signature, with a doc comment stating the contract — called once per newly-stable file, awaited by the scan tick (backpressure, never loss — cite the ledger dedup at rule.rs `is_latest_ledger_entry`), infallible (sinks log their own errors), and `Send + Sync` for `Arc<dyn ...>` sharing.
2. agent/src/scan/scanner.rs:
   - Delete `ScanEvent`, `DEFAULT_BROADCAST_CAPACITY`, the `broadcast` import.
   - `ScannerArgs`: `broadcast_capacity: usize` → `sinks: Vec<Arc<dyn StableFileSink>>`; `Default` uses `Vec::new()`.
   - `SingleThreadScanner`: `subscriber_tx` → `sinks: Vec<Arc<dyn StableFileSink>>`; constructor stores `args.sinks`.
   - `subscribe()` + `emit_stable_files()` → `async fn dispatch_stable_files(&self, stable_files: Vec<(StableFile, FileRule)>)`: for each `(file, rule)`, for each sink, `sink.on_stable_file(file.clone(), &rule).await` (clone only when >1 sink needs the file; with the common single-sink case, move). Keep a debug log for the delivered count.
   - `scan()`: await the dispatch; adjust the tick-complete log.
   - Delete `Command::Subscribe`, its worker arm, `ScannerExt::subscribe`, and the `Scanner` impl of it.
3. agent/src/scan/mod.rs: `pub mod sink;`, re-export `StableFileSink`, drop `ScanEvent` from the re-export line.
4. Compile gates now, and after every milestone: `cargo check --package miru-agent` (no features — recent regression source) AND `cargo check --package miru-agent --features test`.

### M2 — Upload sink + wiring (agent/src/upload/, app/)

5. New `agent/src/upload/sink.rs`: `pub struct UploadStableFileSink<U: UploaderExt> { uploader: Arc<U> }` (generic like the rest of the repo; the trait object erases it at the scanner boundary) with `new(uploader: Arc<U>)`. `impl StableFileSink`: body is today's bridge logic verbatim — if `rule.upload.is_none()`, debug-log "rule {id} does not upload; skipping {file}" and return; else build `Job { file, size, digest, mtime, first_observed_at, last_observed_at, file_rule_id, deployment_id, retention }` from the StableFile and `uploader.enqueue(job)`, logging enqueue failure at `warn!` without propagating. Wrap the async body in `Box::pin(async move { .. })`.
6. Delete agent/src/workers/scan_upload_bridge.rs; remove its decl from workers/mod.rs.
7. agent/src/app/run.rs: delete `init_scan_upload_bridge_worker`, its call site, the `scan_upload_bridge` import, the ShutdownManager field/init/join-arm, and fix its in-source tests (retarget the join-error ShutdownManager tests onto a surviving worker handle rather than deleting coverage of that path).
8. agent/src/app/state.rs: swap init order (uploader first); build `let sinks: Vec<Arc<dyn StableFileSink>> = uploader.iter().map(|u| Arc::new(UploadStableFileSink::new(u.clone())) as Arc<dyn StableFileSink>).collect();` and pass into `init_scanner(layout, enable_uploader, sinks)`; forward into `ScannerArgs`. Rewrite the stale "feeder … already joined" shutdown comment; shutdown ORDER itself unchanged.
9. Sweep for stragglers: `grep -rn "ScanEvent\|scan_upload_bridge\|broadcast_capacity\|subscribe" agent/src agent/tests` — scanner/bridge hits must all be gone (syncer/event-hub/mqtt/sse `subscribe` hits are unrelated and stay).

### M3 — Test surface

10. Scanner in-source tests (agent/src/scan/scanner.rs `#[cfg(test)]`):
    - Add a `RecordingSink` test helper implementing `StableFileSink`, capturing `(StableFile, FileRule)` into `Arc<Mutex<Vec<_>>>`, with accessors (`events()`, `names()` etc.). Add a `spawn_scanner_with_sink(&clock) -> (Scanner, RecordingSink-handle)` helper; keep sink-less `spawn_scanner` for tests that only assert ledger/snapshot state. Delete `spawn_scanner_with_capacity` and the `subscribe`/`assert_one_stable` helpers.
    - Rework each subscribe-based test to assert on the recording sink AFTER the tick returns (dispatch is awaited inside `scan()`, so no wait/poll is needed — a strict improvement over broadcast timing): `subscribe_receives_stable_file_payload` → `sink_receives_stable_file_payload`; `retention_only_rule_scans_and_emits_without_upload` (asserts `rule.upload == None` reaches the sink); `upload_rule_emits_its_upload_block`; `emit_with_no_subscriber_does_not_error` → `scan_with_no_sinks_does_not_error`; the clear_rules/update_rules/scan tests that subscribe mid-flow attach the sink at spawn instead (the sink records from the first tick, so assertions like "no re-emission after re-push" become "no NEW recorded event after re-push" — compare counts before/after the tick).
    - **Burst no-loss test (pins the fixed bug)**: one rule, stability window 0; write >256 files (e.g. 300) into the temp dir; discover tick + evaluate tick; assert the recording sink captured exactly 300 distinct file names and `ledger_count == 300`. Under the old broadcast design with a subscriber-side consumer this size buffer, ≥44 events would have been `Lagged`-dropped and never re-emitted; with awaited sinks the count is exact. Also assert a SECOND evaluate tick records nothing new (ledger dedup — loss would now be permanent, which is why delivery must be lossless).
    - A slow-sink test is nice-to-have: a sink that awaits a small `tokio::time::sleep` (or a oneshot gate) per call still receives every file — proves the tick awaits rather than fire-and-forgets. Include it if cheap; the burst test is the required one.
11. New agent/tests/upload/sink.rs (add `mod sink;` to agent/tests/upload/mod.rs) porting the bridge tests against `UploadStableFileSink` directly (real `Uploader::spawn` + `MockUploadExecutor`, per the deleted harness):
    - `stable_file_becomes_upload_job`: full-struct Job equality (keep the whole-struct assert style — see memory note on struct assertions).
    - `retention_only_stable_file_becomes_no_job`: upload-less rule → no executor drive; interleave with an upload-bearing file so the assertion can't pass vacuously (as the old test did).
    - `each_stable_file_becomes_a_job_in_order`.
    - `enqueue_failure_is_swallowed`: shut the uploader down first (or use a full/erroring path), call `on_stable_file`, assert it returns `()` without panicking — covers the warn path.
12. agent/tests/mocks/scanner.rs: strip the subscribe machinery and `ScannerExt::subscribe` impl; confirm sync_scan_bridge/scan worker tests still compile.
13. Delete agent/tests/workers/scan_upload_bridge.rs and its `mod` decl. Check agent/tests/app/{state,run}.rs for references to the bridge/init order and update.
14. Run `./scripts/test.sh` (never bare cargo test — `--features test` is required) and `./scripts/covgate.sh`; adjust per the covgate Decision-Log entry.

### M4 — Preflight, push, CI, PR

15. `./scripts/update-deps.sh` if Cargo.lock is stale, then `./scripts/preflight.sh` → must print CLEAN. `cargo fmt --package miru-agent` (never `--all`). `cargo check --package miru-agent` with NO features one final time.
16. Commit in reviewable units from within this repo's git context (suggested: M1+M2 may need to be one commit if not independently compilable — prefer the simplest honest split, as in PR 1: `refactor(scan)!: replace lossy scan-event broadcast with awaited stable-file sinks`, then `test(scan,upload): port bridge tests to sink tests and add burst no-loss coverage`). Signed commits (commit.gpgsign is on — never disable).
17. Push `refactor/stable-file-sinks`, open a DRAFT PR onto main (`gh pr create` with full body — `gh pr edit` is broken on mirurobotics repos, see memory), watch CI on the branch head.

## Validation and Acceptance

Behavioral acceptance criteria:

1. **No-loss under burst (the bug)**: the scanner-level burst test delivers >256 stable files from one tick to a recording sink with zero drops, and a subsequent tick re-delivers none (ledger dedup intact). This test would fail on main's broadcast design.
2. **Upload sink behavior parity**: stable file from an upload-bearing rule → exactly one Job with all nine fields mapped (full-struct assert); retention-only rule → no Job (non-vacuous, interleaved); multiple files → jobs in order; enqueue failure → warn, no panic, no propagation.
3. **Scanner policy-unawareness**: scan/ has no import of upload/; the scanner compiles and scans with `sinks: vec![]`; grep proof:

       grep -rn "crate::upload" agent/src/scan/        # expect: no matches
       grep -rn "ScanEvent" agent/src agent/tests      # expect: no matches
       grep -rn "scan_upload_bridge" agent/src agent/tests  # expect: no matches
       grep -rn "broadcast" agent/src/scan/            # expect: no matches

4. **Wiring**: app boots with uploader enabled → scanner holds one upload sink; uploader disabled (`enable_uploader: false`) → scanner spawns with zero sinks (or not at all, as today); uploader spawn failure → scanner still spawns, sink-less (fail-open preserved). Existing app/state tests keep passing with the flipped init order.
5. **Extensibility (PR 3b readiness)**: adding a second sink is one `Arc::new(...)` push into the `sinks` vec in app/state.rs — no scanner or trait changes. Sanity-check by reading the final diff, not by building the delete sink.

Exact commands and expected results:

    ./scripts/test.sh          # all tests pass, exit 0
    ./scripts/covgate.sh       # every module gate met (record any gate adjustments + rationale)
    ./scripts/preflight.sh     # reports CLEAN, exit 0
    cargo check --package miru-agent   # NO features — must pass (recent regression source)

CI: after pushing, the workflow run on the branch head must conclude green. **The PR must not leave draft, and the task must not be reported complete, until preflight reports CLEAN and CI is green on the pushed branch head.**

## Idempotence and Recovery

- All edits are ordinary file edits/deletes on an existing branch; re-running a milestone is safe. Roll back uncommitted work with `git checkout -- <paths>`, committed work with `git reset --hard <last-good>`; the branch fully resets with `git reset --hard main`.
- `cargo check` / `test.sh` / `covgate.sh` / `preflight.sh` are safe to repeat. Force-push is acceptable before review starts.
- Nothing touches libs/backend-api, libs/device-api, or api/specs/ — a diff there is a mistake; revert it.
