# Delete the local source file after a successful upload when the rule says `after_upload`

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Thread each upload rule's `delete_policy` from the scanner through the upload pipeline and delete the local source file after a confirmed upload when the policy is `after_upload`. Changes touch `agent/src/scan/state.rs`, `agent/src/scan/collection.rs`, `agent/src/upload/job.rs`, `agent/src/upload/executor.rs`, `agent/src/workers/scan_upload_bridge.rs`, and the mirror test files under `agent/tests/`. |
| `libs/backend-api/` | read-only | Generated OpenAPI code. The synced `UploadRuleDestination` carries `delete_policy`; the create-upload response `UploadDestination` does NOT. This asymmetry is the reason the policy must be threaded from the rule rather than read from the upload response. Not modified. |

This plan lives in the agent repo because every code change is inside `agent/`. Work happens on branch `feat/upload-delete-policy` (already checked out, clean, based on `main`).

## Purpose / Big Picture

Miru upload rules let an operator choose what happens to a local file after the agent uploads it. Each rule's destination carries a **delete policy** — one of `never` (the default) or `after_upload`. Today the agent uploads the file and always leaves it on disk, regardless of the policy, so `after_upload` has no effect.

After this change, the delete policy is honored end to end:

- `never` (default): after a successful upload, the local source file stays in place. This is today's behavior and must not change.
- `after_upload`: after a successful upload — one that has been durably confirmed with the backend — the agent deletes the local source file.

You can see it work by running the test suite: a new executor test writes a real temp file, runs the production upload path against a replayed/mocked backend with an `after_upload` job, and asserts the file no longer exists on disk; the companion `never` test asserts the file remains. A delete that fails (or a file that is already gone) never turns a durable upload into a failed job.

## Progress

- [ ] (2026-07-16) Milestone 1: carry `delete_policy` on `StableFile` (built from the rule) with a backward-compatible serde default; test + covgate + commit.
- [ ] (2026-07-16) Milestone 2: carry `delete_policy` on `Job`, copy it in the scan-upload bridge; test + covgate + commit.
- [ ] (2026-07-16) Milestone 3: delete the source file in `LiveExecutor::upload` after `confirm_upload` when the policy is `after_upload`, best-effort; test + covgate + commit.
- [ ] (2026-07-16) Milestone 4: preflight to CI-green on the pushed branch head.

Split partially completed work into "done" and "remaining" as needed and add timestamps as steps complete.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

(Add entries as you go. Pre-authoring decisions that shaped this plan are recorded below so a novice understands the "why".)

- Decision: Thread `delete_policy` from the rule through `StableFile` → `Job` rather than reading it from the create-upload response.
  Rationale: The create-upload response type `UploadDestination` (`libs/backend-api/src/models/upload_destination.rs`) has only `bucket_id`, `bucket_name`, `object_key` — no `delete_policy`. Only the synced `UploadRuleDestination` (`agent/src/models/upload_rule.rs`) carries it. The scanner already has the full rule via `CollectionState.cfg.rule`, so stamping it onto `StableFile` at stabilization time is the natural source. Date/Author: 2026-07-16 / ben@miruml.com.
- Decision: Perform the deletion inside `LiveExecutor::upload`, after `confirm_upload` succeeds, reading `job.delete_policy`.
  Rationale: That is the single point where the upload is known to be durably confirmed with the backend, and the executor already owns `job.file`. Putting it in the `Uploader` actor's success path would duplicate the "is it durable yet" knowledge that only the executor has. The `UploadExecutor` cancel-safety contract already tolerates being dropped at any await point, and a delete performed after `confirm_upload` is idempotent (see Idempotence and Recovery). Date/Author: 2026-07-16 / ben@miruml.com.
- Decision: A delete failure is logged and swallowed; the job still reports `Ok`.
  Rationale: The upload has already durably succeeded. Propagating a delete error would make the `Uploader` actor treat the job as failed and re-drive it, re-uploading an already-durable file. Correctness of the upload does not depend on the local delete. Date/Author: 2026-07-16 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (a Rust workspace; the binary crate is `miru-agent` under `agent/`). All paths below are relative to the repo root; all commands run from the repo root. Branch `feat/upload-delete-policy` is already checked out and clean.

The **upload pipeline** moves a file from the filesystem scanner to the backend:

1. The scanner watches configured globs. When a file has been stable long enough it emits `ScanEvent::StableFile(StableFile)`.
2. `agent/src/workers/scan_upload_bridge.rs::enqueue_stable_file` turns each `StableFile` into a `Job` and enqueues it on the `Uploader` actor.
3. The `Uploader` actor pops each `Job` and calls `executor.upload(&job)`.
4. `agent/src/upload/executor.rs::LiveExecutor::upload` does `create_upload` (mint an upload + credentials with the backend) → `transfer` (push the bytes to object storage) → `confirm_upload` (tell the backend the object landed). After `confirm_upload` returns `Ok`, the upload is **durable** — the backend has recorded it.

Key files and current state (verified against `main`):

- `agent/src/models/upload_rule.rs` — defines `DeletePolicy`:

      #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
      #[serde(rename_all = "snake_case")]
      pub enum DeletePolicy {
          #[default]
          Never,
          AfterUpload,
      }

  The struct derives only `Serialize`, but `impl_status_enum!` (invoked just below the enum) generates a manual `impl<'de> serde::Deserialize<'de> for DeletePolicy` (unknown wire value → default `Never`, with a log). So `DeletePolicy` is BOTH serializable and deserializable, and `Default` is `Never`. `UploadRuleDestination` (same file) has `pub delete_policy: DeletePolicy`, populated from the backend `UploadRuleDestination` in its `From` impl. `UploadRule.destination.delete_policy` is therefore the source of truth for a rule's policy.

- `agent/src/scan/state.rs` — the scanner's persisted state. `StableFile` (lines ~122-133) is:

      #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
      pub struct StableFile {
          pub file: File,
          pub size: u64,
          pub digest: String,
          pub mtime: DateTime<Utc>,
          pub mtime_aliases: Vec<DateTime<Utc>>,
          pub first_observed_at: DateTime<Utc>,
          pub last_observed_at: DateTime<Utc>,
          pub deployment_id: String,
          pub upload_rule_id: String,
      }

  `StableFile` is PERSISTED to disk: it lives inside `CollectionState.ledger: HashMap<File, Vec<StableFile>>` (line ~19), which is inside `ScannerSnapshot` (line ~155), serialized through `ScanSnapshotFile = SingleThreadStateFile<...>` (line ~166). An agent that upgrades will read a ledger written by an older binary. **Any new field must deserialize from an old on-disk form that lacks it** — hence `#[serde(default)]` on the new field. `CollectionState.cfg: Config` (line ~16) holds `Config { deployment, rule: UploadRule }` (line ~22-26) and `CollectionState::rule(&self) -> &UploadRule` (line ~38) returns `&self.cfg.rule`. `#[derive(Eq)]` on `StableFile` is fine — `DeletePolicy` derives `Eq`.

- `agent/src/scan/collection.rs` — `build_stable_file(candidate: &Candidate, last_observed_at: DateTime<Utc>, digest: Digest) -> StableFile` (lines ~271-288) constructs the `StableFile`. It does NOT currently receive `state`, so it cannot reach the rule. Its only caller, `differs_from_previous(state: &CollectionState, candidate, observation)` (lines ~249-269), DOES have `state`, and can read `state.rule().destination.delete_policy`. The plan adds a `delete_policy: DeletePolicy` parameter to `build_stable_file` and passes it from the call site.

- `agent/src/upload/job.rs` — the whole file is the `Job` struct, deriving `Clone, Debug, PartialEq`, constructed by struct literal (no serde, no constructor). It currently has: `file, size, digest, mtime, first_observed_at, last_observed_at, upload_rule_id, deployment_id`.

- `agent/src/workers/scan_upload_bridge.rs::enqueue_stable_file` (lines ~70-85) builds a `Job` from a `StableFile` field-by-field via struct literal, then enqueues it.

- `agent/src/upload/executor.rs` — the `UploadExecutor` trait carries a `# Cancel safety` doc block: implementations must tolerate being cancelled at any await point; an interrupted transfer is re-driven after restart via scanner re-observation plus backend digest dedup. `LiveExecutor::upload` (lines ~78-88) is `create_upload` → `transfer` → `confirm_upload` → `Ok(())`. The deletion hook goes right after `confirm_upload` succeeds, before the final `Ok(())`.

- `agent/src/filesys/files.rs::delete` (lines ~299-309):

      pub async fn delete(file: &File) -> Result<(), FileSysErr> {
          match tokio::fs::remove_file(file.path()).await {
              Ok(()) => Ok(()),
              Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
              Err(e) => Err(FileSysErr::DeleteFileErr(...)),
          }
      }

  **A missing file is already `Ok(())`** — so the "missing source file at delete time → success" requirement is satisfied by `files::delete` itself and needs no extra branching in the executor. A genuine delete failure (e.g. permission error) returns `Err`, which the executor must log and swallow.

- `libs/backend-api/src/models/upload_destination.rs` — the create-upload response `UploadDestination` has only `bucket_id`, `bucket_name`, `object_key`. Confirmed no `delete_policy`. Read-only.

Test layout and mocks (verified):

- Tests run via `./scripts/test.sh`, which runs `RUST_LOG=off cargo test --features test`. The `--features test` flag is MANDATORY — many mocks/helpers are behind `#[cfg(feature = "test")]`; bare `cargo test` fails with misleading "missing helper" errors.
- Test files under `agent/tests/` mirror `agent/src/`: `agent/tests/upload/executor.rs`, `agent/tests/workers/scan_upload_bridge.rs`. Scanner tests for `state.rs`/`collection.rs` are IN-SRC `#[cfg(test)] mod tests` at the bottom of those files (there is no `agent/tests/scan/` directory).
- `agent/tests/upload/executor.rs` already has a `make_job(name)` helper (struct literal) and an `end_to_end_with_sdk_transfer_over_replayed_s3` test that writes a REAL temp file via `miru_agent::filesys::files::temp(...)` + `files::write_bytes(...)` and runs the production `LiveExecutor` over a replayed S3 exchange. That is the template for the delete tests. `MockObjectTransfer` (`agent/tests/mocks/object_transfer.rs`) is a scripted `ObjectTransfer` double: `push_ok()`, `push_err()`, `recorded_calls()`; an empty script defaults to `Ok`. `MockClient` has `set_create_upload(...)` / `set_confirm_upload(...)`.
- `agent/tests/workers/scan_upload_bridge.rs` has a `stable_file(name, deployment_id, rule_id)` helper (struct literal) and asserts the whole `Job` with a single `assert_eq!(expected, jobs[0])`.
- Coverage gates (`.covgate` files, minimum coverage %): `agent/src/upload/.covgate` = 97.00, `agent/src/scan/.covgate` = 98.83, `agent/src/workers/.covgate` = 84.67, `agent/src/models/.covgate` = 100. `scripts/covgate.sh` enforces them. NOTE: `agent/src/workers/.covgate` is known to fail LOCALLY (~83.13 vs gate 84.67) even on branches that don't touch workers — a pre-existing local-vs-CI environment gap. Do NOT "fix" it by lowering the gate; rely on CI for the authoritative workers number.

Lint conventions (enforced by `scripts/lint.sh` and CI):

- Import groups in every source file, blank-line separated with a comment header: `// standard crates`, `// internal crates`, `// external crates`. Keep new imports in the right group.
- A field-by-field assert linter flags 4+ `assert_eq!` on fields of the SAME variable in one test function. Prefer asserting the whole struct with one `assert_eq!`. If you must assert fields individually, put `// lint:allow(field-by-field-assert)` inside the test body.

CI (`.github/workflows/ci.yml`) has three jobs: `lint` (runs `LINT_FIX=0 ./scripts/lint.sh`), `test` (runs `./scripts/covgate.sh`, which runs the tests and enforces the gates), and `tools` (lints/tests the `tools/lint` helper crate). CI on GitHub Actions is the authoritative gate.

Out of scope: any change under `libs/`; the object-storage transfer mechanics; the `Uploader` actor's queue/retry logic; and any UI/backend change to how policies are authored.

## Plan of Work

The change is threaded producer-first so each milestone compiles and is independently testable.

### Milestone 1 — `StableFile` carries the policy (built from the rule)

1. In `agent/src/scan/state.rs`, add a field to `StableFile` after `upload_rule_id`, annotated for backward-compatible deserialization of old persisted ledgers:

       /// What to do with the local source file after a successful upload.
       /// Stamped from the rule's destination (`UploadRule.destination.delete_policy`)
       /// at stabilization time. `#[serde(default)]` keeps ledgers written by
       /// older agents (which lack this field) deserializable — they default to
       /// `DeletePolicy::Never`, the safe no-op.
       #[serde(default)]
       pub delete_policy: DeletePolicy,

   Add `use crate::models::DeletePolicy;` — extend the existing `use crate::models::{...}` line (it already imports `UploadRule` etc.) rather than adding a new `use`, keeping it in the `// internal crates` group. `DeletePolicy` derives `Eq`, so `StableFile`'s `#[derive(..., Eq, ...)]` still holds.

2. In `agent/src/scan/collection.rs`, add a `delete_policy: DeletePolicy` parameter to `build_stable_file` and set the new struct field from it:

       fn build_stable_file(
           candidate: &Candidate,
           last_observed_at: DateTime<Utc>,
           digest: Digest,
           delete_policy: DeletePolicy,
       ) -> StableFile {
           ...
           StableFile {
               ...
               upload_rule_id: first_obs.upload_rule_id.clone(),
               delete_policy,
           }
       }

   At the single call site in `differs_from_previous`, pass the rule's policy:

       Ok(Outcome::Stable(build_stable_file(
           candidate,
           observation.timestamp,
           digest,
           state.rule().destination.delete_policy,
       )))

   Add `use crate::models::DeletePolicy;` to the file's `// internal crates` imports if `DeletePolicy` is not already in scope there.

3. Fix every in-src `StableFile { ... }` construction site. Known ones (as of research on `main`): `agent/src/scan/state.rs`'s `#[cfg(test)] mod tests` `stable_file(...)` helper; `agent/src/scan/collection.rs`'s `#[cfg(test)] mod tests` (`stable_file(...)`, `stable_from_obs(...)` and other literals); and a `StableFile { ... }` literal in `agent/src/scan/scanner.rs` (around line 904). Treat this list as a hint, not the authority — the compiler is the authority: build with `--features test` and fix each "missing field `delete_policy`" error wherever it appears. Add `delete_policy: DeletePolicy::Never` (or the value the test wants). Import `DeletePolicy` (`use crate::models::DeletePolicy;`) in any src file that gains the reference.

### Milestone 2 — `Job` carries the policy; the bridge copies it

1. In `agent/src/upload/job.rs`, add a field to `Job` after `upload_rule_id` (or grouped with `deployment_id` — placement is cosmetic):

       /// What to do with the local source file after a successful upload.
       /// Copied from `StableFile.delete_policy` by the scan-upload bridge and
       /// read by the executor after the upload is confirmed durable.
       pub delete_policy: DeletePolicy,

   Add `use crate::models::DeletePolicy;` to the `// internal crates` group (the file currently imports only `crate::filesys::File`).

2. In `agent/src/workers/scan_upload_bridge.rs::enqueue_stable_file`, copy the field:

       let job = Job {
           ...
           upload_rule_id: stable.upload_rule_id,
           deployment_id: stable.deployment_id,
           delete_policy: stable.delete_policy,
       };

3. Fix every `Job { ... }` struct-literal construction site the compiler flags. `Job` is constructed only in tests; known sites (as of research on `main`): `make_job` and the literal in `create_request_maps_job_fields` in `agent/tests/upload/executor.rs`; `make_job`, `make_real_job`, and any literals in `agent/tests/upload/uploader.rs`; `make_job` in `agent/tests/upload/queue.rs`; and the `expected` literal in `agent/tests/workers/scan_upload_bridge.rs::stable_file_becomes_upload_job`. Treat this list as a hint — build with `--features test` and add `delete_policy: DeletePolicy::Never` wherever the compiler reports "missing field `delete_policy`". Import `DeletePolicy` (via `miru_agent::models::DeletePolicy`) in each test file that gains the reference.

### Milestone 3 — delete after a confirmed upload

1. In `agent/src/upload/executor.rs::LiveExecutor::upload`, after `confirm_upload` succeeds and before the final `Ok(())`, delete the source file when the policy asks for it, best-effort:

       self.confirm_upload(&resp.upload.id).await?;

       // The upload is now durable (confirmed with the backend). Honor the
       // rule's delete policy. This is best-effort: a delete failure must NOT
       // fail the job — that would re-drive an already-durable upload. A
       // missing file is already Ok(()) inside files::delete, covering the
       // re-observed-after-a-prior-interrupted-attempt case (idempotent).
       if job.delete_policy == DeletePolicy::AfterUpload {
           if let Err(e) = files::delete(&job.file).await {
               warn!(
                   "upload for {} confirmed but deleting the local source file failed: {e:?}",
                   job.file
               );
           }
       }
       Ok(())

   Add the needed imports to the file's groups: `use crate::filesys::files;` and `use crate::models::DeletePolicy;` in `// internal crates`, and `use tracing::warn;` in `// external crates`. (Do not delete on the `never` branch — leave the file untouched.)

2. If, and only if, moving the delete into `upload` triggers a clippy `too_many_lines`/complexity lint, extract a small private helper `async fn delete_source_if_policy(job: &Job)` on `LiveExecutor` and call it. Prefer inline; only extract if the linter demands it.

## Concrete Steps

All commands run from the repo root on branch `feat/upload-delete-policy`.

### Milestone 1

Step 1 — make the edits in Plan of Work → Milestone 1.

Step 2 — build with the test feature to surface every fixture that needs `delete_policy`, then fix them:

    cargo build -p miru-agent --features test

Expected before fixing fixtures: `error[E0063]: missing field 'delete_policy' in initializer of 'StableFile'` at each `StableFile { ... }` literal in the `#[cfg(test)]` modules of `agent/src/scan/state.rs` and `agent/src/scan/collection.rs`. Fix each, then rebuild clean.

Step 3 — add the backward-compat serde test. In `agent/src/scan/state.rs`'s `#[cfg(test)] mod tests`, add a test proving an old on-disk `StableFile` (JSON without `delete_policy`) deserializes to `Never`, and (optionally in the same test) that a full `CollectionState`/`ScannerSnapshot` JSON lacking the field round-trips. Sketch:

    #[test]
    fn stable_file_without_delete_policy_defaults_to_never() {
        // JSON as written by an older agent: no `delete_policy` field.
        let json = r#"{
            "file": "/data/a.mcap",
            "size": 4,
            "digest": "sha256:deadbeef",
            "mtime": "1970-01-01T00:00:00Z",
            "mtime_aliases": [],
            "first_observed_at": "1970-01-01T00:00:00Z",
            "last_observed_at": "1970-01-01T00:00:00Z",
            "deployment_id": "d",
            "upload_rule_id": "coll"
        }"#;
        let parsed: StableFile = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.delete_policy, DeletePolicy::Never);
    }

Confirm the exact `File` serialized form first (run a tiny throwaway `serde_json::to_string` of a `StableFile` in the test, or inspect an existing serde test) so the `"file"` field shape in the JSON matches — adjust if `File` serializes as an object rather than a bare string.

Step 4 — add a `build_stable_file` policy test (or extend an existing stabilization test) in `agent/src/scan/collection.rs`'s `#[cfg(test)] mod tests`: build a `Config` whose `rule.destination.delete_policy` is `AfterUpload`, drive a file to `Outcome::Stable`, and assert the resulting `StableFile.delete_policy == DeletePolicy::AfterUpload`; a second case with the default rule (`Never`) asserts `Never`. The existing `rule(...)`/`config(...)` fixtures build a default destination (`Never`); set the destination policy explicitly for the `AfterUpload` case.

Step 5 — test + covgate for the touched modules:

    ./scripts/test.sh
    ./scripts/covgate.sh

Expected: suite passes; `scripts/covgate.sh` reports `scan` and `models` at/above their gates (`98.83`, `100`).

Step 6 — commit (end of Milestone 1):

    git add agent/src/scan/state.rs agent/src/scan/collection.rs
    git commit -m "feat(scan): stamp delete_policy onto StableFile from the rule"

### Milestone 2

Step 1 — make the edits in Plan of Work → Milestone 2.

Step 2 — build with the test feature and fix every flagged `Job { ... }` site:

    cargo build -p miru-agent --features test

Expected before fixing: `missing field 'delete_policy'` at `make_job` and the two literals in `agent/tests/upload/executor.rs`, and at `expected` in `agent/tests/workers/scan_upload_bridge.rs`. Add `delete_policy: DeletePolicy::Never` to each; import `DeletePolicy` in those files.

Step 3 — extend the bridge test. In `agent/tests/workers/scan_upload_bridge.rs`, set the `stable_file(...)` fixture's `delete_policy` to `AfterUpload` (either add a parameter or set it on the value under test) and assert the emitted `Job.delete_policy == DeletePolicy::AfterUpload`. Keep the whole-struct `assert_eq!(expected, jobs[0])` (extend `expected` with the matching `delete_policy`) so the copy is covered without tripping the field-by-field assert linter.

Step 4 — test + covgate:

    ./scripts/test.sh
    ./scripts/covgate.sh

Expected: suite passes; `upload` gate `97.00` still met. (If the local `workers` gate `84.67` fails at ~83.13, that is the known pre-existing local-vs-CI gap — do not lower the gate; CI is authoritative.)

Step 5 — commit (end of Milestone 2):

    git add agent/src/upload/job.rs agent/src/workers/scan_upload_bridge.rs agent/tests/upload/executor.rs agent/tests/workers/scan_upload_bridge.rs
    git commit -m "feat(upload): carry delete_policy on Job through the scan-upload bridge"

### Milestone 3

Step 1 — make the edits in Plan of Work → Milestone 3.

Step 2 — add executor tests in `agent/tests/upload/executor.rs`, modeled on `end_to_end_with_sdk_transfer_over_replayed_s3` (real temp file on disk). Four cases:

- `after_upload_deletes_source_after_confirm`: `make_job` with `delete_policy = AfterUpload`, `job.file` pointing at a written temp file; `MockClient::set_create_upload`/`set_confirm_upload` set to succeed and `MockObjectTransfer` defaulting to `Ok`; run `executor.upload(&job).await.unwrap()`; assert the file no longer exists (`assert!(!path.exists())`).
- `never_leaves_source_in_place`: same setup but `delete_policy = Never`; assert the file still exists.
- `delete_failure_after_confirm_still_succeeds`: force `files::delete` to fail after a successful confirm, and assert `executor.upload(&job).await` is `Ok`. Note the temp file lives on a `tempfile::NamedTempFile` whose `Drop` deletes it, so `files::delete` returns `Ok(())` on an already-absent path rather than erroring. To exercise a genuine delete error, point `job.file` at a path whose PARENT is not a directory (e.g. a temp file `t`, then `job.file = <t>/child`), so `remove_file` fails with `NotADirectory` (not `NotFound`) — `files::delete` returns `Err`, the executor logs and swallows it, and `upload` still returns `Ok`. Assert `client.call_count(Call::ConfirmUpload) == 1` to prove confirm ran (upload was durable) and the result is `Ok`.
- `missing_source_at_delete_is_success`: `delete_policy = AfterUpload` with `job.file` at a path that does not exist; run `executor.upload(&job).await.unwrap()` — succeeds because `files::delete` treats `NotFound` as `Ok`. (This case may be redundant with the first once the temp guard has dropped; keep it explicit for clarity.)

  Use `miru_agent::filesys::files` and `WriteOptions` (already imported in this test file) for temp-file setup; import `DeletePolicy` via `miru_agent::models::DeletePolicy`. Avoid 4+ `assert_eq!` on the same variable's fields in any one test (field-by-field assert linter).

Step 3 — test + covgate:

    ./scripts/test.sh
    ./scripts/covgate.sh

Expected: suite passes; `agent/src/upload/.covgate` (97.00) met, since the new executor branch (both `after_upload` delete and the swallowed-error path) is exercised.

Step 4 — commit (end of Milestone 3):

    git add agent/src/upload/executor.rs agent/tests/upload/executor.rs
    git commit -m "feat(upload): delete local source after confirmed upload when policy is after_upload"

### Milestone 4 — preflight to CI-green

Step 1 — local preflight (fast feedback, not authoritative):

    ./scripts/preflight.sh

Expected: the script runs `scripts/lint.sh` and `scripts/covgate.sh` (plus the `tools/lint` equivalents) in parallel and prints their logs; it should end reporting clean, MODULO the known `agent/src/workers/.covgate` local gap (~83.13 vs 84.67). If workers coverage is the only failure and only locally, proceed — CI computes the authoritative number.

Step 2 — push the branch and drive CI to green (see Validation and Acceptance). Do NOT take the PR out of draft or report the task complete until preflight/CI is CLEAN on the pushed branch head.

## Validation and Acceptance

Acceptance is observable behavior plus a green CI run.

Behavioral acceptance (via tests, run with `./scripts/test.sh`):

1. `build_stable_file` stamps the rule's policy: the new `collection.rs` test drives a file to stable under an `after_upload` rule and observes `StableFile.delete_policy == AfterUpload`; under the default rule it observes `Never`. These tests fail before the Milestone 1 edit (the field does not exist / is not set) and pass after.
2. Backward compatibility: the new `state.rs` test deserializes an old ledger JSON with no `delete_policy` and observes `DeletePolicy::Never`. This proves an upgraded agent reading a pre-existing on-disk ledger does not crash and defaults to the safe no-op.
3. The scan-upload bridge copies the policy: the `scan_upload_bridge.rs` test emits a `StableFile { delete_policy: AfterUpload, .. }` and observes the enqueued `Job.delete_policy == AfterUpload`.
4. The executor honors the policy after a durable upload:
   - `after_upload` + successful upload → the source file is gone (`!path.exists()`).
   - `never` + successful upload → the source file remains (`path.exists()`).
   - delete error after a successful `confirm_upload` → `executor.upload(&job)` still returns `Ok` and `ConfirmUpload` ran exactly once (no propagated error, no re-drive).
   - missing source file at delete time → `executor.upload(&job)` returns `Ok`.

Run: `./scripts/test.sh` and expect all tests to pass (the four executor tests, the bridge test, the two scan tests are new and pass only after their respective milestone edit). Then `./scripts/covgate.sh` and expect the `upload`, `scan`, and `models` gates met.

CI acceptance (authoritative): **preflight must report `CLEAN` — i.e. CI must be green on the pushed branch head — before the PR leaves draft or the task is reported complete.** Local validation runs `scripts/lint.sh`, `scripts/test.sh`, and `scripts/covgate.sh` (bundled by `scripts/preflight.sh`), but the authoritative gate is CI on GitHub Actions (`.github/workflows/ci.yml`), whose three jobs are `lint`, `test`, and `tools`. Push the branch and confirm all three jobs are green on the exact commit at the branch head:

    git push -u origin feat/upload-delete-policy
    gh pr checks --watch   # or: gh run watch

Only when every CI job is green on the pushed head is the task complete. The known local `agent/src/workers/.covgate` shortfall (~83.13 vs 84.67) does not count as a failure if CI's workers coverage passes; treat CI as the source of truth. If any CI job fails, read its logs (`gh run view --log-failed`), fix, push, and re-watch.

## Idempotence and Recovery

Every edit is additive (one struct field, one function parameter, one guarded delete block) and safe to re-apply — re-running a step once the field/param/block exists is a no-op. Build/test/covgate/preflight are read-only.

Runtime idempotence (the reason the design is safe): the delete runs only AFTER `confirm_upload` durably succeeds, and `files::delete` treats a missing file as `Ok`. If `executor.upload` is cancelled on shutdown before the delete, the file remains; the scanner re-observes it on the next run, backend digest dedup recognizes the already-uploaded content, and the confirm-then-delete runs again harmlessly. A genuine delete failure is logged and swallowed, so it never re-drives an already-durable upload. `never` never touches the file.

Recovery during implementation: before any commit, `git checkout -- <files>` restores a clean tree. The branch exists only for this work, so `git reset --hard main` is a safe full rollback prior to push. After a commit, `git revert <sha>` or `git reset --hard <prev-sha>` (pre-push) backs out a milestone; each milestone is its own commit specifically so it can be reverted or bisected independently.
