# Stamp the backend's upload metadata onto the uploaded object (S3 / GCS)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Thread the `metadata` map from the backend's create-upload response through the transfer layer and stamp it as user-defined cloud object metadata on every uploaded object. Changes touch `agent/src/s3/mod.rs`, `agent/src/s3/multipart.rs`, `agent/src/gcs/mod.rs`, `agent/src/upload/transfer.rs`, `agent/src/upload/executor.rs`, and the mirror test files under `agent/tests/`. |
| `libs/backend-api/` | read-only | Generated OpenAPI code. `UploadWithCredentials` already carries the required `metadata` field, so no spec revendor or regeneration is needed. Not modified. |

This plan lives in the agent repo because every code change is inside `agent/`. Work happens on branch `feat/store-upload-metadata` (already checked out, clean, based on `main` at `1e6e040`).

## Purpose / Big Picture

When the agent asks the backend to mint an upload (`POST /uploads`), the response (`UploadWithCredentials`) carries a `metadata` map — provenance such as `device_id`, `release_id`, `release_version`, `deployment_id`, `digest`, `mtime` — that the device is supposed to stamp as cloud object metadata on the uploaded object (S3 `x-amz-meta-*` headers, GCS custom object metadata). Today the agent receives this map and silently drops it: the object lands in the bucket with no provenance.

After this change, every object the agent uploads carries the backend-vended map as user-defined object metadata. An operator inspecting an uploaded object (S3 `HeadObject` / console, GCS object details) sees the provenance keys the backend stamped. The map is passed through generically — the agent never inspects or hardcodes its keys, so the backend can evolve the key set without an agent change.

You can see it work by running the test suite: the S3 tests assert the recorded PUT / CreateMultipartUpload requests carry `x-amz-meta-<key>` headers with the vended values, the GCS tests assert the recorded upload body's object-resource JSON carries the map, and an end-to-end executor test proves the map flows from a mocked create-upload response through the production `SdkTransfer` onto the wire.

## Progress

- [ ] Milestone 1: `s3::Store` and `gcs::Store` put paths accept a metadata map and stamp it; store-level tests.
- [ ] Milestone 2: thread the create-upload response's `metadata` through `ObjectTransfer` and `LiveExecutor`; transfer/executor tests.
- [ ] Milestone 3: preflight to CI-green on the pushed branch head.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

(Add entries as you go. Pre-authoring decisions that shaped this plan are recorded below so a novice understands the "why".)

- Decision: "Store the metadata alongside the upload" means stamping it as cloud object metadata during the transfer — not persisting it in the queued `Job` or the queue snapshot.
  Rationale: The OpenAPI spec (`api/specs/backend/v04.yaml`, `UploadWithCredentials.metadata`) says verbatim: "Provenance the device stamps as cloud object metadata on the uploaded object (S3 `x-amz-meta-*` / GCS `x-goog-meta-*`). … Only the create response carries this map." The map only comes into existence inside `LiveExecutor::upload` (after `create_upload`) and is consumed in the same call by the transfer. A retried job mints a new upload and receives fresh metadata, so nothing needs to survive a restart — `Job`, `Queue`, the scanner, and the scan-upload bridge are untouched. Date/Author: 2026-07-17 / ben@miruml.com.
- Decision: Pass the metadata as an explicit `&HashMap<String, String>` parameter down the put path rather than widening `s3::Object` / `gcs::Object` or introducing a `PutOptions` struct.
  Rationale: `Object` is shared by `get`/`delete`/`exists`, where object metadata is meaningless; a `PutOptions` struct is premature for a single option. An explicit parameter keeps the seam obvious and compiler-checked. Date/Author: 2026-07-17 / ben@miruml.com.
- Decision: For S3 multipart uploads, set the metadata only on `CreateMultipartUpload`; leave `resume_multipart_upload` unchanged.
  Rationale: S3 fixes object metadata at initiation — `UploadPart` and `CompleteMultipartUpload` inherit it, and a resumed upload keeps the metadata from its original initiation. Date/Author: 2026-07-17 / ben@miruml.com.
- Decision: No spec revendor / model regeneration.
  Rationale: `libs/backend-api/src/models/upload_with_credentials.rs` already has `pub metadata: std::collections::HashMap<String, String>` (a required field, synced by the 20260713 spec-sync plan). Date/Author: 2026-07-17 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (a Rust workspace; the binary crate is `miru-agent` under `agent/`). All paths are relative to the repo root; all commands run from the repo root. Branch `feat/store-upload-metadata` is already checked out and clean.

The **upload pipeline**: the scanner emits stable files, `agent/src/workers/scan_upload_bridge.rs` turns them into `Job`s and enqueues them on the `Uploader` actor, and the actor calls `LiveExecutor::upload(&job)` (`agent/src/upload/executor.rs`), which does:

1. `create_upload` — `POST /uploads` via `agent/src/http/uploads.rs::create`, returning `backend_api::models::UploadWithCredentials`:

       pub struct UploadWithCredentials {
           pub upload: Box<models::Upload>,
           pub credentials: Box<models::UploadCredentials>,
           pub metadata: std::collections::HashMap<String, String>,   // <- currently dropped
       }

2. `self.transfer.transfer(&resp.credentials, &resp.upload.destination, &job.file)` — the `ObjectTransfer` trait (`agent/src/upload/transfer.rs`). The production impl `SdkTransfer` dispatches on `credentials.scheme` to `transfer_s3` / `transfer_gcs`, which build an `s3::Store` / `gcs::Store` and call `store.put(file, &object)`.
3. `confirm_upload`, then the delete policy.

The **object stores**:

- `agent/src/s3/mod.rs` — `Store::put(src: File, dst: &Object)` routes by size: ≤ 8 MiB → `put_singlepart` (one `PutObject`), larger → `put_multipart` (`agent/src/s3/multipart.rs`: `CreateMultipartUpload` → `UploadPart`* → `CompleteMultipartUpload`, plus a `resume_multipart_upload` used by tests only today). The aws-sdk-s3 builders (`put_object()`, `create_multipart_upload()`) each have `set_metadata(Option<HashMap<String, String>>)`; the SDK serializes each entry as an `x-amz-meta-<key>` request header (verified in aws-sdk-s3 1.138 `operation/{put_object,create_multipart_upload}/builders.rs`).
- `agent/src/gcs/mod.rs` — `Store::put(src: File, dst: &Object)` calls `self.data.write_object(&resource_name, &dst.key, file).send_unbuffered()`. The google-cloud-storage 1.16 `WriteObject` builder has `set_metadata<I, K, V>(i: I)` taking an iterator of `(K, V)` pairs (`Into<String>` each); the map lands in the object-resource JSON part of the multipart upload body (equivalent to `x-goog-meta-*`).

**Why nothing is persisted**: the metadata exists only between `create_upload` and `transfer`, inside one `upload` call. `Job` (`agent/src/upload/job.rs`), the queue snapshot (`agent/src/upload/queue.rs`), the scanner, and the bridge never see it and are not touched by this plan. There is no serde/back-compat concern.

Test infrastructure (all under `agent/tests/`, mirroring `agent/src/`):

- `agent/tests/s3/mod.rs` + `agent/tests/s3/multipart.rs` — offline tests over `aws_smithy_http_client::test_util::StaticReplayClient`. Replay expectations match method + URI; tests assert on `replay.actual_requests()` (an iterator of `http::Request<SdkBody>` — headers are inspectable via `.headers().get(...)`).
- `agent/tests/gcs/mod.rs` — a local axum mock server (`http_store(rec)` / `HttpRecorder`); the upload handler records `upload_uri`, `upload_body` (raw multipart bytes), and bearer presence.
- `agent/tests/upload/transfer.rs` — drives the production `SdkTransfer` against a replayed S3 exchange and a `GcsRecorder` mock server.
- `agent/tests/upload/executor.rs` — drives `LiveExecutor` with `MockClient` (`set_create_upload` / `set_confirm_upload`) and `MockObjectTransfer`; `response_with_status(...)` builds the `UploadWithCredentials` fixture (today with `metadata: HashMap::new()`); `end_to_end_with_sdk_transfer_over_replayed_s3` runs the production composition offline.
- `agent/tests/mocks/object_transfer.rs` — `MockObjectTransfer` records each call as a `(UploadCredentials, UploadDestination, File)` tuple; this tuple gains the metadata map.

Conventions and gates:

- Tests: `./scripts/test.sh` (runs `RUST_LOG=off cargo test --features test`; the `--features test` flag is mandatory). Lint: `./scripts/lint.sh`. Coverage: `./scripts/covgate.sh` — relevant `.covgate` minimums: `agent/src/upload/` 96.00, `agent/src/s3/` 94.00, `agent/src/gcs/` 88.00.
- Import groups in every source file, blank-line separated: `// standard crates`, `// internal crates`, `// external crates`.
- The lint tool flags 4+ `assert_eq!` on fields of the same variable in one test (`// lint:allow(field-by-field-assert)` to suppress; prefer whole-value asserts).
- CI (`.github/workflows/ci.yml`): jobs `lint`, `test`, `tools`. CI on the pushed head is the authoritative gate. Known pre-existing quirk: `agent/src/workers/.covgate` can fail locally (~83 vs 84.67) even on branches that do not touch workers — do not lower the gate; CI is authoritative.

Out of scope: persisting metadata in `Job`/queue; any `libs/` change; validating or size-limiting the map (the backend owns the keys and the provider limits); GCS `resume`/appendable paths (not used); the confirm/list responses (they carry no metadata by design).

## Plan of Work

### Milestone 1 — the stores accept and stamp a metadata map

1. `agent/src/s3/mod.rs`: add `use std::collections::HashMap;` to the `// standard crates` group. Change signatures to:

       pub async fn put(&self, src: File, dst: &Object, metadata: &HashMap<String, String>) -> Result<(), S3Err>
       pub async fn put_singlepart(&self, src: &File, dst: &Object, metadata: &HashMap<String, String>) -> Result<(), S3Err>

   `put` forwards `metadata` to both branches. In `put_singlepart`, add to the builder chain:

       .set_metadata((!metadata.is_empty()).then(|| metadata.clone()))

   (an empty map passes `None`, keeping today's request byte-identical).

2. `agent/src/s3/multipart.rs`: `put_multipart(&self, src: &Source, dst: &Object, metadata: &HashMap<String, String>)` passes the map to `create_multipart_upload(dst, metadata)`, which gains the same `.set_metadata(...)` line on its builder. `resume_multipart_upload` is unchanged. Import `HashMap` in the `// standard crates` group (the file currently spells `std::collections::HashMap` inline; either style is fine, stay consistent within the file).

3. `agent/src/gcs/mod.rs`: `put(&self, src: File, dst: &Object, metadata: &HashMap<String, String>)` chains `.set_metadata(metadata.clone())` on the `write_object(...)` builder before `.send_unbuffered()`. Import `HashMap` in the `// standard crates` group.

4. Keep the tree compiling with unchanged behavior: update the two call sites in `agent/src/upload/transfer.rs` (`transfer_s3`, `transfer_gcs`) to pass a temporary `&HashMap::new()` (import `HashMap`). This placeholder is replaced in Milestone 2.

5. Fix every test call site the compiler flags (`cargo build -p miru-agent --features test` then `cargo test --no-run --features test`): the `put` / `put_singlepart` / `put_multipart` calls in `agent/tests/s3/mod.rs`, `agent/tests/s3/multipart.rs`, and `agent/tests/gcs/mod.rs` — pass `&HashMap::new()` except where a test exercises metadata.

6. New store tests (lean, one concern each):
   - `agent/tests/s3/mod.rs` (in `put::single`): `put_stamps_metadata_headers` — call `put_singlepart` with `[("device_id", "dvc_1"), ("digest", "sha256:abc")]`; assert the recorded request's headers: `requests[0].headers().get("x-amz-meta-device_id") == Some("dvc_1")` and likewise for `digest` (convert via `.and_then(|v| v.to_str().ok())` if needed by the header API).
   - `agent/tests/s3/multipart.rs`: `create_stamps_metadata_headers` — run `put_multipart` over the existing create/upload-part/complete replay script with a one-entry map; assert the first recorded request (the `POST …?uploads` create) carries `x-amz-meta-device_id`, and the subsequent part/complete requests do not need asserting.
   - `agent/tests/gcs/mod.rs` (in `put`): `upload_includes_metadata_in_object_resource` — call `put` with a one-entry map `[("device_id", "dvc_1")]`; assert `r.upload_body` contains the bytes `"device_id"` and `"dvc_1"` (the object-resource JSON part of the multipart body; substring asserts avoid coupling to serializer key order/spacing).

### Milestone 2 — thread the response metadata through the transfer seam

1. `agent/src/upload/transfer.rs`: extend the trait (metadata appended last):

       pub trait ObjectTransfer: Send + Sync {
           fn transfer(
               &self,
               credentials: &UploadCredentials,
               destination: &UploadDestination,
               file: &File,
               metadata: &HashMap<String, String>,
           ) -> impl Future<Output = Result<(), UploadErr>> + Send;
       }

   `SdkTransfer::transfer` and its `transfer_s3` / `transfer_gcs` helpers take the map and pass it to `store.put(...)`, deleting the Milestone 1 placeholders.

2. `agent/src/upload/executor.rs::LiveExecutor::upload`: pass the response map:

       self.transfer
           .transfer(&resp.credentials, &resp.upload.destination, &job.file, &resp.metadata)
           .await?;

3. `agent/tests/mocks/object_transfer.rs`: `MockObjectTransfer` records and returns 4-tuples `(UploadCredentials, UploadDestination, File, HashMap<String, String>)`.

4. Test updates (compiler-driven plus new assertions):
   - `agent/tests/upload/transfer.rs`: add a `fn metadata() -> HashMap<String, String>` helper (`[("device_id", "dvc_1")]`); pass `&HashMap::new()` at existing call sites and `&metadata()` in the two happy-path tests. In `s3_transfer_puts_object_to_bucket_name_and_key`, additionally assert the recorded PUT carries `x-amz-meta-device_id: dvc_1`; in `gcs_transfer_puts_object_to_bucket_name_and_key`, additionally assert `r.body` contains `"device_id"` and `"dvc_1"`.
   - `agent/tests/upload/executor.rs`: give the create-response fixture a non-empty map — add `fn response_metadata() -> HashMap<String, String>` (`[("device_id", "dvc_1")]`) and use it in `response_with_status`. Update the happy-path whole-tuple assert to `vec![(s3_credentials(), destination(), job.file.clone(), response_metadata())]` — this is the load-bearing proof that the executor hands the response's map to the transfer. In `end_to_end_with_sdk_transfer_over_replayed_s3`, assert the replayed PUT request carries `x-amz-meta-device_id: dvc_1` — proving the map flows from the create response through the production `SdkTransfer` onto the wire.

### Milestone 3 — preflight to CI-green

No new code: local preflight, push, and drive CI to green (see Concrete Steps and Validation).

## Concrete Steps

All commands run from the repo root on branch `feat/store-upload-metadata`.

### Milestone 1

Step 1 — make the edits in Plan of Work → Milestone 1 (items 1-4).

Step 2 — surface every call site needing the new parameter and fix each (item 5):

    cargo build -p miru-agent --features test
    cargo test -p miru-agent --features test --no-run

Expected before fixing: `error[E0061]: this method takes 3 arguments but 2 arguments were supplied` at each `put` / `put_singlepart` / `put_multipart` call in `agent/tests/s3/`, `agent/tests/gcs/`. The compiler is the authority on the full list. Fix, then rebuild clean.

Step 3 — add the three store tests (item 6), then:

    ./scripts/test.sh
    ./scripts/covgate.sh

Expected: all tests pass (the three new tests fail if the `.set_metadata` lines are missing); `s3` ≥ 94.00, `gcs` ≥ 88.00, `upload` ≥ 96.00.

Step 4 — commit (end of Milestone 1):

    git add agent/src/s3 agent/src/gcs agent/src/upload/transfer.rs agent/tests/s3 agent/tests/gcs
    git commit -m "feat(storage): stamp caller-supplied metadata on S3 and GCS puts"

### Milestone 2

Step 1 — make the edits in Plan of Work → Milestone 2 (items 1-3).

Step 2 — compiler-driven test fixes plus the new assertions (item 4):

    cargo build -p miru-agent --features test
    cargo test -p miru-agent --features test --no-run

Expected before fixing: E0061 at every `.transfer(...)` call in `agent/tests/upload/transfer.rs` and mismatched-tuple type errors in `agent/tests/upload/executor.rs` against `MockObjectTransfer::recorded_calls`.

Step 3 — run and gate:

    ./scripts/test.sh
    ./scripts/covgate.sh

Expected: all tests pass; the updated happy-path and end-to-end executor tests fail before the executor edit (empty/missing metadata) and pass after; `upload` ≥ 96.00.

Step 4 — commit (end of Milestone 2):

    git add agent/src/upload agent/tests/upload agent/tests/mocks/object_transfer.rs
    git commit -m "feat(upload): stamp backend-vended metadata onto uploaded objects"

### Milestone 3

Step 1 — local preflight (fast feedback, not authoritative):

    ./scripts/preflight.sh

Expected: clean, modulo the known local-only `workers` covgate gap (this plan does not touch `agent/src/workers/`; CI computes the authoritative number).

Step 2 — push and drive CI to green:

    git push -u origin feat/store-upload-metadata
    gh pr checks --watch    # or: gh run watch

If any job fails: `gh run view --log-failed`, fix, commit, push, re-watch. Do not take the PR out of draft or report the task complete until CI is green on the pushed branch head.

## Validation and Acceptance

Behavioral acceptance (run `./scripts/test.sh`; all tests pass, and specifically):

1. S3 single-part: `put_stamps_metadata_headers` observes `x-amz-meta-device_id: dvc_1` (and the second key) on the recorded PUT. Fails before Milestone 1, passes after.
2. S3 multipart: `create_stamps_metadata_headers` observes `x-amz-meta-device_id` on the recorded CreateMultipartUpload request. Fails before Milestone 1, passes after.
3. GCS: `upload_includes_metadata_in_object_resource` observes the key and value bytes in the recorded upload body. Fails before Milestone 1, passes after.
4. Executor seam: the happy-path test observes the transfer call tuple carrying exactly the create response's map, and `end_to_end_with_sdk_transfer_over_replayed_s3` observes `x-amz-meta-device_id: dvc_1` on the wire through the production composition. Both fail before Milestone 2, pass after.
5. Regression: all pre-existing tests still pass with empty maps — an empty map must add no headers/JSON (the S3 `.then()` guard keeps requests byte-identical, which the untouched replay expectations prove).

Coverage: `./scripts/covgate.sh` reports `upload` ≥ 96.00, `s3` ≥ 94.00, `gcs` ≥ 88.00.

CI acceptance (authoritative): **preflight must report CLEAN — CI green on the pushed branch head (`lint`, `test`, `tools` all passing on the exact head commit) — before the PR leaves draft or the task is reported complete.** Local scripts are fast feedback only; GitHub Actions on the pushed head is the source of truth. The known local-only `workers` covgate shortfall does not count as a failure if CI passes.

## Idempotence and Recovery

Every edit is additive (a parameter, a builder line, a tuple element) and safe to re-apply; re-running any build/test/lint/covgate/preflight step is read-only and repeatable. Milestone 1 leaves production behavior unchanged (empty-map placeholders), so the tree is releasable between milestones.

Runtime idempotence: stamping metadata is part of the same put the store already performs — a retried or re-driven upload re-runs `create_upload`, receives a fresh (equivalent) map, and stamps it again on the overwritten object. A resumed S3 multipart upload keeps the metadata from its original initiation, per S3 semantics.

Recovery: before a commit, `git checkout -- <files>` restores a clean tree; before push, `git reset --hard main` is a safe full rollback (the branch exists only for this work). After a commit, revert the milestone's single commit (`git revert <sha>`); each milestone is one commit precisely so it can be reverted or bisected independently.
