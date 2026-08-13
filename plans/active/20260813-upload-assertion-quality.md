# Tighten weak upload test assertions (backoff durations, error typing)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, root `/home/ben/miru/workbench2/repos/agent`) | read-write | Tightens assertions in three upload test files; adds one small typed error to the upload error module and swaps three construction sites in the transfer layer to use it. |

Branch: `test/upload-assertion-quality`, already created off `main` at `c1ebf64`.

## Purpose / Big Picture

A code review flagged assertions in the data-upload tests that pass regardless of the behavior they claim to cover. Two tests assert only that *some* retry sleep happened, so a backoff of 0s and a backoff of 3600s look identical to them. Three transfer tests distinguish an unrecognized storage scheme from missing S3 credentials from missing GCS credentials only by matching text inside the error message, which the repo's testing convention forbids — but deleting those substring checks would leave three tests that cannot tell each other apart at all.

After this change, each of those tests fails if the behavior it names regresses: the two backoff tests assert the exact recorded sleep durations against a backoff pinned in the test, and the three transfer tests assert the *type* of the underlying failure via a new, minimal typed error. Observable outcome: running the upload test suite still reports `0 failed`, and each tightened test flips to failing when the corresponding production behavior is deliberately perturbed (this plan requires demonstrating that).

## Progress

- [ ] Milestone 1: pin backoff and assert exact sleeps in `hung_attempt_times_out_and_is_retried`.
- [ ] Milestone 1: pin backoff and assert exact sleeps in `retried_upload_enqueues_once_at_confirm_time`.
- [ ] Milestone 1: perturbation check (both tests fail with the perturbed constant, pass with it restored).
- [ ] Milestone 1: commit.
- [ ] Milestone 2: drop the message assertion in `full_queue_returns_queue_full_err`, assert the carried fields.
- [ ] Milestone 2: commit.
- [ ] Milestone 3: add `TransferErr` to `agent/src/data_uploads/upload/errors.rs`; use it at the three `transfer.rs` construction sites.
- [ ] Milestone 3: update the three transfer tests to assert by type; perturbation check.
- [ ] Milestone 3: commit.
- [ ] Milestone 4: full gate run (lint, test, covgate, preflight) and push; preflight CLEAN before the PR leaves draft.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: for Part B item (2), take option (i) — introduce a real typed distinction — rather than option (ii) (keep the message substrings and document why).
  Rationale: the three transfer errors are today all built by `executor_err("<string literal>")`, which boxes the message as `Box<dyn std::error::Error + Send + Sync>`; `ExecutorErr` carries no variant, code, or kind, and `crate::errors::Error::code()` falls through to its default `Code::InternalServerError` for all of them, so there is no existing discriminator except the text. The cost of introducing one is unusually small and fully contained: a fieldless `thiserror` enum in `agent/src/data_uploads/upload/errors.rs` whose `Display` strings are byte-identical to today's literals, plus three one-line changes in `agent/src/data_uploads/upload/transfer.rs`. Nothing else in the repo constructs, matches on, or reads these three errors (verified by grep for `unrecognized`, `s3_credentials`, `gcs_credentials` across `agent/src` and `agent/tests`), so blast radius is those four places. Classification is unchanged: `executor_err` sets `is_terminal: false` and `is_network_conn_err: false` regardless of the source type, so terminal/network handling in the uploader is untouched; propagation is unchanged (still a single `UploadErr::ExecutorErr`); wire- and log-visible text is unchanged because `ExecutorErr`'s `#[error("upload executor error: {source}")]` still formats the same strings. Only introspectability changes.
  Rejected: option (ii) (leave the substring assertions). It would leave the repo's own convention ("assert errors by code or type, never by message content") knowingly violated in the exact PR whose purpose is assertion quality, and would leave the tests one `format!` edit away from silently ceasing to discriminate. The honest argument for (ii) is that a production error-type change made solely for testability is a real cost — but here that cost is ~15 lines with no call-site fan-out and no behavior change, which is below the threshold where "don't change production for tests" should win.
  Date/Author: 2026-08-13 / ben@miruml.com.

- Decision: `TransferErr` is a plain fieldless enum with `impl crate::errors::Error for TransferErr {}` rather than an aggregating enum built with the `impl_error!` macro.
  Rationale: `agent/AGENTS.md` directs error *enums* to `impl_error!`, but that macro expands to `match self { Self::Variant(e) => e.code(), ... }` and therefore requires every variant to wrap an inner error type. `TransferErr`'s variants carry no payload; the trait's defaults (`Code::InternalServerError`, non-terminal, non-network) are exactly the classification these three failures already have. Adding three payload-carrying structs plus three `UploadErr` variants would change the error surface the uploader sees, which the doc comment on `executor_err` deliberately keeps as a single `ExecutorErr`.
  Date/Author: 2026-08-13 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

This repo is the Miru agent, a Rust workspace. The package under change is `miru-agent` (source in `agent/src/`, integration tests in `agent/tests/`, mirroring the source module layout). Terms and files a reader needs:

- **Uploader**: the actor in `agent/src/data_uploads/upload/uploader.rs` that pops queued upload jobs, runs one executor attempt each, and on failure requeues the job at the tail with a retry deadline.
- **`UploaderOptions`** (`agent/src/data_uploads/upload/uploader.rs`, around line 37): the uploader's tunables. Relevant fields: `attempts` (lifetime attempt budget, default 30), `backoff` (a `cooldown::Backoff`, default `base_secs: 10, growth_factor: 2, max_secs: 3600`), `attempt_timeout_floor` (default 120s) and `attempt_timeout_bytes_per_sec` (default 64 KiB) which together give a per-attempt deadline via `attempt_deadline(size) = floor + ceil(size / bps)` seconds.
- **`cooldown::calc`** (`agent/src/cooldown/mod.rs`): `min(base_secs * growth_factor^exp, max_secs)`, saturating.
- **How a retry wait becomes a recorded sleep.** In `handle_counted_failure` (uploader.rs ~line 270) a non-terminal, non-network failure does `entry.attempts += 1` and then `let wait = cooldown::calc(&self.options.backoff, entry.attempts - 1)`, i.e. the exponent is the lifetime attempt count minus one. `requeue_after` stamps `entry.next_attempt_at = now_fn() + wait` and appends at the tail. Back in the run loop, when the queue is non-empty but nothing is eligible, the loop computes `wait = earliest_next_attempt - now` and calls `idle_wait(wait)`, which is the **only** place `sleep_fn` is invoked. Network-classified failures instead take `handle_network_failure`, which uses a flat `backoff.base_secs` and does not consume the attempt budget.
- **Test clock seam** (`agent/tests/data_uploads/upload/uploader.rs`, helpers `spawn_with_test_clock` and `spawn_with_test_clock_and_deleter` around lines 85-128): `sleep_fn` pushes the requested `Duration` into a shared `Arc<Mutex<Vec<Duration>>>` **and advances a shared chrono clock by that duration**, returning an immediately-ready future; `now_fn` reads that same clock. Because the clock only ever moves via `sleep_fn`, the duration recorded for a single queued job is exactly the `wait` seconds that `handle_counted_failure` computed. That is why `retry_backoff_follows_expected_sequence` (uploader.rs ~line 282, the model to copy) can assert an exact vector.
- **Transfer layer**: `agent/src/data_uploads/upload/transfer.rs`. `SdkTransfer::transfer` matches on `credentials.scheme` and dispatches to `transfer_s3` / `transfer_gcs`, or returns `executor_err("unrecognized upload credential scheme")` for `Scheme::SchemeUnknown` (line ~170). `transfer_s3` returns `executor_err("s3 scheme is missing s3_credentials")` when `credentials.s3_credentials` is `None` (line ~89); `transfer_gcs` returns `executor_err("gcs scheme is missing gcs_credentials")` (line ~113).
- **Upload error types**: `agent/src/data_uploads/upload/errors.rs`. `UploadErr` is a `thiserror` enum with `#[error(transparent)]` variants `QueueFullErr`, `ExecutorErr`, `AttemptTimeoutErr`, `SendActorMessageErr`, `ReceiveActorMessageErr`, wired to the shared trait via `crate::impl_error!`. `QueueFullErr` is a struct carrying `capacity: usize`, `file: String`, `trace`. `ExecutorErr` is a struct carrying `source: Box<dyn std::error::Error + Send + Sync>`, `is_terminal: bool`, `is_network_conn_err: bool`, `trace` — **no** variant/code/kind distinguishing what failed. `executor_err<E: Into<Box<dyn std::error::Error + Send + Sync>>>(source)` wraps anything as a non-terminal, non-network `ExecutorErr`.
- **Shared error trait**: `agent/src/errors/mod.rs`. `Error` provides defaults `code() -> Code::InternalServerError`, `http_status()`, `params() -> None`, `is_network_conn_err() -> false`, `is_terminal() -> false`. There is no per-error code registry that could distinguish the three transfer failures today.
- **Testing convention** (`.claude/skills/write-tests/SKILL.md`, authoritative here): assert errors by code or type, never by message content; keep tests deterministic.

Full test names used below:

- `data_uploads::upload::uploader::hung_attempt_times_out_and_is_retried` (file `agent/tests/data_uploads/upload/uploader.rs`, ~line 316)
- `data_uploads::upload::uploader::retention_producer::retried_upload_enqueues_once_at_confirm_time` (~line 836)
- `data_uploads::upload::queue::enqueue::full_queue_returns_queue_full_err` (`agent/tests/data_uploads/upload/queue.rs`, ~line 134)
- `data_uploads::upload::transfer::unknown_scheme_is_unsupported` (~line 175), `::s3_scheme_without_credentials_errs` (~line 192), `::gcs_scheme_without_credentials_errs` (~line 278)

## Plan of Work

### Milestone 1 — exact backoff assertions (Part A)

**Edit 1: `agent/tests/data_uploads/upload/uploader.rs`, `hung_attempt_times_out_and_is_retried` (~line 316).**

Add a pinned `backoff` to the `UploaderOptions` literal the test already builds, keeping the existing `attempt_timeout_floor: Duration::from_secs(1)` and `attempt_timeout_bytes_per_sec: 64 * 1024`:

    backoff: miru_agent::cooldown::Backoff {
        base_secs: 1,
        growth_factor: 2,
        max_secs: 4,
    },

Replace `assert!(!sleeps.lock().unwrap().is_empty());` with

    assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_secs(1)]);

Arithmetic (state this as a comment in the test): the script is `Hang` then `Ok`. The hang is never released, so the attempt ends at the deadline `attempt_timeout_floor + ceil(42 / 65536) = 1s + 1s = 2s` of virtual tokio time. `AttemptTimeoutErr` is neither terminal nor network-classified, so it takes `handle_counted_failure`: `entry.attempts` becomes 1, and `wait = cooldown::calc(backoff, attempts - 1) = calc(exp = 0) = min(1 * 2^0, 4) = 1s`. The entry is stamped at `now + 1s` and requeued; the run loop finds nothing eligible and calls `idle_wait(1s)`, the single recorded sleep. The second attempt succeeds, the queue empties, and the loop parks in `receiver.recv()` without sleeping. Hence exactly `[1s]`.

**Edit 2: `agent/tests/data_uploads/upload/uploader.rs`, `retention_producer::retried_upload_enqueues_once_at_confirm_time` (~line 836).**

The test currently passes `UploaderOptions::default()` to `spawn_with_test_clock_and_deleter`. Replace that with the same pinned backoff:

    UploaderOptions {
        backoff: miru_agent::cooldown::Backoff {
            base_secs: 1,
            growth_factor: 2,
            max_secs: 4,
        },
        ..UploaderOptions::default()
    }

Replace `assert!(!sleeps.lock().unwrap().is_empty());` with

    assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_secs(1)]);

Arithmetic: the script is `Err` then `Ok`; `Err` is the non-terminal, non-network scripted failure, so one counted failure gives `attempts = 1` and `wait = calc(exp = 0) = 1s`, one `idle_wait(1s)`, then the confirming attempt.

Do **not** add an assertion comparing `confirmed_at` to `job.first_observed_at`: the shared clock's epoch is taken inside the spawn helper, which runs *before* `make_job`, so `confirmed_at - first_observed_at` is a hair under 1s and such an assertion would be flaky.

**Perturbation check (mandatory).** Temporarily edit `agent/src/data_uploads/upload/uploader.rs` line ~287, changing

    let wait = cooldown::calc(&self.options.backoff, entry.attempts - 1).max(0);

to use `entry.attempts` instead of `entry.attempts - 1`. With the pinned backoff this makes the first retry wait 2s instead of 1s. Both tests must FAIL. Restore the line (`git checkout -- agent/src/data_uploads/upload/uploader.rs`) and confirm both PASS. Record the observed failure output in Surprises & Discoveries. Do not commit the perturbation.

### Milestone 2 — queue error assertion (Part B item 1)

**Edit: `agent/tests/data_uploads/upload/queue.rs`, `enqueue::full_queue_returns_queue_full_err` (~line 134).**

Delete `assert!(err.to_string().contains("queue is full"), "message: {err}");`. Keep the `matches!(err, UploadErr::QueueFullErr(_))` type assertion as the contract, and strengthen it by asserting the carried fields — the queue was constructed with `Queue::new(1)` and the rejected job is `make_job("b.log")`:

    let UploadErr::QueueFullErr(e) = err else {
        panic!("expected QueueFullErr, got: {err:?}");
    };
    assert_eq!(e.capacity, 1);
    assert_eq!(e.file, "/data/b.log");

Confirm the expected `file` string against `make_job` at the top of the file rather than assuming. Leave the existing `assert_eq!(queue.len(), 1);` in place.

### Milestone 3 — typed transfer errors (Part B item 2, option (i))

**Edit 1: `agent/src/data_uploads/upload/errors.rs`.** Add, next to the other error types (above `UploadErr` so the file reads concrete-types-then-aggregate):

    /// The transfer layer's own preconditions, as a type so tests and callers
    /// can tell them apart. Carried as the `source` of an
    /// [`ExecutorErr`]; classification is unchanged (non-terminal,
    /// non-network) and the `Display` text is what already appeared in logs.
    #[derive(Debug, thiserror::Error)]
    pub enum TransferErr {
        #[error("unrecognized upload credential scheme")]
        UnrecognizedScheme,
        #[error("s3 scheme is missing s3_credentials")]
        MissingS3Credentials,
        #[error("gcs scheme is missing gcs_credentials")]
        MissingGcsCredentials,
    }

    impl crate::errors::Error for TransferErr {}

The `Display` strings must be byte-identical to the current string literals in `transfer.rs` so no log or wire text changes.

**Edit 2: `agent/src/data_uploads/upload/transfer.rs`.** Import `TransferErr` alongside the existing error imports, and replace the three string arguments:

- line ~89: `executor_err(TransferErr::MissingS3Credentials)`
- line ~113: `executor_err(TransferErr::MissingGcsCredentials)`
- line ~170: `executor_err(TransferErr::UnrecognizedScheme)`

No signature changes: `executor_err` takes `E: Into<Box<dyn std::error::Error + Send + Sync>>`. Verify `TransferErr` is reachable from the tests the same way `ExecutorErr` is.

**Edit 3: `agent/tests/data_uploads/upload/transfer.rs`.** Add a local helper near the other helpers at the top of the file:

    /// Extracts the transfer layer's typed precondition failure from the
    /// single `ExecutorErr` surface the actor sees.
    fn transfer_err(err: &UploadErr) -> &TransferErr {
        let UploadErr::ExecutorErr(e) = err else {
            panic!("expected ExecutorErr, got: {err:?}");
        };
        e.source
            .downcast_ref::<TransferErr>()
            .unwrap_or_else(|| panic!("expected TransferErr source, got: {:?}", e.source))
    }

Then in each of the three tests, replace the `matches!(err, UploadErr::ExecutorErr(_))` assertion **and** the following `to_string().contains(...)` assertion with a single typed assertion:

- `unknown_scheme_is_unsupported`: `assert!(matches!(transfer_err(&err), TransferErr::UnrecognizedScheme), "got: {err:?}");`
- `s3_scheme_without_credentials_errs`: `... TransferErr::MissingS3Credentials ...`
- `gcs_scheme_without_credentials_errs`: `... TransferErr::MissingGcsCredentials ...`

Leave the other transfer tests untouched — their sources are SDK errors, not `TransferErr`.

**Perturbation check (mandatory).** Temporarily change `transfer_s3`'s `ok_or_else` to produce `TransferErr::UnrecognizedScheme` (the exact bug class the substring assertions were guarding). `s3_scheme_without_credentials_errs` must FAIL while `unknown_scheme_is_unsupported` still passes. Restore with `git checkout -- agent/src/data_uploads/upload/transfer.rs` and confirm all three pass. Record the result.

**Coverage note.** The three derived `Display` arms are new lines and are not executed by type-based assertions. `agent/src/data_uploads/upload/.covgate` requires 96.00. If `./scripts/covgate.sh` drops below the gate, add a unit test to the existing `#[cfg(test)] mod tests` block at the bottom of `errors.rs` asserting each variant's `to_string()`. That is a `Display`-contract test on the type that owns the text — not "asserting an error by message". Do not lower the `.covgate` threshold.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent` (owns `rust-toolchain.toml`, pinning 1.97.0). Running cargo from a parent directory resolves 1.94.0 and fails on the AWS SDK MSRV.

Baseline before touching anything:

    cargo test --features test data_uploads::upload

Read the result as "0 failed" plus the presence of the named tests; never assert a hard suite total.

### Milestone 1

1. Apply Edits 1 and 2 from Milestone 1.
2. `cargo test --features test data_uploads::upload::uploader` — expect `0 failed`.
3. Perturb `uploader.rs` line ~287 (`entry.attempts - 1` -> `entry.attempts`), rerun step 2, confirm both tests FAIL with a diff like `left: [2s]` / `right: [1s]`. Paste the failure into Surprises & Discoveries.
4. Restore: `git checkout -- agent/src/data_uploads/upload/uploader.rs`; rerun step 2, expect `0 failed`.
5. `cargo fmt -p miru-agent -- --check`.
6. Commit:

        git add agent/tests/data_uploads/upload/uploader.rs
        git commit -m "test(upload): assert exact retry backoff durations in timeout and retry-confirm tests"

### Milestone 2

1. Apply the Milestone 2 edit.
2. `cargo test --features test data_uploads::upload::queue` — expect `0 failed`.
3. `cargo fmt -p miru-agent -- --check`.
4. Commit:

        git add agent/tests/data_uploads/upload/queue.rs
        git commit -m "test(upload): assert QueueFullErr by type and carried fields, not message text"

### Milestone 3

1. Apply Edits 1-3 from Milestone 3.
2. `cargo test --features test data_uploads::upload` — expect `0 failed`.
3. Perturbation: change `transfer_s3`'s missing-credentials error to `TransferErr::UnrecognizedScheme`, rerun, confirm `s3_scheme_without_credentials_errs` FAILS and `unknown_scheme_is_unsupported` passes. Restore with `git checkout -- agent/src/data_uploads/upload/transfer.rs` and rerun to `0 failed`.
4. `./scripts/covgate.sh` — expect the `data_uploads/upload` module at or above 96.00.
5. `cargo fmt -p miru-agent -- --check` and `./scripts/lint.sh`.
6. Commit:

        git add agent/src/data_uploads/upload/errors.rs agent/src/data_uploads/upload/transfer.rs agent/tests/data_uploads/upload/transfer.rs
        git commit -m "refactor(upload): add TransferErr so transfer preconditions are assertable by type"

### Milestone 4 — gates and push

1. `./scripts/preflight.sh`.
2. Known flake: preflight sometimes reports one failing component under its own parallelism while every component passes alone. If that happens, rerun `./scripts/lint.sh`, `./scripts/test.sh`, `./scripts/covgate.sh` individually, record which component flaked, then rerun preflight once more. Only a reproducible failure counts.
3. `git push -u origin test/upload-assertion-quality`.
4. Open the PR as a draft. It leaves draft only once preflight reports CLEAN and CI is green on the pushed branch head.

## Validation and Acceptance

- `cargo test --features test data_uploads::upload` reports `0 failed`, and the run includes `hung_attempt_times_out_and_is_retried`, `retention_producer::retried_upload_enqueues_once_at_confirm_time`, `enqueue::full_queue_returns_queue_full_err`, `unknown_scheme_is_unsupported`, `s3_scheme_without_credentials_errs`, `gcs_scheme_without_credentials_errs`.
- **Constraint proof (non-optional).** With `entry.attempts - 1` changed to `entry.attempts`, the two backoff tests fail (`left: [2s]`, `right: [1s]`); restored, they pass. With `transfer_s3`'s missing-credentials error changed to `TransferErr::UnrecognizedScheme`, `s3_scheme_without_credentials_errs` fails and `unknown_scheme_is_unsupported` still passes; restored, all three pass. A tightened assertion that passes both before and after its perturbation has not been tightened — this has already happened once in this codebase, which is why the check is mandatory. Both results recorded in Surprises & Discoveries.
- No behavior change: `TransferErr` is constructed only through `executor_err`, which hardcodes `is_terminal: false` and `is_network_conn_err: false`, so the uploader's terminal/network/attempt-budget handling is byte-identical; `Display` strings match the previous literals, so log and error text are unchanged.
- `cargo fmt -p miru-agent -- --check` produces no output; `./scripts/lint.sh` passes; `./scripts/covgate.sh` passes with `.covgate` unchanged at 96.00.
- **`./scripts/preflight.sh` reports CLEAN and CI is green on the pushed branch head before the PR leaves draft.**

## Idempotence and Recovery

Every step is a source edit plus a read-only command, all safely repeatable. The riskiest steps are the two deliberate perturbations, because a perturbed constant must never be committed:

- Perturb only files with no other pending edits in that milestone, so `git checkout -- <file>` restores exactly.
- After each restore, run `git diff -- agent/src/` and confirm it shows only the intended changes (empty during Milestone 1).
- Before every commit, run `git diff --cached` and confirm no perturbation is staged.

To abandon: `git reset --hard c1ebf64` on `test/upload-assertion-quality`. Individual milestones can be undone with `git revert <sha>` since each is its own commit.
