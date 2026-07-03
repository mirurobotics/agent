# Add an S3 object-store CRUD module to the agent

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | New module `agent/src/object_store/`, matching tests under `agent/tests/object_store/`, and dependency additions to `Cargo.toml` + `agent/Cargo.toml`. |

This plan lives in `agent/plans/backlog/` because every file touched is under the `agent` submodule (`agent/agent/` for source, `agent/Cargo.toml` for deps).

Working directory for every command below is the `agent` submodule root — i.e. the directory that contains the workspace `Cargo.toml`, `agent/`, `libs/`, and `scripts/`. On this machine that is `/home/ben/miru/workbench5/repos/agent`. The branch `feat/s3-object-storage-crud` (base `main`) is already checked out. Do not switch branches. The eventual PR targets `main`.

## Purpose / Big Picture

The Miru backend will mint short-lived AWS STS credentials and hand them to the agent so the agent can read and write objects in an S3 bucket directly (for example, uploading device artifacts and fetching remote blobs). Today the agent has no S3 capability at all.

After this change the agent gains a new module `object_store` exposing an async `S3Store` client with basic CRUD:

- `put_object(key, bytes)` — create or overwrite an object from an in-memory `Vec<u8>`.
- `get_object(key) -> Vec<u8>` — read an object's whole body into memory.
- `delete_object(key)` — delete an object (idempotent per S3 semantics).
- `object_exists(key) -> bool` — HEAD the object; `true` on 200, `false` on 404.
- `list_objects(prefix) -> Vec<String>` — list object keys under a prefix.

The client is constructed **only** from caller-supplied temporary credentials (access key id, secret access key, session token) plus a region and bucket name. It never reads ambient AWS configuration (environment variables, `~/.aws`, or EC2/ECS instance metadata). Errors are surfaced through the agent's existing `crate::errors::Error` trait so callers handle S3 failures the same way they handle every other agent error.

You can see it working by running the module's offline unit tests (they inject canned HTTP responses — no live AWS): `./scripts/test.sh` runs them green, and each CRUD path (`put` success, `get` byte round-trip, `get` 404 → mapped `NotFound` error, `delete`, `exists` true/false, `list` returns keys) is asserted.

Streaming (multipart / `ByteStream`-based partial reads and writes) is explicitly **out of scope** for this PR and is noted as a follow-up in the Decision Log.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) M1 — Add dependencies and confirm the workspace builds.
- [ ] M2 — Implement the `object_store` module (errors + `S3Store` client + CRUD methods).
- [ ] M3 — Write offline unit tests using the smithy `StaticReplayClient` mock transport.
- [ ] M4 — Add the `.covgate` gate, run preflight, resolve lint/coverage findings until clean.

Add timestamps and split entries as work proceeds.

## Surprises & Discoveries

(Add entries as work proceeds.)

- Observation: …
  Evidence: …

## Decision Log

All entries below are dated 2026-07-03, authored by the plan author. Add new entries (with date) as decisions are made during implementation.

- **Module name and location: `agent/src/object_store/`, in the agent crate (not a new lib crate).** The existing `agent/src/storage/` module is *local on-disk state* (device.json, settings.json, `storage::Layout`, `storage::Storage`) and must not be touched or conflated with remote object storage. The name `object_store` is unambiguous against `storage`. The workspace `members` are only `agent`, `libs/device-api`, `libs/backend-api`; the two `libs/` crates are OpenAPI-generated clients, not hand-written domain libraries. There is no established pattern of hand-written lib crates, and every hand-written module lives under `agent/src/`. So the module belongs in `agent/src/`, consistent with `http`, `network`, `storage`, etc.

- **Credentials are always caller-supplied; no ambient AWS config.** Construction uses `aws_sdk_s3::config::Credentials::new(access_key_id, secret_access_key, Some(session_token), None, "miru-agent")` fed to `aws_sdk_s3::config::Builder` together with an explicit `Region`. We do **not** call `aws_config::load_from_env()` or any default-provider chain, so IMDS/env/`~/.aws` are never consulted. This matches the backend-mints-STS design. Region and bucket are constructor arguments.

- **HTTP client / TLS: use the SDK's default rustls HTTPS client for production; inject a mock transport for tests.** The workspace deliberately routes MQTT/reqwest TLS through native-tls/OpenSSL to avoid transitively pinning the vulnerable `rustls-webpki 0.102.8` that `rumqttc`/`rumqttd` drag in (see the long comment in the workspace `Cargo.toml`). `aws-sdk-s3`'s default `rustls` feature pulls a *separate, current* rustls stack (`rustls`, `hyper-rustls`, `rustls-pki-types`, `rustls-native-certs`) that does **not** reintroduce `rustls-webpki 0.102.x`, so it does not conflict with that advisory. We keep the SDK default HTTPS client for real use. For the offline tests we override the connector with `aws_smithy_http_client::test_util::StaticReplayClient`, so no live TLS or network is exercised in CI. If security review later prefers a single TLS stack, a follow-up can switch the SDK to a native-tls hyper connector; that is out of scope here.

- **Test transport: `StaticReplayClient` (record-replay), primary.** It lets each test assert the exact HTTP request the SDK emitted (method, URI/key, body bytes) and return a canned HTTP response, which is exactly the coverage the task asks for. Import it from `aws_smithy_http_client::test_util::{StaticReplayClient, ReplayEvent}` (a dev-dependency, `test-util` feature). Note: `aws-smithy-runtime` also re-exports these types at `aws_smithy_runtime::client::http::test_util`, but that module is `#[deprecated]` ("Please use the `test-util` feature from `aws-smithy-http-client` instead"); using it would trip `clippy -D warnings`, so we depend on `aws-smithy-http-client` directly. The alternative operation-level `aws-smithy-mocks` crate is not used, to keep one mocking approach.

- **Behavior version pinned via the `behavior-version-latest` feature** on `aws-sdk-s3` so `Config::builder()` does not panic for a missing behavior version and no per-call `.behavior_version(...)` boilerplate is needed.

- **Streaming is out of scope (follow-up).** `put_object`/`get_object` move whole objects as `Vec<u8>`. A later PR can add multipart upload and streaming `get` via `ByteStream` once a concrete large-object use case exists.

- **No shared cross-provider trait yet.** This PR ships only S3. A tiny private helper trait is acceptable if it makes the module cleaner, but no public cross-provider abstraction is introduced (GCS and a shared trait are separate later work).

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent is a Rust binary workspace. The workspace `Cargo.toml` (at the submodule root) declares `[workspace.dependencies]` (a shared version table) and `[workspace.package]` with `rust-version = "1.93.0"`. Member crates reference shared deps as `foo = { workspace = true }` in their own `Cargo.toml`. The agent binary crate is `agent/Cargo.toml` (package name `miru-agent`); it has a `[features] test = []` flag and a `[dev-dependencies]` block. The installed toolchain is Rust 1.94, and `aws-sdk-s3` 1.137 and its companions require `rust-version 1.91.1`, so they are compatible with the workspace floor.

`Cargo.lock` is git-ignored (see `.gitignore`); `./scripts/update-deps.sh` (`cargo update`) regenerates it. Regenerate the lock before linting so the unused-dependency linters see a consistent graph.

Module layout convention (from `AGENTS.md` → "Adding a new module"): a module is a directory `agent/src/<module>/` containing `mod.rs` (and `errors.rs` when it has its own error type), registered with `pub mod <module>;` in `agent/src/lib.rs`. A matching test directory `agent/tests/<module>/` mirrors it, registered in `agent/tests/mod.rs` and via a per-module `agent/tests/<module>/mod.rs`. Each source module directory contains a `.covgate` file with a minimum region-coverage percentage; `./scripts/covgate.sh` discovers every `.covgate` under `agent/src` and fails if any module is below its threshold.

Import ordering is enforced by a custom linter: three groups separated by a blank line and a `// standard crates` / `// internal crates` / `// external crates` comment, in that order.

Error handling convention (from `agent/src/errors/mod.rs`, `agent/src/http/errors.rs`, `agent/src/storage/errors.rs`):

- Every leaf error is a `struct` deriving `thiserror::Error`, carrying a `pub trace: Box<Trace>` field, and manually implementing `crate::errors::Error` (usually taking the trait defaults). `Trace` is captured with the `crate::trace!()` macro.
- Each module's errors are aggregated into one enum whose variants are `#[error(transparent)]` wrappers, and the enum implements the `Error` trait via the `crate::impl_error!(EnumName { Variant, ... });` macro.
- The `Error` trait (in `agent/src/errors/mod.rs`) provides defaults for `code()`, `http_status()`, `params()`, `is_network_conn_err()`. Leaf errors override only what they need. `Code` is a small enum with `ResourceNotFound`, `InternalServerError`, `BackendError(String)`, etc.
- Example to mirror precisely: `agent/src/http/errors.rs` (per-kind structs, one aggregating `HTTPErr` enum, `impl_error!`, plus a `reqwest_err_to_http_client_err` conversion helper). `object_store` will have an analogous `s3_sdk_err_to_object_store_err`-style mapper.

Async test convention (from `agent/tests/http/client.rs`): tests are `#[tokio::test] async fn`, grouped in nested `pub mod` blocks, importing the crate under test as `use miru_agent::...;`. Tests must be run with `--features test`; `AGENTS.md` and `ARCHITECTURE.md` state tests require `--features test` and run single-threaded relative to shared resources (use `#[serial]` only when a test binds a fixed global resource — the S3 tests do not).

Relevant AWS SDK facts (verified with `cargo info` against crates.io on 2026-07-03):

- `aws-sdk-s3` 1.137.0 — the S3 client. Default features include `rustls` + `default-https-client`; we add it with a curated feature set (see M1). Feature `behavior-version-latest` avoids behavior-version panics. Feature `test-util` is enabled under the agent's `test` feature and dev-deps for mocking.
- `aws-config` 1.8.x — provides `aws_config::Region` re-export and credential types; here we only need `Region` and `Credentials`, both of which are also reachable via `aws_sdk_s3::config::{Region, Credentials}`, so `aws-config` is **not** required. Do not add it unless a concrete need appears.
- `aws-smithy-http-client` 1.1.x — hosts `test_util::{StaticReplayClient, ReplayEvent}` behind the `test-util` feature; a dev-dependency. (`StaticReplayClient::new`, `.actual_requests()`, and `.assert_requests_match(ignore_headers: &[&str])` all live here; `impl HttpClient for StaticReplayClient` lets it plug into the S3 config's `.http_client(...)`.)
- `aws-smithy-types` 1.5.x — provides `body::SdkBody` used to build the canned request/response bodies in tests; a dev-dependency.

The SDK error shape to map: fallible S3 calls return `Result<Output, aws_sdk_s3::error::SdkError<E, R>>` where `E` is the operation error enum (e.g. `GetObjectError`, `HeadObjectError`). `SdkError` has variants `ConstructionFailure`, `TimeoutError`, `DispatchFailure` (network/connection), `ResponseError`, and `ServiceError(_)` (the modeled service error, from which `.into_err()` yields `E`). For `get`/`head`, the not-found case is the service error `GetObjectError::NoSuchKey` / a 404 `HeadObjectError`; these map to `Code::ResourceNotFound`. `SdkError` exposes `.raw_response()` for the HTTP status when present.

## Plan of Work

All source paths below are relative to the submodule root.

### M1 — Dependencies

Edit the workspace `Cargo.toml` `[workspace.dependencies]` table (alphabetically among the AWS entries), adding:

    aws-sdk-s3 = { version = "1.137", default-features = false, features = ["behavior-version-latest", "rt-tokio", "sigv4a", "http-1x", "rustls", "default-https-client"] }
    aws-smithy-http-client = { version = "1.1", default-features = false }
    aws-smithy-types = { version = "1.5", default-features = false }

Rationale for the feature list: reproduce the S3 default set (`sigv4a`, `http-1x`, `rustls`, `default-https-client`, `rt-tokio`) explicitly and add `behavior-version-latest`. Keeping `default-features = false` + an explicit list makes the TLS choice auditable in one place.

Edit `agent/Cargo.toml`:

- In `[dependencies]` add `aws-sdk-s3 = { workspace = true }`.
- In the agent crate's `[features]`, extend the `test` feature so it enables the SDK test-util path:

      test = ["aws-sdk-s3/test-util"]

- In `[dev-dependencies]` add:

      aws-sdk-s3 = { workspace = true, features = ["test-util"] }
      aws-smithy-http-client = { workspace = true, features = ["test-util"] }
      aws-smithy-types = { workspace = true }

Then regenerate the lock and confirm a clean build (see Concrete Steps M1). If `cargo machete`/`cargo diet` (run inside `./scripts/lint.sh`) flags `aws-smithy-http-client`/`aws-smithy-types` as unused because they only appear in dev/test code, keep them as dev-dependencies only (they are used from `agent/tests/`, which machete counts) — do not move them to normal `[dependencies]`.

### M2 — The `object_store` module

Create `agent/src/object_store/errors.rs`. Mirror `agent/src/http/errors.rs` exactly in shape:

- Define per-kind leaf error structs, each `#[derive(Debug, thiserror::Error)]`, each with `pub trace: Box<Trace>`, each `impl crate::errors::Error`:
  - `ObjectNotFoundErr { key: String, trace }` — `#[error("object not found: {key}")]`; override `code()` → `Code::ResourceNotFound` and `http_status()` → `HTTPCode::NOT_FOUND`.
  - `ConnectionErr { key: String, msg: String, trace }` — for `SdkError::DispatchFailure`/`TimeoutError`; override `is_network_conn_err()` → `true`.
  - `RequestFailedErr { operation: String, key: Option<String>, status: Option<u16>, msg: String, trace }` — for other `ServiceError`/`ResponseError` cases; default trait impls (internal server error).
  - `InvalidResponseErr { operation: String, msg: String, trace }` — for malformed/parse issues (e.g. a body that cannot be collected). Default impls.
- Define the aggregating enum `ObjectStoreErr` with `#[error(transparent)]` variants for each leaf, plus `crate::impl_error!(ObjectStoreErr { ObjectNotFoundErr, ConnectionErr, RequestFailedErr, InvalidResponseErr });`.
- Add mapping helpers that convert `aws_sdk_s3::error::SdkError<E, R>` into `ObjectStoreErr`. Because each operation has a distinct `E`, write a small generic helper over the common `SdkError` variants that need no operation-specific knowledge (Construction/Timeout/Dispatch/Response), and let each operation decide its not-found case from its own service-error enum before delegating. Concretely:
  - `fn map_sdk_err_common<E, R>(operation: &str, key: Option<String>, err: SdkError<E, R>) -> ObjectStoreErr` handling `TimeoutError`/`DispatchFailure` → `ConnectionErr`, `ResponseError` → `RequestFailedErr` (with status from `raw_response()` when present), `ConstructionFailure` → `RequestFailedErr`, and `ServiceError` → `RequestFailedErr` fallback. This helper takes the already-classified non-not-found path.
  - In `get_object`, match `err.into_service_error()` for `GetObjectError::NoSuchKey(_)` → `ObjectNotFoundErr`; otherwise call the common mapper.
  - In `object_exists`, treat a `HeadObjectError` whose HTTP status is 404 as "not found" → return `Ok(false)` (not an error); any other error maps via the common mapper.

Create `agent/src/object_store/mod.rs`:

- Module docs (a short `//! ` comment) stating this is remote S3 object storage, distinct from local `crate::storage`, and that it only uses caller-supplied temporary credentials.
- `pub mod errors;` and `pub use errors::ObjectStoreErr;`.
- `pub struct S3Store { client: aws_sdk_s3::Client, bucket: String }`.
- A `pub struct Credentials { pub access_key_id: String, pub secret_access_key: String, pub session_token: String }` (a plain input DTO) **or** accept the three strings directly on the constructor — pick the DTO if it reads cleaner; keep it self-contained to this module.
- Constructor `pub fn new(creds: Credentials, region: String, bucket: String) -> Self` that builds `aws_sdk_s3::config::Credentials::new(creds.access_key_id, creds.secret_access_key, Some(creds.session_token), None, "miru-agent")`, an `aws_sdk_s3::config::Config::builder().region(Region::new(region)).credentials_provider(...).behavior_version_latest()`-style config, and `aws_sdk_s3::Client::from_conf(config)`. No network happens at construction.
- A test-only constructor gated `#[cfg(feature = "test")]`: `pub fn with_http_client(http_client, region, bucket) -> Self` (or `from_conf_parts`) that builds the same config but installs a caller-provided `SharedHttpClient` (the `StaticReplayClient`). This is the seam the tests use to inject canned HTTP. Keep credentials dummy/static inside it.
- The five async methods, each `-> Result<_, ObjectStoreErr>`:
  - `put_object(&self, key: &str, bytes: Vec<u8>)` → `self.client.put_object().bucket(&self.bucket).key(key).body(ByteStream::from(bytes)).send()`, mapping errors via the common mapper; returns `()` on success.
  - `get_object(&self, key: &str) -> Vec<u8>` → `get_object().bucket().key().send()`, mapping `NoSuchKey`/404 → `ObjectNotFoundErr`; on success `.body.collect().await` → map a collect failure to `InvalidResponseErr` → `.into_bytes().to_vec()`.
  - `delete_object(&self, key: &str)` → `delete_object().bucket().key().send()`; success `()`.
  - `object_exists(&self, key: &str) -> bool` → `head_object().bucket().key().send()`; `Ok(_)` → `true`; a 404 `HeadObjectError` → `false`; other errors propagate.
  - `list_objects(&self, prefix: &str) -> Vec<String>` → `list_objects_v2().bucket().prefix().send()`; collect `output.contents()` keys into a `Vec<String>` (skip `None` keys). For this PR a single page is acceptable; note pagination as a follow-up in the Decision Log if the response is truncated (do not silently drop a truncated flag — record it as a discovery).

Register the module: add `pub mod object_store;` to `agent/src/lib.rs` (keep the list alphabetically ordered — it goes between `network` and `privilege`).

Keep all imports grouped and commented per the import-ordering convention.

### M3 — Offline unit tests

Create `agent/tests/object_store/mod.rs` and register it: add `pub mod object_store;` to `agent/tests/mod.rs` (alphabetical, between `network` and `privilege`).

Write a small helper in the test module that builds an `S3Store` wired to a `StaticReplayClient` (from `aws_smithy_http_client::test_util::{StaticReplayClient, ReplayEvent}`) constructed from a `Vec<ReplayEvent>`. Each `ReplayEvent` pairs an expected `http::Request<SdkBody>` with the canned `http::Response<SdkBody>` to return (the `http` crate here is `http` 1.x; `SdkBody` is `aws_smithy_types::body::SdkBody`). Use `miru_agent::object_store::S3Store::with_http_client(...)` (the `#[cfg(feature = "test")]` seam). After each call, use `replay_client.assert_requests_match(&[])` (or inspect `.actual_requests()`) to assert method/URI/body where the task requires it.

Cover exactly these cases (each a `#[tokio::test]`, grouped in nested `pub mod put/get/delete/exists/list`):

- `put` success: canned `200`; assert the call returns `Ok(())` and that the recorded request was `PUT /<bucket>/<key>` with the exact body bytes.
- `get` success: canned `200` with a known body; assert the returned `Vec<u8>` equals the canned bytes (byte round-trip) and the request was `GET /<bucket>/<key>`.
- `get` not-found: canned `404` with an S3 `NoSuchKey` error XML body; assert `Err(ObjectStoreErr::ObjectNotFoundErr(_))` and that `err.code()` is `Code::ResourceNotFound` / `err.http_status()` is 404.
- `delete` success: canned `204`; assert `Ok(())` and request `DELETE /<bucket>/<key>`.
- `exists` true: canned HEAD `200`; assert `Ok(true)`.
- `exists` false: canned HEAD `404`; assert `Ok(false)` (a 404 is not an error for `object_exists`).
- `list` returns keys: canned `200` with a `ListBucketResult` XML body containing two `<Contents><Key>` entries; assert the returned `Vec<String>` equals those two keys in order.

Keep the canned XML bodies as short `const &str` fixtures inline in the test file (S3 uses XML, not JSON, for these payloads — the SDK parses XML). If constructing exact request-body assertions for `put` proves brittle across SDK header/signing differences, assert method + URI + body and pass `&["authorization", "x-amz-date", "x-amz-content-sha256", ...]` to `assert_requests_match`'s ignore list so signing headers are not compared.

### M4 — Coverage gate + preflight

Add `agent/src/object_store/.covgate`. Start with a placeholder threshold of `0` (which the covgate script treats as "skip") only long enough to get a first green run, then measure actual coverage from `./scripts/covgate.sh` output and set the threshold to just below the measured value (round down to two decimals, matching the style of the existing gates like `93.90`). The seven tests above are expected to cover well over 90% of the module's regions; aim the gate at the measured number so the module is genuinely gated, not skipped. Do not ship a `0` gate.

Run the full local preflight and fix any lint (import order, clippy, fmt, machete/diet) or coverage findings until it reports clean.

## Concrete Steps

Working directory for all commands: the submodule root (`/home/ben/miru/workbench5/repos/agent`).

### M1

1. Edit `Cargo.toml` and `agent/Cargo.toml` as described in Plan of Work M1.
2. Refresh the lockfile:

       ./scripts/update-deps.sh

   Expect it to print "Updating Cargo dependencies" and resolve `aws-sdk-s3`, `aws-smithy-http-client`, `aws-smithy-types`, and their transitive crates with no version-conflict errors.
3. Confirm the workspace still builds:

       cargo build

   Expect it to finish with `Finished ...` and no errors. (Warnings from generated `libs/` crates are pre-existing and unrelated.)
4. Commit milestone M1:

       git add Cargo.toml agent/Cargo.toml
       git commit -m "build(agent): add aws-sdk-s3 and smithy test deps for object_store"

   (Commits must be signed; if `git commit` produces an unsigned commit, re-sign before pushing — Miru branch protection requires verified signatures.)

### M2

5. Create `agent/src/object_store/errors.rs` and `agent/src/object_store/mod.rs`; add `pub mod object_store;` to `agent/src/lib.rs`.
6. Type-check without running tests:

       cargo build

   Expect a clean build. Resolve any error-mapping type mismatches against the actual `SdkError`/operation-error enums (the SDK version is pinned, so the enum variant names are stable).
7. Commit milestone M2:

       git add agent/src/object_store agent/src/lib.rs
       git commit -m "feat(agent): add object_store S3 CRUD module"

### M3

8. Create `agent/tests/object_store/mod.rs`; add `pub mod object_store;` to `agent/tests/mod.rs`.
9. Run the module's tests:

       ./scripts/test.sh

   This runs `RUST_LOG=off cargo test --features test`. Expect all `object_store` tests to pass, e.g. a line like:

       test object_store::get::success::get_round_trips_bytes ... ok

   and the overall run to report `test result: ok.` with the pre-existing suite still green.
10. Commit milestone M3:

        git add agent/tests/object_store agent/tests/mod.rs
        git commit -m "test(agent): offline S3 CRUD tests via StaticReplayClient"

### M4

11. Add `agent/src/object_store/.covgate` (initially `0`). Measure coverage:

        ./scripts/covgate.sh

    Read the `object_store: NN.NN%` line from the output and set the `.covgate` threshold just below it.
12. Refresh the lock (in case dep features shifted) and run the full local gate:

        ./scripts/update-deps.sh
        ./scripts/preflight.sh

    `preflight.sh` runs lint, coverage, and the tools lint/tests in parallel and must end with:

        Preflight clean

    If it prints `Preflight FAILED (...)`, read the per-section output it dumps (`=== Lint ===`, `=== Tests ===`), fix the findings (import order, clippy `-D warnings`, `cargo fmt`, `cargo machete`/`diet` unused-dep flags, or a coverage shortfall), and re-run until clean.
13. Commit milestone M4:

        git add agent/src/object_store/.covgate
        git commit -m "test(agent): gate object_store coverage; preflight clean"

## Validation and Acceptance

- **Build:** From the submodule root, `cargo build` succeeds with no errors.
- **Tests:** `./scripts/test.sh` passes. The new tests fail before the module exists (they will not compile) and pass after. Specifically, all seven `object_store` cases are green: `put` success, `get` byte round-trip, `get` 404 → `ObjectStoreErr::ObjectNotFoundErr` with `code() == Code::ResourceNotFound`, `delete` success, `exists` true (HEAD 200), `exists` false (HEAD 404), `list` returns the two canned keys in order. No live network or AWS credentials are used — the `StaticReplayClient` serves every response.
- **Lint:** `./scripts/lint.sh` is clean (custom import linter, `cargo fmt --check`, `cargo machete`, `cargo diet`, `rustsec audit`, and `cargo clippy --all-features -- -D warnings` all pass).
- **Coverage:** `./scripts/covgate.sh` passes; `object_store` meets its `.covgate` threshold (a real non-zero gate set just below the measured coverage).
- **Preflight (required before opening the PR):** `./scripts/preflight.sh` prints `Preflight clean`. The PR must not be opened until this holds.

## Idempotence and Recovery

- Editing `Cargo.toml`/`agent/Cargo.toml` and re-running `./scripts/update-deps.sh` is safe to repeat; `cargo update` reconverges the lock. If dependency resolution fails, revert the dep edits (`git checkout -- Cargo.toml agent/Cargo.toml`) and re-apply them one crate at a time to isolate the conflict.
- Creating the module files is idempotent; re-running `cargo build` after edits is safe.
- The tests hit no external services and hold no global OS resources, so they need no `#[serial]` annotation and can be re-run freely in parallel.
- The `.covgate` threshold can be adjusted and `./scripts/covgate.sh` re-run any number of times; if a later edit drops coverage below the gate, add tests or lower the gate to the new measured value (never below what the tests actually achieve).
- Each milestone ends in its own commit, so any milestone can be inspected or reverted independently with `git revert`/`git reset` while leaving earlier milestones intact.
