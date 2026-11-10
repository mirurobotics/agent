# Put the latest calibrations store behind its own actor worker

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | Split `agent/src/calibrations/latest.rs` into `latest_impl.rs` (`LatestStoreImpl`) and an actor in `latest.rs` (`LatestStore` handle and worker); shut the store down in `agent/src/app/state.rs`; update `ARCHITECTURE.md` and the tests in `agent/tests/`. |

This plan lives in `agent/plans/` because every change is in the agent repo. Branch `refactor/latest-store-worker`, base `main`.

**Depends on the calibration rename PR.** A separate PR lands first and renames `calibrations/cal_mngr.rs` → `calibrations/uploader.rs` (`CalibrationManager` → `Uploader`, `CalMngrArgs` → `UploaderArgs`), `calibrations/uploader.rs` → `calibrations/uploader_impl.rs` (`Uploader` → `UploaderImpl`), and the `cal_mngr` fields and helpers → `cal_uploader` (for example `AppState::cal_uploader`). This branch is rebased onto `main` after that PR merges, and this plan uses the new names throughout. Where it names a file or helper the rename PR is expected to produce but this plan could not see (the integration test file `agent/tests/calibrations/uploader.rs`, formerly `cal_mngr.rs`; the fixture `test_utils::calibrations::cal_uploader`, formerly `cal_mngr`; `AppState::init_cal_uploader`), use whatever name the rename PR actually gave it.

## Purpose / Big Picture

A reviewer (Ben) on agent#421 asked for the latest calibrations store to have its own worker, the way this codebase already wraps disk state in an actor (`data_uploads::retention::Deleter` over `DeleterImpl`, `cache::FileCache`, `filesys::SharedStateFile`). Today `LatestStore` is a plain cloneable struct shared by `AppState::latest_cals` and the calibration uploader's worker; only that worker calls `put`, but only by convention, and every `list` and `content` re-reads `index.json` from disk with retries.

After this change the store is single-writer by construction (one worker task owns the files), the index is read once at open and kept in memory so `list` touches no disk and `content` reads only its content file, and `AppState` shuts the store down after the calibration uploader. It is a restructure only: `POST /v0.2/calibrations` still writes a report through to the store before it returns, and nothing a Device API client can observe changes. A later PR ("PR B") will load the store from Miru and keep an `is_confirmed` flag in this worker; this plan adds neither, and the worker owning the in-memory index is what makes that flag easy to add.

Observable: start the agent, `POST /v0.2/calibrations` a report, and the latest store lists it (through `AppState::latest_cals.list`, or the `GET /v0.2/calibrations/latest` route once agent#421 lands), exactly as before; stopping the agent logs `calibrations: latest store shutdown complete` after the calibration uploader's shutdown line.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `calibrations::latest_impl` (module, `agent/src/calibrations/latest_impl.rs`) | `latest_store.rs`, `latest/store.rs` | Matches `retention::deleter_impl`, `upload::uploader_impl` and the renamed `calibrations::uploader_impl`: the `<handle>_impl.rs` file beside the actor file. |
| `LatestStoreImpl` | `SingleThreadLatestStore`, `LatestStoreState` | The `<Handle>Impl` convention (`DeleterImpl`, `UploaderImpl`, `ScannerImpl`); `SingleThread*` is the name #350 retired. |
| `LatestStore::shutdown` | `close`, `stop` | Every actor handle in `AppState::shutdown` calls `shutdown`. |
| `LatestStore::spawn` (replaces `open`) | `open`, `start` | Every actor's constructor is `spawn(buffer_size, ..) -> (Self, JoinHandle<()>)`, as `calibrations::Uploader::spawn`. |
| `latest::Command` (private, variants `Put`, `List`, `Content`, `Shutdown`) | `Request`, `Msg` | `deleter.rs` and the calibration uploader name their enum `Command` and its variants after the handle methods. |
| `latest::Worker` (private) | `StoreWorker`, `Actor` | `deleter.rs` and the calibration uploader both name it `Worker`. |

`LatestStore`, `LatestStore::{put, list, content}`, `Filter` and `calibrations::latest::DEFAULT_CAPACITY` keep their names and signatures. `Filter` and `DEFAULT_CAPACITY` are defined in the private `latest_impl.rs`; `calibrations/mod.rs` re-exports `Filter` and `latest.rs` re-exports `DEFAULT_CAPACITY`, so no caller names `latest_impl`.

### Precedent

| New file | Mirrors |
|----------|---------|
| `agent/src/calibrations/latest_impl.rs` (today's `latest.rs`, moved) | `agent/src/calibrations/uploader_impl.rs`: a private module (`mod latest_impl;`) holding the single-threaded logic the worker owns, `pub(crate)` type, constructor and operations, inline unit tests at the bottom. |
| `agent/src/calibrations/latest.rs` (rewritten as the actor) | `agent/src/calibrations/uploader.rs`: private `enum Command` (whose `Shutdown` replies `()`), private `struct Worker` with a `while let Some(cmd)` loop whose `Shutdown` arm replies then breaks and logs `shutdown complete` after the loop, a private `respond(respond_to, value, op)` function, `spawn(buffer_size, ..) -> (Self, JoinHandle<()>)`, and a handle whose private `send_command` takes an `op` and logs send and receive failures. Error values come from `calibrations/errors.rs` (`SendActorMessageErr`, `ReceiveActorMessageErr`). |

No new test files: actor tests go in the existing `agent/tests/calibrations/latest.rs`, mirroring `agent/tests/data_uploads/retention/deleter.rs` (`enqueue_after_shutdown_errors`) and the calibration uploader's `commands_after_shutdown_fail`; the teardown test goes beside its siblings in `agent/tests/app/state.rs`.

### Left out

- A `LatestStoreExt` trait like `DeleterExt`: that trait lets drivers be generic over a mock deleter; nothing mocks the store, and the calibration uploader has no such trait either.
- The `is_confirmed` flag and loading the store from Miru: PR B.
- `report_id` on `LatestContent`: agent#421 adds it; it is not on `main`.
- `put` taking `String` to save one copy of the data per report: the handle keeps today's `&str` signature, so the handle copies it into the command.
- Copying a corrupt `index.json` aside and starting empty, as `StateFile::new_with_default` does: it would turn today's failing `list` into an empty one (a Device API change), and the next open would then delete every content file. See Open questions.
- Re-reading `index.json` when something outside the agent edits it: the worker is its only writer.
- Sharing one `dispatch!`/`respond` helper across actors: each actor defines its own today (`deleter.rs`, `shared_state_file.rs`, `cache/shared.rs`, the calibration uploader's `respond`); a shared helper is a follow-up.

### Open questions

- Should `open` follow `Deleter::spawn` and return `(Self, JoinHandle<()>)`, taking a `buffer_size`, so `AppState` joins the worker like its other seven? Settled by the design review: yes (see Decision Log). Original default: no. The brief keeps `LatestStore::open(layout, capacity) -> Self` so agent#421 and every fixture are untouched; `open` uses a channel of 64 (the size `AppState` gives every actor, and the literal `scan/scanner.rs` uses for its inner channel) and detaches the worker. `AppState::shutdown` awaits the worker's reply to `Shutdown`, and the worker holds nothing to flush after it, so not joining loses nothing.
- What does `open` do when the index can't be read? Default: what it does today, carried into memory. `open` reads the index with `files::read_json_with_retry` (a missing file is an empty index). On any error it logs, deletes no content, and the worker holds no index; each later call tries the read again (with the same retries) and fails with its error until a read succeeds, after which the index stays in memory. So a corrupt index still fails every call, and a read that fails for a moment at boot recovers on the next call. `StateFile` instead copies a corrupt file aside and starts from the default, and returns other read errors from its constructor; the store can't return an error from `open` without changing its signature, and copy-aside changes Device API behavior (see Left out). PR B, which loads the store from Miru, is the natural place to recover a corrupt index.
- Where do `Filter` and `DEFAULT_CAPACITY` live? Settled by the design review: defined in a private `latest_impl.rs`, with `DEFAULT_CAPACITY` re-exported from `latest.rs` (see Decision Log). Original default: in `latest_impl.rs`, where the move puts them, as `DeleterArgs` and `DEFAULT_QUEUE_CAPACITY` live in `deleter_impl.rs`. `calibrations::Filter` stays re-exported from `calibrations/mod.rs`, so its callers don't change; callers of `latest::DEFAULT_CAPACITY` change to `latest_impl::DEFAULT_CAPACITY` in the move commit. Alternative: move both back into `latest.rs` in milestone 2, keeping `latest::DEFAULT_CAPACITY` for agent#421 at the cost of moving the same lines twice.

## Progress

- [x] Milestone 0: branch rebased onto `main` after the calibration rename PR (agent#446) merged.
- [x] Milestone 1: move `latest.rs` to `latest_impl.rs` (move only), committed.
- [x] Milestone 2: `LatestStoreImpl` with the in-memory index, and the actor in `latest.rs`, committed.
- [x] Milestone 3: `AppState` shuts the store down; `ARCHITECTURE.md` updated; committed.
- [x] Source written
- [x] Design review findings applied: `LatestStore::spawn`, a private `latest_impl`, and the uploader's actor style.
- [ ] Milestone 4 (tests stage): tests updated and added, committed.
- [ ] Validation: `$preflight` reports `CLEAN`.

## Surprises & Discoveries

- The move commit also rewraps the `miru_agent::calibrations` import in `agent/tests/services/calibration/create.rs`: `latest_impl` pushes it past 100 columns.
- `ARCHITECTURE.md` names `calibrations::latest::DEFAULT_CAPACITY` too; Context's list of its users left it out.

## Decision Log

- 2026-11-10: An unreadable index fails each call until a read succeeds (the second open question's default).
- 2026-11-10 (design review): `LatestStore::open` becomes `pub async fn spawn(buffer_size, layout, capacity) -> (Self, JoinHandle<()>)`, infallible like `calibrations::Uploader::spawn`, replacing the default that `open` keeps its signature and detaches the worker. `AppState::init_cal_uploader` returns the store's join handle with the calibration uploader's, and the shutdown future joins it with the other actors'. The `latest_store` test fixture keeps its signature, calls `spawn(64, ..)` and drops the handle.
- 2026-11-10 (design review, round 2): `spawn` is sync, like `calibrations::Uploader::spawn`. `LatestStoreImpl::new` reads nothing, and the worker runs `prune_orphaned_contents` before it serves a command, as the uploader's worker runs `prune_orphans`, so boot doesn't wait on the index read. The `latest_store` fixture stays `async` so its callers (including agent#421) don't change.
- 2026-11-10 (design review): `latest_impl` is a private module, like `uploader_impl`, and `LatestStoreImpl` is `pub(crate)`, replacing the default that callers name `latest_impl::DEFAULT_CAPACITY`. `DEFAULT_CAPACITY` is defined in `latest_impl.rs` but re-exported from `latest.rs`, so `calibrations::latest::DEFAULT_CAPACITY` and every caller read as on `main` (the move commit's path edits and the `create.rs` import rewrap are undone). `Filter` stays re-exported from `calibrations/mod.rs`. The reviewer's rule that callers never import a `*_impl` module ([agent#447](https://github.com/mirurobotics/agent/pull/447#discussion_r4520743816)) is captured in `AGENTS.md`.
- 2026-11-10 (design review): `latest.rs` follows `calibrations/uploader.rs`, one actor style per module: private `Command` and `Worker`, `Shutdown { respond_to: oneshot::Sender<()> }`, a private `respond` function instead of `dispatch!`, and `send_command` taking an `op` and logging send and receive failures. The worker, not `shutdown`, logs `calibrations: latest store shutdown complete` when its loop ends.
- 2026-11-10: Coverage gates are never lowered; uncovered actor plumbing gets tests instead.
- 2026-11-10: The branch is stacked on the rename PR (agent#446) instead of waiting for it to merge, and is retargeted to `main` once #446 merges.
- 2026-11-10: `LatestStoreImpl::index` returns `&Index`, not `&mut Index`: `put` edits a copy and every other caller only reads.
- 2026-11-10: The `ARCHITECTURE.md` path `latest::DEFAULT_CAPACITY` → `latest_impl::DEFAULT_CAPACITY` landed in milestone 3's commit with the rest of the doc edits, not in the move commit, which was already made when it turned up.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

All paths are relative to the agent repo root.

An **actor** here is a tokio task (the **worker**) that owns some state and serves `Command`s from an `mpsc` channel one at a time, and a **handle** that sends a command with a `oneshot` reply channel and awaits the reply. The pattern this plan copies is `agent/src/data_uploads/retention/`: `deleter_impl.rs` holds `DeleterImpl`, plain single-threaded logic with `&mut self` methods; `deleter.rs` holds a local `dispatch!` macro (run an operation, send its result to `respond_to`, log if the caller is gone), `pub(crate) enum Command`, `pub(crate) struct Worker { deleter: DeleterImpl, receiver }` whose `run` loops `while let Some(cmd) = self.receiver.recv().await`, replying `Ok(())` to `Shutdown` and breaking, and the `Deleter` handle whose private `send_command` maps a failed send to `SendActorMessageErr` and a dropped reply to `ReceiveActorMessageErr`, each built with `trace!()`. Commit `8f6772a` (#350) made that split.

`agent/src/calibrations/latest.rs` today holds the whole store: `DEFAULT_CAPACITY` (1000), `type Index = Vec<ReportMetadata>`, `Filter` (with `is_match`), and `#[derive(Clone, Debug)] pub struct LatestStore { index: File, contents: Dir, capacity: usize }` with:

- `pub async fn open(layout: LatestCalsLayout, capacity: usize) -> Self`, which runs `prune_orphaned_contents` (delete every file in `contents/` that no entry names; if the index can't be read, log at error level and delete nothing);
- `pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr>`: read the index, take out the entry with the same key (component, role, aspect), reject a new key at capacity with `CalibrationStoreFullErr` (507 `calibration_store_full`), write the content file named by `report_id`, push the entry, write the index atomically, then delete the replaced entry's content unless it is the same `report_id`;
- `pub async fn list(&self, filter: &Filter) -> Result<Vec<ReportMetadata>, CalibrationsErr>`: matching entries sorted by component, role, aspect;
- `pub async fn content(&self, report_id: Uuid) -> Result<String, CalibrationsErr>`: not found (`CalibrationNotFoundErr`, 404) unless an entry has that `report_id` and its file exists;
- private `read_index` (`files::read_json_with_retry`, missing file → empty), `write_index`, `content_file`, `write_content`, `delete_content`, and free functions `take_entry`, `key_parts`, `matches`, `check_capacity`, `not_found`, with inline unit tests for those helpers at the bottom.

`agent/src/calibrations/mod.rs` has `pub mod latest;` and `pub use self::latest::{Filter, LatestStore};`. `agent/src/calibrations/errors.rs` already defines `SendActorMessageErr`/`ReceiveActorMessageErr` aliases and the matching `CalibrationsErr` variants. `agent/src/calibrations/.covgate` requires 99.64% line coverage for the module.

The calibration uploader (`agent/src/calibrations/uploader.rs` after the rename PR) holds a `LatestStore` clone in its worker; `Worker::enqueue` queues a report, then awaits `self.latest.put(report.metadata, &report.data)`, and removes the queued report if the put fails. That code does not change.

`agent/src/app/state.rs`: `AppState` has `pub latest_cals: calibrations::LatestStore`. The calibration uploader's init function opens the store with `calibrations::LatestStore::open(layout.latest_cals(), calibrations::latest::DEFAULT_CAPACITY)`, passes a clone in `UploaderArgs::latest`, and returns it. `AppState::shutdown` shuts actors down in dependency order, the calibration uploader first, each as `if let Err(e) = self.x.shutdown().await { tracing::error!("failed to shutdown x: {e}"); first_err.get_or_insert(e.into()); }`. `ServerErr` already converts from `CalibrationsErr`. The HTTP servers stop before `AppState::shutdown` runs (`agent/src/app/run.rs`, step 8), so no request reaches a stopped store.

`ARCHITECTURE.md` describes the store in the paragraph that starts "`CalibrationManager`'s worker also writes the latest calibrations store" (after the rename PR, it names the calibration `Uploader`), and the invariant "**Shutdown ordering matters.**" lists ordering rules.

Users of `latest::DEFAULT_CAPACITY` on `main`: `agent/src/app/state.rs`, `agent/tests/server/{tcp.rs, sse.rs, handlers.rs}`, `agent/tests/services/calibration/{helpers.rs, create.rs}`, `agent/tests/calibrations/{latest.rs, uploader.rs}`. Find them with `grep -rn "latest::" agent/src agent/tests`.

agent#421 (open) edits `latest.rs` (adds `LatestContent`, changes `content`), the same `ARCHITECTURE.md` paragraph, and uses `latest::DEFAULT_CAPACITY`; whichever lands second resolves those conflicts. Its `services::calibration::{list_latest, get_content}` take `&LatestStore` and need no other change.

## Plan of Work

**Milestone 0.** Make sure the base has the rename PR: `agent/src/calibrations/uploader_impl.rs` exists and `UploaderArgs` is defined in `agent/src/calibrations/uploader.rs`. If not, rebase onto `origin/main` once that PR merges.

**Milestone 1: move only.** `git mv agent/src/calibrations/latest.rs agent/src/calibrations/latest_impl.rs`. Change nothing inside the file. Fix only the paths that named the old module: in `calibrations/mod.rs`, `pub mod latest;` → `pub mod latest_impl;` and the re-export to `pub use self::latest_impl::{Filter, LatestStore};`; in `calibrations/uploader.rs`, the import `latest::LatestStore` → `latest_impl::LatestStore`; and every `latest::DEFAULT_CAPACITY` (with its `latest` import) → `latest_impl::DEFAULT_CAPACITY` in the files listed in Context. The commit then reads as a rename plus path updates.

**Milestone 2: the impl and the actor.** In `agent/src/calibrations/latest_impl.rs`:

- Rename `LatestStore` to `LatestStoreImpl`, drop `Clone` from its derive (the worker owns the only one; keep `Debug`), and rename the field `index: File` to `index_file: File`. Add `index: Option<Index>`: the entries, read from `index_file` once; `None` until a read succeeds. Update the type's doc comment to say the worker in `latest.rs` owns it.
- `pub async fn open(layout, capacity) -> Self`: build with `index: None`, run `prune_orphaned_contents`, return. Its doc keeps "deletes the content no entry names".
- Add `async fn index(&mut self) -> Result<&mut Index, CalibrationsErr>`: if `self.index` is `None`, read it with today's `read_index` (propagating the error and leaving `None`); return `self.index.insert(..)` or the held one. Write it without `unwrap`/`expect`, e.g. `let index = match self.index.take() { Some(index) => index, None => self.read_index().await? }; Ok(self.index.insert(index))`.
- `prune_orphaned_contents(&mut self)`: get the names from `self.index().await` instead of `read_index`; on error log as today and return.
- `pub(crate) async fn put(&mut self, metadata, data: &str)`: start from `self.index().await?.clone()`, then the same steps as today on that copy; after `write_index` succeeds, set `self.index = Some(copy)`, then delete the replaced content. A failure at any step leaves the in-memory entries unchanged, matching the index on disk. Drop the doc sentence "Puts must not overlap…": the worker serves one command at a time.
- `pub(crate) async fn list(&mut self, filter: &Filter)`: filter and clone from `self.index().await?`, then sort as today.
- `pub(crate) async fn content(&mut self, report_id: Uuid)`: check `self.index().await?` for the `report_id`, then read the content file as today.
- `read_index` keeps its retry; it now runs at open and only again while no index is held. Keep `Filter`, `DEFAULT_CAPACITY`, `Index`, the free helpers and the inline unit tests where they are.

Create a new `agent/src/calibrations/latest.rs`, laid out as `deleter.rs` (imports in the repo's standard/internal/external order with comments):

    macro_rules! dispatch { /* as in deleter.rs */ }

    // ================================== WORKER ======================================= //
    pub(crate) enum Command {
        Put { metadata: Box<ReportMetadata>, data: String, respond_to: oneshot::Sender<Result<(), CalibrationsErr>> },
        List { filter: Filter, respond_to: oneshot::Sender<Result<Vec<ReportMetadata>, CalibrationsErr>> },
        Content { report_id: Uuid, respond_to: oneshot::Sender<Result<String, CalibrationsErr>> },
        Shutdown { respond_to: oneshot::Sender<Result<(), CalibrationsErr>> },
    }

    pub(crate) struct Worker { store: LatestStoreImpl, receiver: mpsc::Receiver<Command> }
    // run(): while let Some(cmd) = recv; Shutdown replies Ok(()) and breaks; Put dispatches
    // self.store.put(*metadata, &data); List dispatches self.store.list(&filter);
    // Content dispatches self.store.content(report_id).

    // ================================== HANDLE ======================================= //
    /// Handle to the latest calibrations store's worker, which owns the store's files and
    /// its index in memory, so it is the store's only writer. Clones share the worker.
    #[derive(Clone, Debug)]
    pub struct LatestStore { sender: mpsc::Sender<Command> }

    impl LatestStore {
        pub async fn open(layout: LatestCalsLayout, capacity: usize) -> Self
        // LatestStoreImpl::open(layout, capacity).await, then mpsc::channel(64), tokio::spawn(worker.run()), detach.
        pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr>
        pub async fn list(&self, filter: &Filter) -> Result<Vec<ReportMetadata>, CalibrationsErr>
        pub async fn content(&self, report_id: Uuid) -> Result<String, CalibrationsErr>
        /// Stop the worker. Calls through any clone then fail.
        pub async fn shutdown(&self) -> Result<(), CalibrationsErr>
        // info!("calibrations: shutting down latest store..."), send Shutdown, `.await??`,
        // info!("calibrations: latest store shutdown complete").
        async fn send_command<R>(&self, cmd: impl FnOnce(oneshot::Sender<R>) -> Command) -> Result<R, CalibrationsErr>
    }

`Put` boxes its metadata as the calibration uploader's `Enqueue` boxes its report, so clippy's `large_enum_variant` doesn't fire. `put`, `list` and `content` are `self.send_command(..).await?` (the outer `?` for the channel error, the inner result returned), as in `Deleter`. Move the doc comments for `open`, `put`, `list` and `content` to the handle (the behavior callers see) and keep short ones on the impl methods. The `dispatch!` messages follow the module's log style, e.g. `"calibrations: failed to send the latest store's put response"`. Keep `run` under the 50-line function limit.

In `calibrations/mod.rs`: `pub mod latest;` beside `pub mod latest_impl;`, and re-export `pub use self::latest::LatestStore;` and `pub use self::latest_impl::Filter;`. In `calibrations/uploader.rs`, import `LatestStore` from `latest` again.

**Milestone 3: shutdown and docs.** In `agent/src/app/state.rs`, `AppState::shutdown`: right after the calibration uploader's block, add the same block for `self.latest_cals.shutdown()` with the log "failed to shutdown latest calibrations store", under the comment `// shutdown the latest calibrations store after the calibration uploader, whose worker writes it`. Update the uploader init function's doc if it describes the store. `init` needs no change: `open` still returns the handle.

In `ARCHITECTURE.md`, in the latest store paragraph (as the rename PR left it):

- After the first sentence, add: "The store is its own actor: its single-threaded logic lives in `LatestStoreImpl` (`latest_impl.rs`), and `latest.rs` wraps it with the worker and the cloneable `LatestStore` handle, as `retention` does for `DeleterImpl`."
- Replace "It reads the index from disk on every call, retrying a failed read with a state file's delays unless the file is missing or unparseable. Only the manager's worker puts, one report at a time, and a put writes the content before the index." with: "The worker reads the index once, when the store opens, retrying a failed read with a state file's delays unless the file is missing or unparseable, then keeps it in memory: `list` reads no file, and `content` reads only the entry's content file. If the index can't be read at open, each call tries the read again and fails until one succeeds, so a corrupt index fails every call rather than being replaced. The worker is the store's only writer and serves one command at a time; a put writes the content, then the index, and only then changes the entries in memory, so a failed write leaves them as they were."
- Before the last sentence, add: "`AppState` shuts the store down after the calibration uploader."
- In the "**Shutdown ordering matters.**" invariant, add: "The latest calibrations store shuts down after the calibration uploader, whose worker writes it."

**Milestone 4: tests** (the `$implement` tests stage). In `agent/tests/calibrations/latest.rs` (update the header comment to mention shutdown). `Fixture::reopen` starts a second worker over the same files while `f.store`'s worker still holds the old index in memory, so after a `reopen` call the reopened store, not `f.store` or `f.entries()`:

- Replace `a_corrupt_index_fails_every_call` with `a_corrupt_index_at_open_fails_every_call`: put `arm`, `corrupt_index`, reopen, and assert `put`, `list` and `content` on the reopened store each fail with `CalibrationsErr::FileSysErr`.
- Replace `list_retries_an_index_read_that_fails_for_a_moment` with `open_retries_an_index_read_that_fails_for_a_moment`: same blocker and fixer (paused time), but `reopen` while blocked; the reopened store lists `[arm]`.
- Add `list_serves_the_index_read_at_open`: put `arm`, delete `index.json`, and `f.store.list` still returns `[arm]`.
- Add `a_call_reads_an_index_that_could_not_be_read_at_open` (paused time): put `arm`, put a directory where `index.json` goes, reopen, restore the file, and the reopened store lists `[arm]`.
- Add `put_fails_and_keeps_the_entries_when_the_index_cannot_be_written`: put `arm`, replace `index.json` with a directory, put `leg` fails with `FileSysErr`, and `list` returns `[arm]`.
- Add `commands_after_shutdown_fail`: spawn the store with `LatestStore::spawn` to keep its join handle, `shutdown` succeeds, await the handle so the worker has dropped its receiver, then `put`, `list` and `content` each fail with `CalibrationsErr::SendActorMessageErr`; mirrors the calibration uploader's `commands_after_shutdown_fail` in `agent/tests/calibrations/uploader.rs`, whose harness joins the handle in `stop`.

The other tests there (open, capacity, put, list, content) stay as they are and now run through the actor. The inline unit tests in `latest_impl.rs` need no change. The calibration uploader's tests (`agent/tests/calibrations/uploader.rs`) need none either; confirm `a_failed_store_write_queues_nothing` still holds a `LatestStore` clone and passes.

In `agent/tests/app/state.rs`, `shutdown` module, add `latest_cals_shutdown_error_does_not_abort_teardown`, a copy of the calibration uploader's `*_shutdown_error_does_not_abort_teardown` that stops `state.latest_cals` directly, expects `AppState::shutdown` to return `ServerErr::CalibrationsErr(_)`, and checks the device file is offline.

## Concrete Steps

All commands run from the agent repo root. Never run `cargo` or `scripts/*` locally; CI validates.

Milestone 0:

    git fetch origin
    git log --oneline origin/main -5        # the calibration rename PR must be listed
    git rebase origin/main                  # only if this branch predates it
    test -f agent/src/calibrations/uploader_impl.rs && grep -n "pub struct UploaderArgs" agent/src/calibrations/uploader.rs

Milestone 1:

    git mv agent/src/calibrations/latest.rs agent/src/calibrations/latest_impl.rs
    grep -rn "latest::" agent/src agent/tests      # edit each hit to latest_impl::
    git diff -M --stat                             # shows latest.rs => latest_impl.rs plus path edits
    git commit -m "refactor(calibrations): move the latest store to latest_impl.rs"   # via $commit

Milestone 2: edit `latest_impl.rs`, create `latest.rs`, edit `mod.rs` and `uploader.rs` as in Plan of Work, then:

    grep -rln "LatestStoreImpl\|latest_impl::" agent/src    # expect latest_impl.rs, latest.rs, mod.rs, app/state.rs
    git commit -m "refactor(calibrations): run the latest store in its own worker"   # via $commit

Milestone 3: edit `agent/src/app/state.rs` and `ARCHITECTURE.md`, then:

    git commit -m "refactor(app): shut the latest store down after the calibration uploader"   # via $commit

Milestone 4: edit `agent/tests/calibrations/latest.rs` and `agent/tests/app/state.rs`, then:

    git commit -m "test(calibrations): cover the latest store worker"   # via $commit

## Validation and Acceptance

Validation runs in CI only. Push the branch and run `$preflight`; the task is complete, and the PR may leave draft, only when `$preflight` reports `CLEAN`: CI green on the pushed branch head.

In `.github/workflows/ci.yml`: job `lint` passes (import order, the 50-line function limit, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo machete`); job `test` passes `cargo test` and `./scripts/covgate.sh`, including `agent/src/calibrations/.covgate` (99.64%); jobs `windows-check` and the Windows build jobs pass.

Acceptance, as behavior:

- Every existing test in `agent/tests/calibrations/latest.rs` (open, capacity, put, list, content), the calibration uploader's tests, `latest_cals_lists_what_cal_uploader_reports` in `agent/tests/app/state.rs`, and the Device API route tests in `agent/tests/server/handlers.rs` pass unchanged, showing the Device API and intake behave as before.
- `a_corrupt_index_at_open_fails_every_call`: a corrupt index still fails every call, as today.
- `open_retries_an_index_read_that_fails_for_a_moment` and `a_call_reads_an_index_that_could_not_be_read_at_open`: an index read that fails at boot is retried, at open and then on each call.
- `list_serves_the_index_read_at_open`: `list` no longer reads `index.json`.
- `put_fails_and_keeps_the_entries_when_the_index_cannot_be_written`: a failed put leaves the in-memory entries matching disk.
- `commands_after_shutdown_fail` and `latest_cals_shutdown_error_does_not_abort_teardown`: the worker stops on `shutdown`, and `AppState` shuts it down after the calibration uploader without aborting teardown on its error.

If `covgate` fails for `calibrations`, read the uncovered lines from the CI log and cover them with tests, including the actor plumbing (a caller that drops its reply receiver, a command after shutdown). Never lower `agent/src/calibrations/.covgate`.

## Idempotence and Recovery

Every step is a file edit plus a commit, and can be redone. If milestone 1's `git mv` is half done, `git status` shows it; finish it or `git restore --staged --worktree agent/src/calibrations`. If the rebase in milestone 0 conflicts with agent#421 (should it land first), keep #421's changes and apply them to `latest_impl.rs` (`LatestContent` and the `content` body) and to the `Content` command's reply type. No step touches data on a device: the files under `resources/latest_calibrations/` keep their format, so an agent upgraded or rolled back across this change reads them as before.
