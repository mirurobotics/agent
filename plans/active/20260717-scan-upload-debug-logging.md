# Add tracing logs to the scan and upload modules for runtime observability

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries,
Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Add purely-additive `tracing` log lines (`info!`/`debug!`/`trace!`) to the scan module (`agent/src/scan/`) and upload module (`agent/src/upload/`). Extend existing tests only where a new log line would drop per-module line coverage below its covgate. |

This plan lives in the agent repo because every change is inside it. Base branch is
`main`. Do the work on the already-checked-out branch
`claude/agent-upload-debug-logging-gk05bx` (created from `main`). Do **not** create or
switch to any other branch.

This is an **observability-only** change. No behavior, control flow, function
signatures, or public API changes. Every edit either (a) inserts a `tracing` macro
call, (b) extends a scoped `use tracing::{...}` import to cover the macros a file now
uses, or (c) adds/extends a test purely to keep an already-exercised line covered.

## Purpose / Big Picture

The agent's uploading feature spans two actor-based modules — the scanner
(`agent/src/scan/`) discovers files that have become stable on disk and emits
them as upload candidates; the uploader (`agent/src/upload/`) mints uploads against
the backend, transfers bytes to cloud object storage (S3/GCS), and confirms them.
Both are structured as a Tokio `mpsc` command loop driving an owned, single-threaded
core. Today the runtime behavior of these paths is hard to follow from logs: only
sparse `warn!`/`error!` lines and a couple of lifecycle `info!` lines exist. When an
upload silently fails to appear, or a file is unexpectedly considered unstable, an
engineer has almost no trace of the per-item decisions the modules made.

After this change, an engineer running the agent with `RUST_LOG=debug` (or `trace`)
can follow: which deployment/rules the scanner is configured with; per-collection
candidate discovery and stability transitions; how many stable files each scan tick
emits; and, on the upload side, each job's lifecycle through mint → transfer →
confirm → delete, including backoff/requeue decisions and queue depth. High-level
milestones use `info!`; per-item/per-step diagnostics use `debug!`; very fine detail
uses `trace!`.

Non-goals: no metrics, no spans/instrumentation attributes, no new log sinks, no
changes to what the modules *do*. Sensitive data is never logged (see the Redaction
rule below).

## Progress

- [ ] Milestone 1: scan module logs (`scanner.rs`, `collection.rs`, `state.rs`).
- [ ] Milestone 2: upload module logs (`uploader.rs`, `executor.rs`, `transfer.rs`, `queue.rs`).
- [ ] Milestone 3: coverage reconciliation — run `./scripts/covgate.sh`, extend tests where a new log line dropped coverage, until both covgates pass; full preflight CLEAN.

## Surprises & Discoveries

- (placeholder — record anything the implementer learns that contradicts this plan)

## Decision Log

- (placeholder — record non-obvious choices with rationale, date, and author)

## Outcomes & Retrospective

- (placeholder — fill in when the work is complete)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` at `/home/user/agent` (a Rust
workspace; the binary crate is `miru-agent` under `agent/`). All paths below are
repo-relative; **every command in this plan runs from the repo root** unless stated.
The real agent instructions file is `AGENTS.md` (`CLAUDE.md` is a symlink to it).

### Conventions this change must match EXACTLY

These are drawn from the existing log lines in the two modules. Deviating from them
fails CI (import linter, `cargo fmt --check`, or `cargo clippy -D warnings`).

- **Import placement.** Files use three import groups with comment headers:
  `// standard crates`, `// internal crates`, `// external crates`. `tracing` is a
  third-party crate, so its `use tracing::{...}` belongs in the `// external crates`
  group, kept alphabetically among the other `use` lines there. Any file that gains a
  `debug!`/`info!`/`trace!` call must import exactly those macros (unused import =
  clippy `-D warnings` failure; a used-but-unimported macro = compile error).
  - `scanner.rs:19` already has `use tracing::{debug, error, info, warn};` — extend to
    add `trace` only if you actually add a `trace!` there.
  - `collection.rs:16` has `use tracing::warn;` — widen to
    `use tracing::{debug, warn};` (keep members alphabetical).
  - `uploader.rs:20` has `use tracing::{error, info};` — widen to
    `use tracing::{debug, error, info};`.
  - `executor.rs:17` has `use tracing::warn;` — widen to `use tracing::{debug, warn};`.
  - `queue.rs:15` has `use tracing::warn;` — widen to `use tracing::{debug, warn};`.
  - `state.rs` and `transfer.rs` have **no** tracing import today. Add a new
    `use tracing::debug;` (or `use tracing::{debug, trace};` if a `trace!` is added) in
    the `// external crates` group. Note: `transfer.rs` currently lists its
    `backend_api::models::...` imports without a separate `// external crates` header
    (they sit under `// internal crates`). Do not reorganize the existing lines beyond
    what the import linter requires; add the `tracing` import where the linter accepts
    it and run the linter (Milestone-end validation) to confirm placement. If the
    linter flags grouping, add/correct the `// external crates` header as the linter
    directs — this is the one structural exception allowed, and only if the linter
    forces it.
- **Message style.** Lowercase message text. Inline captured interpolation only:
  `debug!("scan: emitted {count} stable files")`, `debug!("upload: job {file} digest
  {digest}")`. Use `{var}` for `Display` and `{var:?}` for `Debug`. **No** structured
  key=value fields (e.g. `debug!(count, "...")`) — none exist in these modules today,
  do not introduce the pattern.
- **Domain prefixes.** Scan-domain messages are prefixed `scan: ` and upload-domain
  messages `upload: `, matching existing lines (`"scan: evaluate failed for collection
  {cid}: {err}"`, `"upload: failed to persist upload queue: {err}"`). Pure lifecycle
  lines that already omit the prefix (e.g. `"Scanner shutdown complete"`) may stay
  unprefixed if you mirror an existing sibling; prefer the domain prefix for new
  domain messages.
- **`crate::trace!` is NOT the log macro.** Both modules `use crate::trace;`. That is
  Miru's *error-trace* macro (attaches call-site context to an `errs::Error`), unrelated
  to `tracing`. Do not call it, remove it, or confuse it with `tracing::trace!`. When
  you write a fine-grained log you must qualify or import `tracing::trace` — never let
  a bare `trace!` resolve to `crate::trace`.

### Redaction rule (security — never log these)

Never emit, in any log line: an auth `Token` or `token.token`; cloud credentials
(S3 `access_key_id` / `secret_access_key` / `session_token`, GCS `access_token`); any
`UploadCredentials` struct (or its `Debug`); or raw file bytes / file contents. These
appear in `transfer.rs` and `executor.rs` signatures — log only the **safe
identifiers** listed next, never the credential/token arguments.

Safe to log (already logged elsewhere, non-sensitive): `job.file` / paths,
`upload_rule_id`, `deployment_id`, `digest` (the sha256 hex string), `size`,
`attempts`, `capacity`, queue length, collection id (`cid`), upload id, bucket name,
object key, scheme (S3 vs GCS), backoff `Duration`, counts.

### Existing logging inventory (do not duplicate; match tone)

- `scanner.rs`: `:105` warn persist-failed; `:116` debug no-subscribers; `:196`/`:202`
  warn evaluate/discover failed per collection; `:292`/`:323` error actor plumbing;
  `:443` info `"Scanner shutdown complete"`; `dispatch!` macro (L33) error.
- `collection.rs`: `:88` warn skip-candidate; `:139` warn skip-unreadable.
- `uploader.rs`: `dispatch!` (L26) error; `:204` error dropping requeue-failed;
  `:225` `log_success` info `"uploaded file {} (rule {}, digest {}) on attempt {}"`;
  `:232` `log_dropped` error; `:263`/`:329`/`:336` error plumbing; `:360`/`:363` info
  shutdown.
- `executor.rs`: `:84` warn delete-after-upload failed.
- `queue.rs`: `:103` warn queue full; `:124` warn persist-failed.

### The coverage gate (the critical risk)

CI enforces per-module line-coverage floors via `scripts/covgate.sh`, which reads a
`.covgate` file per module directory:

- `agent/src/scan/.covgate` = **98.83**
- `agent/src/upload/.covgate` = **97.00**

`covgate.sh` runs `cargo llvm-cov --package miru-agent --features test` with
`RUST_LOG=off` and compares each module's measured line coverage against its gate.
**Every added log line is one or more new coverage regions.** A log line on a code
path that no existing test exercises will be counted as *uncovered lines added*,
lowering the module percentage and failing CI. Because the gates are extremely high
(98.83 / 97.00), there is almost no headroom — even a couple of untested log lines can
fail the gate.

Mitigation, applied throughout Milestones 1–2: **prefer placing logs on paths existing
tests already exercise.** The placement table below marks each site with a coverage
note. For any site on an under-tested branch, Milestone 3 either confirms an existing
test covers it or adds/extends a test so the new line is covered. Tests run with
`--features test` and `RUST_LOG=off`, so log macros execute (their argument
expressions are evaluated and thus counted as covered) but emit nothing and cannot
break assertions.

### Strategic placement points

Authoritative list. Line numbers are from `main` at plan-writing time — re-read each
function before editing, since earlier edits in the same file shift later lines.

**SCAN — `agent/src/scan/scanner.rs`**

- `scan()` L186–219: `debug!` at entry (scanner count, deployed-collection count);
  `debug!` per-collection inside the L193–208 loop (`cid`, candidate/stable counts);
  `info!` (or `debug!`) at end (stable files emitted count, scanners pruned count).
  Cov: exercised by scan integration + inline tests.
- `update_rules()` L147–184: `info!` (deployment id, rule count, collection ids).
- `clear_rules()` L141–145: `info!`/`debug!` rules cleared.
- `prune()` L221–227: `debug!` (cutoff, removed count).
- `persist_snapshot()` L91–107: `debug!` on success (collections count).
- `emit_stable_files()` L113–119: `debug!` emitted count.
- `Worker::run` / `handle_command`: `debug!` per command variant (queue/actor plumbing).

**SCAN — `agent/src/scan/collection.rs`**

- `evaluate_candidates()` L73–116: `debug!` per outcome transition — `Stable` (with
  `file`, `digest`, `size`), `AlreadyInLedger`, `Unstable`/dropped,
  `WaitForStabilityWindow`.
- `discover_candidates()` / `observe_untracked()` L65–71 / L127–146: `debug!` counts
  (globbed, skipped-tracked, newly observed, promoted).
- `differs_from_previous()` L249–270: `debug!` new-version vs deduped.

**SCAN — `agent/src/scan/state.rs`** (needs a new `use tracing::debug;`)

- `set_config()` L73–84: `debug!` config set.
- `prune_ledger()` L86–93: `debug!` ledger pruned (count).

**UPLOAD — `agent/src/upload/uploader.rs`**

- `run_round()` L150–181: `debug!` job start (`file`, `digest`, `size`, `attempt`).
- `attempt_upload()` L186–197: `debug!` before attempt; `debug!`/`info!` on
  `AttemptOutcome::Failed`.
- `await_next_round()` L215–222: `debug!` backoff duration + attempt.
- `requeue()` L201–209: `info!`/`debug!` on successful requeue.
- `handle_command()` L259–284: `debug!` per command with queue length.
- `run()` idle branch L127–136: `debug!` queue drained to idle.

**UPLOAD — `agent/src/upload/executor.rs`**

- `upload()` L93–106: `debug!` each stage (create requested / returned upload id;
  transfer start / finish; confirm start / finish).
- `create_upload()` L53–65: `debug!` upload id + scheme (never the credentials).
- `delete_source_file()` L81–90: `debug!` on delete.

**UPLOAD — `agent/src/upload/transfer.rs`** (needs a new `use tracing::debug;`)

- `transfer()` L136–149: `debug!` selected scheme, bucket, object key, file.
- `transfer_s3` / `transfer_gcs` L74–115: `debug!` start/finish put with bucket, key.
  Never log the `UploadCredentials`/`S3UploadCredentials` argument.

**UPLOAD — `agent/src/upload/queue.rs`**

- `enqueue()` L77–82 / `requeue()` L86–91 / `pop_front()` L93–99: `debug!` with the
  resulting queue length.
- `persist()` L116–126: `debug!` on success (entry count).

### Tests

- Run tests: `./scripts/test.sh` (wraps `RUST_LOG=off cargo test --features test`).
- Coverage gate: `./scripts/covgate.sh` (wraps `cargo llvm-cov --features test`).
- Inline unit tests: `scanner.rs` L456+, `collection.rs` L305+, `state.rs` L171+,
  `errors.rs` L91+.
- Integration tests: `agent/tests/upload/{executor,queue,transfer,uploader}.rs`; scan
  under `agent/tests/workers/{scan,sync_scan_bridge,scan_upload_bridge}.rs`; scanner
  mock at `agent/tests/mocks/scanner.rs`. Serial tests are annotated `#[serial]`.
- The import linter also flags 4+ `assert_eq!` on fields of the same variable
  (field-by-field assert); if a coverage-driven test triggers it, prefer a
  whole-struct assertion, or add `// lint:allow(field-by-field-assert)` as the repo
  convention allows.

## Plan of Work

Three milestones, one commit each. Keep it additive and proportional — no refactoring,
no reordering of existing code, no signature changes.

### Milestone 1 — scan module logs

Add the scan-side log lines from the placement table to `agent/src/scan/scanner.rs`,
`agent/src/scan/collection.rs`, and `agent/src/scan/state.rs`. Extend/add each file's
`use tracing::{...}` to exactly the macros used. Follow the message-style and
redaction rules. Build and run tests (not yet the covgate — that is Milestone 3, but a
quick covgate check here is encouraged to catch regressions early). Commit.

### Milestone 2 — upload module logs

Add the upload-side log lines to `agent/src/upload/uploader.rs`,
`agent/src/upload/executor.rs`, `agent/src/upload/transfer.rs`, and
`agent/src/upload/queue.rs`. Same import/style/redaction discipline. In `transfer.rs`
and `executor.rs`, log only safe identifiers (bucket, key, scheme, upload id, file) —
never the credentials/token arguments. Build and run tests. Commit.

### Milestone 3 — coverage reconciliation

Run `./scripts/covgate.sh`. If either module is below its gate, identify which new log
lines sit on uncovered branches (llvm-cov output points at the file:line), then extend
the nearest existing test in the module's test files so that branch executes. Prefer
strengthening an existing test over adding a new one. Re-run the covgate; iterate until
**both** `agent/src/scan` (≥ 98.83) and `agent/src/upload` (≥ 97.00) pass. Then run the
full preflight and confirm CLEAN. Commit any test additions.

## Concrete Steps

All commands run from the repo root (`/home/user/agent`) on branch
`claude/agent-upload-debug-logging-gk05bx` (already checked out).

### Setup

    # already on branch claude/agent-upload-debug-logging-gk05bx — no branch creation needed
    git branch --show-current   # expect: claude/agent-upload-debug-logging-gk05bx

### Milestone 1 — scan logs

Step 1.1 — Re-read the target functions before editing (line numbers drift):

    sed -n '85,230p' agent/src/scan/scanner.rs
    sed -n '60,150p'  agent/src/scan/collection.rs
    sed -n '65,95p'   agent/src/scan/state.rs

Step 1.2 — Apply the scan-side log lines per the placement table. Widen
`collection.rs`'s import to `use tracing::{debug, warn};`; add
`use tracing::debug;` to `state.rs` in its `// external crates` group; extend
`scanner.rs`'s import only if you add a `trace!`. Example shapes (adapt to real
locals):

    // scanner.rs, scan() entry
    debug!("scan: tick over {scanner_count} scanners, {deployed} deployed collections");

    // collection.rs, evaluate_candidates() Stable arm
    debug!("scan: candidate stable {file} digest {digest} size {size}");

    // state.rs, prune_ledger()
    debug!("scan: pruned {removed} ledger entries");

Step 1.3 — Build and test:

    cargo build -p miru-agent --features test
    ./scripts/test.sh

Expected (abridged): build clean; the scan-related suites pass with an unchanged test
count, e.g.

    test result: ok. NN passed; 0 failed; ...

Step 1.4 — (recommended early check) confirm the scan gate did not regress:

    ./scripts/covgate.sh

If `agent/src/scan` is below 98.83, note the offending lines for Milestone 3 (you may
fix now or defer). Do not weaken any log to pass; cover the line instead.

Step 1.5 — Lint, then commit:

    LINT_FIX=0 ./scripts/lint.sh
    git add agent/src/scan/scanner.rs agent/src/scan/collection.rs agent/src/scan/state.rs
    git commit -m "feat: add scan lifecycle and per-item tracing logs"

### Milestone 2 — upload logs

Step 2.1 — Re-read targets:

    sed -n '120,290p' agent/src/upload/uploader.rs
    sed -n '45,110p'  agent/src/upload/executor.rs
    sed -n '70,150p'  agent/src/upload/transfer.rs
    sed -n '75,130p'  agent/src/upload/queue.rs

Step 2.2 — Apply the upload-side log lines. Widen imports: `uploader.rs` →
`use tracing::{debug, error, info};`; `executor.rs` → `use tracing::{debug, warn};`;
`queue.rs` → `use tracing::{debug, warn};`; add `use tracing::debug;` to
`transfer.rs`. Log only safe identifiers. Example shapes:

    // uploader.rs, run_round()
    debug!("upload: job {file} digest {digest} size {size} attempt {attempt}");

    // executor.rs, create_upload()
    debug!("upload: created upload {upload_id} scheme {scheme:?}");

    // transfer.rs, transfer_s3()
    debug!("upload: s3 put start bucket {bucket} key {key}");

    // queue.rs, enqueue()
    debug!("upload: enqueued, queue length {len}");

Step 2.3 — Build and test:

    cargo build -p miru-agent --features test
    ./scripts/test.sh

Expected: build clean; upload suites pass, unchanged test count.

Step 2.4 — Lint, then commit:

    LINT_FIX=0 ./scripts/lint.sh
    git add agent/src/upload/uploader.rs agent/src/upload/executor.rs agent/src/upload/transfer.rs agent/src/upload/queue.rs
    git commit -m "feat: add upload pipeline tracing logs"

### Milestone 3 — coverage reconciliation

Step 3.1 — Run the covgate:

    ./scripts/covgate.sh

Expected on success (abridged): both modules at or above their gates, e.g.

    agent/src/scan ... 98.9% >= 98.83 OK
    agent/src/upload ... 97.1% >= 97.00 OK

Step 3.2 — If a module is under its gate, get the uncovered lines and map them to the
new log calls:

    cargo llvm-cov --package miru-agent --features test --show-missing-lines

For each uncovered new log line, extend the nearest existing test in
`agent/src/scan/*_test.go`-equivalent inline `#[cfg(test)]` blocks
(`scanner.rs` L456+, `collection.rs` L305+, `state.rs` L171+) or the integration files
(`agent/tests/upload/*.rs`, `agent/tests/workers/*.rs`) so the branch executes. Prefer
strengthening an existing test; keep assertions whole-struct where possible to avoid
the field-by-field-assert linter rule.

Step 3.3 — Re-run until both gates pass:

    ./scripts/covgate.sh

Step 3.4 — Full local validation:

    LINT_FIX=0 ./scripts/lint.sh
    ./scripts/test.sh
    ./scripts/covgate.sh

Step 3.5 — Commit any test additions (skip if no test changes were needed):

    git add agent/src/scan agent/src/upload agent/tests
    git commit -m "test: cover new scan and upload tracing log lines"

## Validation and Acceptance

Acceptance is verifiable CI behavior, not just local runs.

1. **Preflight must report CLEAN — CI green on the pushed branch head — before the PR
   leaves draft or the task is reported complete.** CLEAN means all CI jobs in
   `.github/workflows/ci.yml` pass on the pushed head:
   - **lint job**: import linter (3-group ordering with comment headers) +
     `cargo fmt --check` + `cargo clippy` with `-D warnings` (so no unused imports and
     no clippy findings) + machete/diet/audit.
   - **test job**: `./scripts/covgate.sh` passing **both** covgates —
     `agent/src/scan` ≥ 98.83 and `agent/src/upload` ≥ 97.00.
   - **tools job**: passing.
2. **Exact local commands** (run from `/home/user/agent`) that must all succeed before
   pushing:

       LINT_FIX=0 ./scripts/lint.sh
       ./scripts/test.sh
       ./scripts/covgate.sh

   `./scripts/lint.sh` also runs with auto-fix by default; `LINT_FIX=0` mirrors CI
   (fail instead of fix). `./scripts/test.sh` runs `RUST_LOG=off cargo test --features
   test` — logs are silenced so no new log line can change a test outcome; test counts
   must be unchanged except for any tests deliberately extended in Milestone 3.
3. **Behavior unchanged.** The diff contains only: added `tracing` macro calls, widened
   `use tracing::{...}` imports, and (Milestone 3) test additions. No signature,
   control-flow, or public-API change. Reviewer can confirm by reading the diff.
4. **Redaction respected.** No log line references a token, cloud credential,
   `UploadCredentials`, or raw file bytes. Grep the diff to confirm:

       git diff main -- agent/src/scan agent/src/upload | grep -nEi 'token|secret|access_key|session_token|credential' || echo "no sensitive refs"

   Expected: `no sensitive refs` (any hit must be a false positive on a comment/name,
   reviewed by hand).

## Idempotence and Recovery

- Every edit is additive and independently re-appliable. Re-reading a function and
  re-inserting a log line that is already present is a no-op once you notice it — check
  before duplicating.
- Build/test/lint/covgate steps are read-only and safe to re-run any number of times.
- If an edit breaks the build (e.g. a `trace!` resolving to `crate::trace`, or an
  unused import), the compiler/clippy points at the exact line; fix in place.
- To discard uncommitted work on a file: `git checkout -- <path>`. The branch exists
  solely for this work, so `git reset --hard main` is a safe full rollback before push.
- Milestones are separate commits; if Milestone 3 reveals a badly placed log (on a
  hard-to-cover branch), you may move or drop that single log line and re-run the
  covgate rather than contort a test — record the choice in the Decision Log.
