# Implement the GCS arm of the uploader's transfer seam

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Replace the GCS stub in `agent/src/upload/transfer.rs` with a real `transfer_gcs`; add tests to `agent/tests/upload/transfer.rs`. No other files change. |
| `agent/src/gcs/` | read-only | The GCS store module consumed by the new arm. Do not modify it. |
| `libs/backend-api/` | read-only | Generated models (`UploadCredentials`, `GcsUploadCredentials`). Never edit by hand. |

This plan lives in `agent/plans/` because all changes are made in the agent repo.

Git context: the working branch is `feat/uploader-gcs-transfer`, **stacked on `feat/upload-executor`** (open PR #143 against `main`). The PR base for this work is `feat/upload-executor`, NOT `main`. Do not rebase onto `main`.

## Purpose / Big Picture

The upload executor (`agent/src/upload/executor.rs`) asks the backend to vend short-lived, downscoped cloud credentials and hands them to the `ObjectTransfer` seam (`agent/src/upload/transfer.rs`), which drives the native cloud SDK to push the file's bytes to the object store. Today only the `s3` scheme works; the `gcs` scheme returns "GCS uploads are not yet implemented". After this change, a device whose bucket lives in Google Cloud Storage can complete broker uploads: `SdkTransfer::transfer` with a `gcs`-scheme credential streams the file into GCS via `gcs::Store::put` using the vended OAuth2 bearer token. You can see it working by running `./scripts/test.sh` — the new offline tests drive a real `gcs::Store` upload against a local axum mock server through the full `SdkTransfer` dispatch path.

## Progress

- [x] Milestone 1 — implement `transfer_gcs`, `gcs_credentials`, and the test-only endpoint seam in `agent/src/upload/transfer.rs`; `cargo build --features test` succeeds. (2026-07-13: both `--features test` and no-feature builds pass; `executor_err` helper also reused by `transfer_s3` per the plan's cost-nothing clause.)
- [x] Milestone 2 — tests in `agent/tests/upload/transfer.rs`; `./scripts/test.sh` and `./scripts/covgate.sh` pass. (2026-07-13: 1483 tests, 0 failures; upload coverage 92.33% ≥ 91.00, all modules pass.)
- [x] Milestone 3 — `./scripts/lint.sh` clean; `./scripts/preflight.sh` prints `Preflight clean`. (2026-07-13: lint clean — only the 3 pre-existing allowed audit warnings; test.sh and covgate.sh re-verified after the mock-body fix below. Preflight is delegated to a dedicated preflight agent per the session's orchestration and was not run here.)

## Surprises & Discoveries

- `./scripts/update-deps.sh` (Milestone 3, step 8) bumped the aws-sdk crates to versions requiring rustc 1.94.1, but the local toolchain is 1.94.0, so the refreshed `Cargo.lock` failed to build. Since this change adds no dependencies, `Cargo.lock` was restored to HEAD and `./scripts/lint.sh` run against the committed lockfile (clean). The lockfile refresh needs a toolchain bump first and is out of scope here.
- The first cut of the happy-path mock handler omitted axum's `Bytes` body extractor, so the mock replied before the SDK client finished writing the multipart body — a race that surfaced as a flaky client-side `ConnectionErr` ("error sending request") in roughly half the runs of the small `upload::transfer` subset, while full-suite runs happened to pass. The gcs suite's handler (`agent/tests/gcs/mod.rs`) already drains the body via its `body: Bytes` extractor, which is why the identical pattern there is stable. Fixed by adding a `_body: Bytes` parameter; 10/10 subset runs pass after the fix.

## Decision Log

- Decision: Inject the mock endpoint via a `#[cfg(feature = "test")] pub gcs_endpoint: Option<String>` field on `SdkTransfer`, rather than via the credentials.
  Rationale: `S3UploadCredentials` carries an `endpoint` field so the s3 arm is mock-targetable through production data; `GcsUploadCredentials` has no endpoint (only `access_token` + `expires_at` — the backend downscopes a token, it never redirects the host), so the override must be a test seam. A field on `SdkTransfer` keeps the happy-path test exercising the real trait dispatch (`SdkTransfer::transfer` → scheme match → `transfer_gcs`). It mirrors how `gcs::Store` itself exposes `from_endpoint` under the same feature flag.
  Date/Author: 2026-07-13 / plan author.
- Decision: Build the test happy-path mock inline in `agent/tests/upload/transfer.rs` (a ~25-line axum router) instead of extracting the richer mock in `agent/tests/gcs/mod.rs` into `agent/tests/mocks/`.
  Rationale: Scope stays tight — the transfer test needs only "one POST upload succeeded, with this bearer token", not the gcs suite's recorder of URIs, bodies, truncation, and status overrides. Extracting shared plumbing would touch the gcs test module, which this plan deliberately leaves alone.
  Date/Author: 2026-07-13 / plan author.

## Outcomes & Retrospective

Implemented as planned, in three commits. `transfer_gcs` mirrors `transfer_s3` (credential-arm pluck → store build → `put`), with the extra fallible-async `Store::new` error site isolated in `gcs_store` alongside the test-only endpoint override; the shared `UploadErr::ExecutorErr` wrap was extracted into `executor_err` and reused by `transfer_s3` (the plan's cost-nothing clause). Tests land the four planned cases plus the retained s3/unknown-scheme ones; upload module coverage rose to 92.33% against the 91.00 gate. Two deviations from the script, both recorded in Surprises & Discoveries: `update-deps.sh` was rolled back (toolchain 1.94.0 vs aws crates requiring 1.94.1), and the mock upload handler needed a `Bytes` body extractor to avoid a response-before-body-drained race. `preflight.sh` was intentionally left to the downstream preflight agent.

## Context and Orientation

Terms: the **uploader** is the agent subsystem (`agent/src/upload/`) that pushes device files to cloud object storage; the **transfer seam** is the `ObjectTransfer` trait in `agent/src/upload/transfer.rs` separating the executor's orchestration from the concrete cloud SDKs; **vended credentials** are the short-lived, single-object-scoped credentials the backend returns (`backend_api::models::UploadCredentials`).

### The file being changed: `agent/src/upload/transfer.rs`

It defines:

- `trait ObjectTransfer` — `transfer(&self, credentials: &UploadCredentials, destination: &UploadDestination, file: &File) -> impl Future<Output = Result<(), UploadErr>> + Send`. Implementations must tolerate cancellation at any await point (the upload actor's future may be dropped on shutdown).
- `pub struct SdkTransfer;` — the production impl. Its `transfer` matches on `credentials.scheme` (`backend_api::models::upload_credentials::Scheme`): `UPLOAD_SCHEME_S3` → `transfer_s3`, `UPLOAD_SCHEME_GCS` → `Err(unsupported("GCS uploads are not yet implemented"))` (the stub this plan replaces), `SchemeUnknown` → `Err(unsupported(...))`.
- `async fn transfer_s3(credentials, destination, file)` — the shape to mirror: pluck the credential arm (`credentials.s3_credentials.as_deref().ok_or_else(|| unsupported("s3 scheme is missing s3_credentials"))?`), build the store (`s3::Store::new(s3_config(creds))`), build the object from `destination.bucket_name` / `destination.object_key`, then `store.put(file.clone(), &object).await.map_err(|e| UploadErr::ExecutorErr(ExecutorErr { source: Box::new(e), trace: trace!() }))`.
- `pub fn s3_config(creds: &S3UploadCredentials) -> s3::Config` — the pure credential→SDK mapping, kept `pub` so it is unit-testable without a live transfer. The GCS analogue is `gcs_credentials`.
- `fn unsupported(msg: &str) -> UploadErr` — wraps a static reason in `UploadErr::ExecutorErr`.

`ExecutorErr` (`agent/src/upload/errors.rs`) is `{ source: Box<dyn std::error::Error + Send + Sync>, trace: Box<Trace> }`; `UploadErr::ExecutorErr` wraps it. `trace!()` is `crate::trace!()`.

### The models

`backend_api::models::UploadCredentials` (`libs/backend-api/src/models/upload_credentials.rs`): `scheme: Scheme`, `s3_credentials: Option<Box<S3UploadCredentials>>`, `gcs_credentials: Option<Box<GcsUploadCredentials>>`, `expires_at: String`. Exactly one arm is populated per the scheme.

`backend_api::models::GcsUploadCredentials` (`libs/backend-api/src/models/gcs_upload_credentials.rs`) — the token-vending contract: `scheme` (always `gcs`), `access_token: String` (a downscoped OAuth2 bearer token, sent as `Authorization: Bearer <token>`; the backend restricts it with a Credential Access Boundary to `storage.objects.create` on the exact object), `expires_at: String`. There is no endpoint, region, or bucket in the credentials — the bucket comes from `destination.bucket_name`, and the transfer does not consume `expires_at` (mid-upload expiry handling belongs to the executor, out of scope here).

### The store being wired in: `agent/src/gcs/` (merged to main in PR #103)

`gcs::Store` deviates from `s3::Store` in ways that matter here (documented inline in `agent/src/gcs/mod.rs`):

- `Store::new(creds: gcs::Credentials) -> Result<Self, GcsErr>` is **async and fallible** (client builders are async; a token with a byte invalid in an HTTP header value yields `GcsErr::BuildErr`). s3's `Store::new(cfg)` is sync and infallible. So `transfer_gcs` has an extra error site to map into `UploadErr::ExecutorErr`.
- There is **no `gcs::Config`**: `gcs::Credentials` is just `{ access_token: String }`, and there is no endpoint/region. The bucket lives on `gcs::Object { bucket, key }` (same shape as `s3::Object`); the store derives the `projects/_/buckets/<bucket>` resource name internally.
- `#[cfg(feature = "test")] Store::from_endpoint(creds, endpoint: String) -> Result<Self, GcsErr>` points the HTTP data client at a local mock server — the seam the happy-path test uses.
- `Store::put(&self, src: File, dst: &Object) -> Result<(), GcsErr>` streams the file off disk (simple or resumable upload chosen by the SDK); a missing local source surfaces as `GcsErr::FileSysErr` before any request is dispatched.

### Existing tests and the coverage gate

`agent/tests/upload/transfer.rs` currently has four tests: `gcs_scheme_is_unsupported` (asserts the stub's error — **this test is replaced** by this plan), `unknown_scheme_is_unsupported`, `s3_config_maps_credentials_and_endpoint`, and `s3_scheme_without_credentials_errs`. They build `UploadCredentials` by deserializing `serde_json::json!` values (helpers `credentials_json`/`s3_credentials_json` in `agent/tests/mocks/upload_client.rs`; `credentials_json("gcs")` populates the *s3* arm and nulls the gcs arm, so it is exactly the missing-gcs_credentials shape).

The established offline GCS test pattern is in `agent/tests/gcs/mod.rs`: start a local axum server with `crate::mocks::http_client::run_server(router) -> Server { base_url }`; route `POST /upload/storage/v1/b/{*rest}` to a handler that records the request (hit count, `Authorization` header) and returns `200` with content-type `application/json` and a minimal decodable GCS `Object` JSON body such as `{"name": "logs/a.log", "bucket": "my-bucket"}` — that is all the SDK needs to finalize a small (multipart single-POST) upload. The gRPC `StorageControl` stub (`Store::from_stub` + `mockall`) exists for delete/exists; **`put` never touches the control client**, so the transfer tests need only the HTTP mock.

`agent/src/upload/.covgate` is `91.00` — the upload module's minimum coverage, enforced by `./scripts/covgate.sh`. New `transfer_gcs` lines count against it; the tests below cover every line except the two-line production `Store::new` branch of the endpoint helper.

Repo conventions (see `agent/AGENTS.md`): three-group import ordering with `// standard crates` / `// internal crates` / `// external crates` comments (enforced by the lint); run tests only via `./scripts/test.sh` (needs `--features test`); no `#[serial]` needed here (tests bind only ephemeral `127.0.0.1:0` ports and temp files).

## Plan of Work

All edits are in `agent/src/upload/transfer.rs` and `agent/tests/upload/transfer.rs`. Do not modify the executor, queue, or `agent/src/gcs/`; if a genuine blocker forces a gcs-module change, stop and record it in the Decision Log first.

Milestone 1 — `agent/src/upload/transfer.rs`:

1. Imports: add `use crate::gcs;` to the internal-crates group and `GcsUploadCredentials` to the existing `backend_api::models` import.
2. Turn `SdkTransfer` into a braced struct with a test-only endpoint seam, and update its doc comment (drop "GCS is not implemented yet"; describe the gcs arm as a native GCS upload driven by the vended downscoped bearer token):

       #[derive(Default)]
       pub struct SdkTransfer {
           /// Test-only override pointing the GCS data client at a local mock
           /// server. Always `None` in production builds, where the field does
           /// not exist and the real GCS endpoint is used.
           #[cfg(feature = "test")]
           pub gcs_endpoint: Option<String>,
       }

   Note: `SdkTransfer` is constructed nowhere in `agent/src/` (only re-exported from `agent/src/upload/mod.rs`), so this is not a production API break; the three bare `SdkTransfer` expressions in `agent/tests/upload/transfer.rs` become `SdkTransfer::default()` in Milestone 2.
3. Replace the `UPLOAD_SCHEME_GCS` match arm with a call to `transfer_gcs(credentials, destination, file, endpoint).await`, where `endpoint` is `self.gcs_endpoint.clone()` under `#[cfg(feature = "test")]` and `None` under `#[cfg(not(feature = "test"))]`.
4. Add `pub fn gcs_credentials(creds: &GcsUploadCredentials) -> gcs::Credentials` mirroring `s3_config` — `gcs::Credentials { access_token: creds.access_token.clone() }` — with a doc comment noting it is kept separate so the credential→SDK mapping is unit-testable without a live transfer, and that `expires_at` is deliberately not consumed here.
5. Add `transfer_gcs` mirroring `transfer_s3`'s shape and doc style (physical bucket from `destination.bucket_name`, not the Miru `bucket_id`):

       async fn transfer_gcs(
           credentials: &UploadCredentials,
           destination: &UploadDestination,
           file: &File,
           endpoint: Option<String>,
       ) -> Result<(), UploadErr> {
           let creds = credentials
               .gcs_credentials
               .as_deref()
               .ok_or_else(|| unsupported("gcs scheme is missing gcs_credentials"))?;
           let store = gcs_store(gcs_credentials(creds), endpoint)
               .await
               .map_err(executor_err)?;
           let object = gcs::Object {
               bucket: destination.bucket_name.clone(),
               key: destination.object_key.clone(),
           };
           store.put(file.clone(), &object).await.map_err(executor_err)
       }

   where `executor_err` is a small private helper `fn executor_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> UploadErr` wrapping into `UploadErr::ExecutorErr` (extracting the closure `transfer_s3` writes inline; reuse it from `transfer_s3` too only if it costs nothing — otherwise leave `transfer_s3` untouched), and `gcs_store` isolates the fallible-async construction plus the seam:

       /// Builds the GCS store; the `endpoint` override (test builds only)
       /// points the data client at a local mock server.
       async fn gcs_store(
           creds: gcs::Credentials,
           endpoint: Option<String>,
       ) -> Result<gcs::Store, gcs::GcsErr> {
           let _ = &endpoint; // consumed only in test builds
           #[cfg(feature = "test")]
           if let Some(ep) = endpoint {
               return gcs::Store::from_endpoint(creds, ep).await;
           }
           gcs::Store::new(creds).await
       }

   (The `let _ = &endpoint;` silences the unused-parameter warning in non-test builds; place it before the cfg block since the `if let` moves `endpoint`. If clippy objects, `#[cfg_attr(not(feature = "test"), allow(unused_variables))]` on the parameter is the fallback.)

Milestone 2 — `agent/tests/upload/transfer.rs`:

1. Update the three existing bare `SdkTransfer` constructions to `SdkTransfer::default()`.
2. Add a local helper building the gcs credential arm (inline `json!`, matching the file's existing `creds_from` pattern):

       fn gcs_creds_json(token: &str) -> serde_json::Value {
           json!({
               "scheme": "gcs",
               "s3_credentials": null,
               "gcs_credentials": {
                   "scheme": "gcs",
                   "access_token": token,
                   "expires_at": "2021-01-01T01:00:00Z"
               },
               "expires_at": "2021-01-01T01:00:00Z"
           })
       }

3. Replace `gcs_scheme_is_unsupported` with `gcs_scheme_without_credentials_errs`: `creds_from(credentials_json("gcs"))` (gcs arm null), assert `UploadErr::ExecutorErr` and that the message mentions the missing `gcs_credentials`.
4. Add `gcs_credentials_maps_access_token` (mirrors `s3_config_maps_credentials_and_endpoint`): deserialize a `GcsUploadCredentials` from json, call `miru_agent::upload::transfer::gcs_credentials`, assert `access_token` is carried through.
5. Add the happy path `gcs_scheme_uploads_file_to_store`: write a temp file (reuse the gcs suite's approach: `miru_agent::filesys::files::temp` + `write_bytes`), start `run_server` with a minimal router —

       async fn upload_handler(State(rec): ..., headers: HeaderMap) -> (StatusCode, [(HeaderName, &'static str); 1], String)
       // records hit count + the Authorization header value into an Arc<Mutex<...>>,
       // returns 200, content-type application/json,
       // body {"name": "logs/a.log", "bucket": "my-bucket"}
       Router::new().route("/upload/storage/v1/b/{*rest}", post(upload_handler)).with_state(rec)

   — then run `SdkTransfer { gcs_endpoint: Some(server.base_url) }.transfer(&creds_from(gcs_creds_json("gcs-token-test")), &destination(), &src.to_file()).await`. Assert `Ok(())`, exactly 1 upload hit, and that the recorded auth header equals `"Bearer gcs-token-test"` — proving the vended token flowed through `gcs_credentials` into the request. (Needed imports — `axum`, `run_server` — follow `agent/tests/gcs/mod.rs`; all crates are already dev-dependencies.)
6. Add the store-error path `gcs_transfer_error_maps_to_executor_err`: same setup but the handler returns `403` with a GCS error JSON body (`{"error":{"code":403,"message":"denied"}}`); assert `UploadErr::ExecutorErr`. This pins the `GcsErr`→`ExecutorErr` wrap on the `put` failure site.

Milestone 3 — gates: lint, covgate, preflight (commands below); fix findings until clean.

## Concrete Steps

All commands run from the agent repo root `/home/ben/miru/workbench4/repos/agent`, on branch `feat/uploader-gcs-transfer`.

Milestone 1:

1. Edit `agent/src/upload/transfer.rs` per Plan of Work.
2. Run `cargo build -p miru-agent --features test` — expect success. Also `cargo build -p miru-agent` (no features) to prove the non-test cfg branches compile.
3. Commit: `git add agent/src/upload/transfer.rs && git commit -m "feat(upload): implement GCS arm of the SDK transfer seam"`.

Milestone 2:

4. Edit `agent/tests/upload/transfer.rs` per Plan of Work.
5. Run `./scripts/test.sh` — expect `test result: ok.` with zero failures; the suite includes the four new/updated transfer tests.
6. Run `./scripts/covgate.sh` — expect `✅ upload: <NN.NN>% (requires 91.00%)` and the final all-modules-pass line. If upload dips below 91.00, the uncovered lines are almost certainly in `transfer_gcs`/`gcs_store`; add a targeted test (e.g. assert the `BuildErr` path by vending a token containing `\n`, which makes `gcs_store` fail before any request) rather than lowering the gate.
7. Commit: `git add agent/tests/upload/transfer.rs && git commit -m "test(upload): cover GCS transfer arm via endpoint-override mock"`.

Milestone 3:

8. Run `./scripts/update-deps.sh` then `./scripts/lint.sh` — expect no findings (import order, fmt, clippy, machete/diet, audit). No dependencies were added, so machete/audit results are unchanged.
9. Run `./scripts/preflight.sh` — expect the final line to be exactly `Preflight clean`.
10. Commit any fixes: `git add -A && git commit -m "chore(upload): lint/coverage fixes for gcs transfer arm"` (skip if the tree is clean).

## Validation and Acceptance

Acceptance is offline behavior, verifiable with no network or GCP project:

- `SdkTransfer::transfer` with a `gcs`-scheme `UploadCredentials` whose `gcs_credentials.access_token` is `gcs-token-test`, a destination of bucket `my-bucket` / key `logs/a.log`, and a real temp file, pointed at a local mock via `gcs_endpoint`, returns `Ok(())` and the mock records exactly one upload request carrying `Authorization: Bearer gcs-token-test` (test `gcs_scheme_uploads_file_to_store`).
- The same call with `gcs_credentials: null` returns `UploadErr::ExecutorErr` mentioning the missing credentials without dispatching any request (test `gcs_scheme_without_credentials_errs`; this test fails against the old stub because the message changes from "not yet implemented").
- A `403` from the store surfaces as `UploadErr::ExecutorErr` (test `gcs_transfer_error_maps_to_executor_err`).
- `gcs_credentials` maps `access_token` verbatim (test `gcs_credentials_maps_access_token`).
- Pre-existing behavior is unchanged: `unknown_scheme_is_unsupported`, `s3_config_maps_credentials_and_endpoint`, and `s3_scheme_without_credentials_errs` still pass.

Commands and expected results: `./scripts/test.sh` → zero failures; `./scripts/covgate.sh` → upload ≥ 91.00 and all modules pass; `./scripts/lint.sh` → clean. **Hard gate: `./scripts/preflight.sh` must print `Preflight clean` before the changes are published (pushed / PR opened). Do not publish until it does.** The eventual PR base is `feat/upload-executor` (or `main` only after PR #143 merges).

## Idempotence and Recovery

All steps are edits to two files plus read-only script runs; re-running any step is safe. If a step goes wrong, `git restore agent/src/upload/transfer.rs agent/tests/upload/transfer.rs` (or `git reset --hard` to the last milestone commit) recovers. Tests bind only ephemeral `127.0.0.1:0` ports and temp files — no `#[serial]`, no global state, re-runnable at will. No migrations, no destructive operations, no dependency changes. If the SDK's upload finalization rejects the minimal mock `Object` JSON (it does not — `agent/tests/gcs/mod.rs::put::upload_streams_file_body` passes with the same body today), copy the exact handler from that file and note it in Surprises & Discoveries.

## Interfaces and Dependencies

Everything needed already exists on this branch; no `Cargo.toml` changes.

- `crate::gcs::{Store, Credentials, Object, GcsErr}` (`agent/src/gcs/mod.rs`): `Store::new(Credentials) -> Result<Store, GcsErr>` (async), `#[cfg(feature = "test")] Store::from_endpoint(Credentials, String) -> Result<Store, GcsErr>` (async), `Store::put(&self, src: File, dst: &Object) -> Result<(), GcsErr>`, `Credentials { access_token: String }`, `Object { bucket: String, key: String }`.
- `backend_api::models::{UploadCredentials, GcsUploadCredentials, UploadDestination}` and `backend_api::models::upload_credentials::Scheme::UPLOAD_SCHEME_GCS`.
- `crate::upload::errors::{ExecutorErr, UploadErr}`, `crate::trace!`, `crate::filesys::File`.
- Tests: `crate::mocks::http_client::run_server`, `crate::mocks::upload_client::credentials_json`, `miru_agent::filesys::files` temp helpers, `axum` (dev-dependency, used by `agent/tests/gcs/mod.rs` today), `serde_json::json`.
