# Align the `gcs` object-storage module's shape with the `s3` module

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Refactor `agent/src/gcs/mod.rs` and `agent/src/gcs/errors.rs`; rewrite `agent/tests/gcs/mod.rs` and split it into `agent/tests/gcs/multipart.rs` if warranted (see Decision Log); adjust `agent/src/gcs/.covgate` only if the achieved coverage floor moves. No new deps, no `lib.rs`/`tests/mod.rs` registration changes (the module is already wired). |
| `agent/src/s3/` | read-only | The canonical sibling being mirrored. Do NOT modify it. |
| `agent/tests/s3/` | read-only | The canonical test-layout reference. Do NOT modify it. |
| `agent/tests/mocks/http_client.rs` | read-only | Provides `run_server(router) -> Server { base_url }`. Do NOT modify it. |

This plan lives in `plans/backlog/` because it is not yet started; move it to `plans/completed/` at the end.

Note on git: the working branch is `feat/gcs-object-storage-crud`, freshly rebased onto `main`, so BOTH `s3` and `gcs` modules are present side by side. Delivery is **push mode to the existing branch (PR #103)**. This is a pure refactor: observable behavior and semantics stay identical; only shape/naming/organization change.

## Purpose / Big Picture

The `gcs` module already implements the four object-storage operations against Google Cloud Storage and passes its offline test suite. It was, however, built with a different public API shape than the canonical `s3` module: `GcsStore` instead of `Store`, `&str` keys + `&Path` instead of the `Object`/`File` value types, method names `put_object`/`get_object`/`delete_object`/`object_exists` instead of `put`/`get`/`delete`/`exists`, a thinner error taxonomy, and a single flat test file.

After this refactor a caller uses `gcs::Store` exactly the way they use `s3::Store`, module-qualified so `gcs::Store` and `s3::Store` read as faithful siblings:

- `gcs::Config { creds: gcs::Credentials, bucket: String }`
- `gcs::Credentials { access_token: String }` (wraps the short-lived OAuth2 token; replaces `StaticTokenCredentials` as the public value type)
- `gcs::Object { bucket: String, key: String }` with `Display` (`gs://<bucket>/<key>`)
- `Store::new(cfg) -> Result<Self, GcsErr>` and a test-only `Store::from_stub_and_endpoint(...)`-style seam
- `put(&self, src: File, dst: &Object)`, `put_singlepart(&self, src: &File, dst: &Object)`, `get(&self, src: &Object, dest: &File)`, `delete(&self, obj: &Object)`, `exists(&self, obj: &Object) -> bool`

The genuine GCS invariants are preserved: the store is constructed **only** from a caller-supplied short-lived OAuth2 token + bucket (never ADC/env/metadata), object bodies stream to/from disk (never buffered whole), and a missing object surfaces as `Code::ResourceNotFound` (HTTP 404). You can see it working by running `./scripts/test.sh` (all offline) and confirming `./scripts/covgate.sh` keeps `gcs` at/above its gate.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Milestone 1 — `gcs/errors.rs` reaches structural parity with `s3/errors.rs`.
- [ ] Milestone 2 — `gcs/mod.rs` value types (`Config`, `Credentials`, `Object`) + `Store` rename + constructor alignment.
- [ ] Milestone 3 — method rename/re-signature (`put`/`put_singlepart`/`get`/`delete`/`exists`) using `Object`/`File`.
- [ ] Milestone 4 — rewrite `tests/gcs/mod.rs` to mirror `tests/s3/mod.rs` structure; add `tests/gcs/multipart.rs` iff warranted.
- [ ] Milestone 5 — `./scripts/test.sh` green; `./scripts/covgate.sh` keeps `gcs` ≥ its gate.
- [ ] Milestone 6 — `./scripts/preflight.sh` prints `Preflight clean`; ready to push to PR #103.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Rename `GcsStore` → `Store` and always refer to it module-qualified as `gcs::Store`, mirroring `s3::Store`.
  Rationale: Direct task requirement; the two modules should read as siblings. Both crates already alias their SDK import so the bare `Store` name does not collide.
  Date/Author: 2026-07-09 / plan author.

- Decision: Introduce `gcs::Object { bucket, key }` (with `Display` → `gs://<bucket>/<key>`) as the object identifier, replacing raw `&str` keys, and take/return `crate::filesys::file::File` in the ops rather than `&Path`.
  Rationale: Mirrors `s3::Object` (which prints `s3://<bucket>/<key>`) and the `File`/`Source` value types. GCS's canonical URI scheme is `gs://`, so the `Display` prefix is the one honest deviation from s3's `s3://`.
  Date/Author: 2026-07-09 / plan author.

- Decision: `gcs::Config { creds: gcs::Credentials, bucket: String }` and `gcs::Credentials { access_token: String }`. `Config` carries `bucket` (not `region`).
  Rationale: `s3::Config` carries `region` because S3 requires a region; GCS does not. GCS instead needs the bucket to form the `projects/_/buckets/<bucket>` resource path. `bucket` therefore takes `region`'s structural slot. Add a `//` comment at the field explaining the substitution. A `#[cfg(feature = "test")] Default for Credentials` mirrors s3's test-only default.
  Date/Author: 2026-07-09 / plan author.

- Decision: `put` routes by size to `put_singlepart` (small) exactly like s3, but there is **no** `put_multipart`/`gcs/multipart.rs` split — GCS has no user-visible multipart/resumable API to mirror.
  Rationale (SDK-forced): The `google-cloud-storage` `write_object(...).send_unbuffered()` builder performs a single streaming upload and the SDK **internally** chooses simple vs resumable transfer based on payload size (`write_object.rs:1021`). There is no `create/upload_part/complete/abort` surface and no `upload_id` to resume, so s3's `multipart.rs` (create → upload_part → complete → abort, plus `resume_multipart_upload`, `part_plan`, `part_size_for`) has no GCS analog. `put` still routes by `files::size(&src)` so the public two-method shape (`put` + `put_singlepart`) matches s3; both branches call the same `write_object` under the hood, with a `//` comment noting the SDK folds multipart in. `put_singlepart` is the honest name for "one `write_object` call".
  Consequence for file layout: keep GCS's upload inline in `mod.rs`; do NOT create `gcs/multipart.rs`. See "Consider a multipart split" below for the rejected alternative.
  Date/Author: 2026-07-09 / plan author.

- Decision: Test file layout mirrors `tests/s3/mod.rs` (nested `pub mod put { single, access_denied, source_missing, routing }`, `get { success, dest_unwritable, not_found, access_denied, transport_failure }`, `delete { success, access_denied }`, `exists { present, absent, access_denied }`, `construction`, `error_types`), but does NOT add `tests/gcs/multipart.rs` — there is no multipart surface to test. If `tests/mod.rs` currently declares only `pub mod gcs;`, leave it; do not add a `pub mod multipart;` under gcs.
  Rationale: Faithful to s3's grouping while honest to the SDK. `tests/gcs/mod.rs` keeps the axum HTTP mock for the data path (put/get) and the `mockall` `StorageControl` stub for the control path (delete/exists), unchanged in mechanism — only renamed call sites and the new `Object`/`File` value types.
  Date/Author: 2026-07-09 / plan author.

- Decision: Bring `gcs/errors.rs` to structural parity by adding `LocalIoErr` and the body/io mapper fns; do NOT add `NoSuchUploadErr` or the `ByteStream`-specific mappers.
  Rationale: s3 has `LocalIoErr` (distinct from `InvalidResponseErr`) for local filesystem failures while streaming, plus `map_body_io_err`, `map_bytestream_err`, `map_body_read_err`, `missing_response_field`, and `NoSuchUploadErr`. GCS today overloads `InvalidResponseErr` for local I/O — align it by adding `LocalIoErr` and a `map_body_io_err(operation, obj: &Object, file: &File, err) -> GcsErr` mirroring s3's signature. GCS has no `ByteStream` (it uses `tokio::fs::File` + the SDK's `ReadObjectResponse::next()`), so `map_bytestream_err` and `map_body_read_err` have no analog — a `//` comment records that a body-read failure surfaces through `map_gcs_err` instead. `NoSuchUploadErr` and `missing_response_field` have no analog because there is no multipart/upload-id surface and the SDK finalizes the response object itself. Add `//` comments at the `GcsErr` enum noting which s3 variants are intentionally absent and why.
  Date/Author: 2026-07-09 / plan author.

- Decision: Keep `gcs/.covgate` at `88.00` unless the refactor measurably changes the achievable floor.
  Rationale: The task states the gate is 88.00 (the reference CRUD plan proposed 85.00 but the committed value is 88.00). This is a refactor, not a feature, so coverage should not regress. If adding `LocalIoErr` introduces a new uncovered branch, add a targeted test rather than lowering the gate. Only adjust the number with a Decision Log entry and never below the achieved coverage.
  Date/Author: 2026-07-09 / plan author.

## Context and Orientation

Read this fully before starting. It gives a novice everything needed with no prior repo knowledge.

### The two modules, side by side

Canonical reference (do not modify):
- `agent/src/s3/mod.rs` — `Config { creds, region }`, `Credentials { access_key_id, secret_access_key, session_token }` (+ `#[cfg(feature="test")] Default`), `Object { bucket, key }` + `Display` (`s3://…`), `Store { client }`. Constructors `Store::new(cfg) -> Self` (infallible) and `#[cfg(feature="test")] Store::from_http_client(http_client, cfg) -> Self`. Methods `put(src: File, dst: &Object)`, `put_singlepart(&File, &Object)`, `get(&Object, &File)`, `delete(&Object)`, `exists(&Object) -> bool`. `put` routes by `files::size` between `put_singlepart` and `put_multipart`.
- `agent/src/s3/errors.rs` — leaf structs `ObjectNotFoundErr`, `NoSuchUploadErr`, `ConnectionErr`, `RequestFailedErr` (custom `Display`), `InvalidResponseErr`, `LocalIoErr`; enum `S3Err` (+ `From<FileSysErr>`) via `crate::impl_error!`; mappers `is_not_found`, `map_sdk_err`, `map_body_io_err`, `map_bytestream_err`, `map_body_read_err`, `missing_response_field`; a `#[cfg(test)] mod tests` with a `body_mappers` submodule and an `error_types` submodule.
- `agent/src/s3/multipart.rs` — the stateless/resumable multipart surface (no GCS analog).
- `agent/tests/s3/mod.rs` + `agent/tests/s3/multipart.rs` — the offline test layout to mirror.

To refactor:
- `agent/src/gcs/mod.rs` — currently `GcsStore`, `StaticTokenCredentials`, `new(access_token, bucket, endpoint)`, `with_control_stub(stub, bucket, endpoint)`, `put_object(key: &str, path: &Path)`, `get_object(key, dest)`, `delete_object(key)`, `object_exists(key) -> bool`, plus private `classify_read_err`, `map_body_io_err`, `build_err`, and the `StaticTokenCredentials` `CredentialsProvider` impl + its unit tests.
- `agent/src/gcs/errors.rs` — `ObjectNotFoundErr`, `ConnectionErr`, `RequestFailedErr` (field `key`, custom `Display`), `InvalidResponseErr`; enum `GcsErr`; `is_not_found(&GaxError)`, `map_gcs_err(operation, key: Option<&str>, err)`. No `LocalIoErr`, no body mappers, no `From<FileSysErr>`.
- `agent/tests/gcs/mod.rs` — flat file with the axum HTTP data-path mock, the `mockall` control-path stub, and per-op `pub mod`s (`put`, `get`, `delete`, `exists`, `construction`, `error_types`).

### The GCS SDK facts that force deviations (verified)

1. Upload has no multipart surface: `Storage::write_object(bucket_resource, key, tokio::fs::File).send_unbuffered()` streams the file and the SDK internally picks simple vs resumable transfer by size (`google-cloud-storage-1.15.0/src/storage/write_object.rs:1021`). → no `put_multipart`, no `gcs/multipart.rs`, no `upload_id`, no `part_plan`.
2. Two transports: data client (`Storage`, HTTP/JSON) for put/get; control client (`StorageControl`, gRPC) for delete/exists. s3 has one HTTP client. → the test-only constructor must build BOTH a data client (endpoint override → axum mock) and a control client (`StorageControl::from_stub(mock)`), so it cannot be a drop-in `from_http_client`. Keep the gRPC-vs-HTTP reality but present it in the s3 idiom (see Milestone 2).
3. Single crate error type `google_cloud_gax::error::Error`, not a generic `SdkError<E>`. → `map_gcs_err` takes the concrete error; the s3 generic `<E>` machinery has no analog. `is_not_found` checks the gRPC `Code::NotFound` OR HTTP 404 (`http_status_code() == Some(404)`) because the two transports report not-found differently.
4. No `ByteStream`: downloads iterate `ReadObjectResponse::next() -> Option<Result<Bytes>>`; a wire read error is a `google_cloud_gax::error::Error`, mapped by `map_gcs_err`. → `map_bytestream_err`/`map_body_read_err` have no analog.
5. Token → header can fail: `HeaderValue::from_str(format!("Bearer {token}"))` errors on an invalid byte, so `Store::new` is fallible (`-> Result<Self, GcsErr>`), unlike s3's infallible `new`. This is an existing, genuine deviation — keep it and comment it.

### Repo conventions (from `agent/AGENTS.md`)

- Import ordering in every source file: `// standard crates`, blank, `// internal crates`, blank, `// external crates`. Enforced by the custom import linter in `scripts/lint.sh`.
- Leaf errors derive `thiserror::Error` + implement `crate::errors::Error`; aggregating enums use `crate::impl_error!`; `crate::trace!()` builds the `Box<Trace>`.
- `#[cfg(feature = "test")]` gates test-only constructors/seams.
- Tests run via `./scripts/test.sh` (`RUST_LOG=off cargo test --features test`). gcs tests bind only ephemeral `127.0.0.1:0` ports + temp files → no `#[serial]`.
- Field-by-field assert lint: 4+ `assert_eq!` on fields of the same variable in one test triggers the linter; suppress with `// lint:allow(field-by-field-assert)` only when unavoidable. Prefer matching s3's assertion style (single `matches!` + a couple of scalar asserts), which stays under the threshold.
- Coverage gate per module via `.covgate`; enforce with `scripts/covgate.sh`.

## Plan of Work

Six milestones; commit after each. Keep behavior identical throughout — every change is rename/re-shape.

### Milestone 1 — `agent/src/gcs/errors.rs` structural parity

Mirror `s3/errors.rs` section-for-section, GCS-adapted:

1. Keep `ObjectNotFoundErr`, `ConnectionErr`, `RequestFailedErr`, `InvalidResponseErr` as-is (they already match s3 shape; `RequestFailedErr.key` stays `key` — s3 calls it `object`; rename the field to `object: Option<String>` to match s3 exactly, and update its `Display` and `map_gcs_err`). Adjust the `Display` string to keep saying `GCS {operation} request for object '{object}' failed with status {status}: {msg}`.
2. Add `LocalIoErr { operation, object, msg, trace }` mirroring s3's, with `impl crate::errors::Error` (defaults → 500). `#[error("local I/O error during {operation} for object '{object}': {msg}")]`.
3. Add `LocalIoErr` to the `GcsErr` enum and to `crate::impl_error!(GcsErr { … })`. Above the enum, add `//` comments listing the s3 variants deliberately omitted and why: `NoSuchUploadErr` (no multipart/upload-id surface) and `FileSysErr` (see step 4 — add only if the ops actually convert a `FileSysErr`).
4. Decide on `FileSysErr`: s3 has `From<filesys::FileSysErr> for S3Err` because `s3::put` calls `files::size(&src)?` which yields a `FileSysErr`. If GCS's new `put` routes by `files::size` too (Milestone 3), add the identical `GcsErr::FileSysErr(filesys::FileSysErr)` variant + `From` impl + `impl_error!` entry. Otherwise omit it and comment why.
5. Add `pub fn map_body_io_err(operation: &str, obj: &Object, file: &File, err: std::io::Error) -> GcsErr` returning `GcsErr::LocalIoErr` (mirror s3's message: `filesystem I/O error at path '{file}': {err}`). Import `use crate::gcs::Object;` and `use crate::filesys::{self, file::File};` in the internal-crates group.
6. Keep `is_not_found` and `map_gcs_err` (concrete error type). Add a `//` comment on `map_gcs_err` noting it is the analog of s3's `map_sdk_err` AND absorbs the body-read-error role that s3 splits into `map_body_read_err` (no `ByteStream` in GCS).
7. Tests: keep the existing mapper tests. Add a `body_mappers` submodule mirroring s3's, testing `map_body_io_err` → `LocalIoErr` (with a `gs://bucket/key` object) and NOT a network error. Expand the `error_types`-style assertions (they currently live in `tests/gcs/mod.rs`, not the `#[cfg(test)]` module — leave them in the integration test to match s3, which also puts `error_types` in `tests/s3/mod.rs`; only the mapper/body tests live in `src`).

Build check: `cargo build -p miru-agent --features test` after Milestones 1–3 together (errors reference `Object`/`File` which land in Milestone 2/3), to avoid a mid-refactor non-compiling state.

### Milestone 2 — `agent/src/gcs/mod.rs` value types + `Store` + constructors

1. Rename `struct GcsStore` → `struct Store`. Update the module doc comment: replace every `GcsStore` with `Store`, and adjust the "sibling of the S3 module" wording to state the API now mirrors `s3::Store`. Keep the ADC/env/metadata invariant paragraph and the streaming paragraph. Update the transports paragraph. Keep the deferred-integration-test TODO.
2. Add value types mirroring s3:
   - `pub struct Config { pub creds: Credentials, pub bucket: String }` — with a `//` comment: "GCS has no region; `bucket` occupies the slot `s3::Config::region` holds and forms the `projects/_/buckets/<bucket>` resource path."
   - `pub struct Credentials { pub access_token: String }` (+ `#[cfg(feature = "test")] impl Default` returning a dummy token) — this replaces the public role of `StaticTokenCredentials`. Keep the private `StaticTokenCredentials` `CredentialsProvider` impl internal (it is the SDK plumbing, built from `cfg.creds.access_token`); add a `//` comment that `Credentials` is the caller-facing value type and `StaticTokenCredentials` is the internal provider it is turned into.
   - `pub struct Object { pub bucket: String, pub key: String }` + `impl Display` → `write!(f, "gs://{}/{}", self.bucket, self.key)` (comment the `gs://` vs `s3://` deviation).
3. Constructors:
   - `pub async fn new(cfg: Config) -> Result<Self, GcsErr>` — build the `Bearer` header from `cfg.creds.access_token`, build both clients, store `bucket_resource = format!("projects/_/buckets/{}", cfg.bucket)`. Keep the fallible-`new` comment (token → header can fail). Note in a `//` comment that s3's `new` is infallible + sync while GCS's is fallible + async because the header can be invalid and the GCS builders are async.
   - Replace `with_control_stub(stub, bucket, endpoint)` with a test seam named to echo s3's `from_http_client`, e.g. `#[cfg(feature = "test")] pub async fn from_stub(stub, cfg: Config, endpoint: String) -> Result<Self, GcsErr>` (bikeshed the exact name during implementation; prefer one that reads as the GCS counterpart of `from_http_client`). It takes a `Config` (for the bucket) + the control stub + the data-client endpoint, mirroring s3's `(http_client, cfg)` pairing as closely as two transports allow. Add a `//` comment: "s3 injects one HTTP client; GCS must inject a gRPC control stub AND point the HTTP data client at `endpoint`, so the seam takes both."
4. `Object` now lives in this module; make sure `errors.rs`'s `use crate::gcs::Object;` resolves.

### Milestone 3 — method rename + re-signature

Rewrite the four ops to the s3 signatures, keeping identical behavior:

1. `pub async fn put(&self, src: File, dst: &Object) -> Result<(), GcsErr>` — mirror `s3::Store::put`: `let size = files::size(&src).await?; if size > PART_SIZE { … } else { self.put_singlepart(&src, dst).await }`. Since GCS has no multipart, BOTH branches call `put_singlepart` (or `put` just calls `put_singlepart` unconditionally); include the size read only if you also add the `FileSysErr` variant, so the missing-source test surfaces `FileSysErr` exactly like s3. Add a `//` comment: "GCS `write_object` folds the multipart/resumable decision inside the SDK, so there is no size-routed second path; `put` and `put_singlepart` share one implementation." If keeping a single path is cleaner, have `put` delegate straight to `put_singlepart` and drop the size read — but then the missing-source error is whatever `tokio::fs::File::open` maps to (`LocalIoErr`), NOT `FileSysErr`; pick one and make the test assert match. Prefer mirroring s3 (read size → `FileSysErr` on missing source) for maximum shape parity.
2. `pub async fn put_singlepart(&self, src: &File, dst: &Object) -> Result<(), GcsErr>` — open `src.path()` via `tokio::fs::File::open`, map open failure with `errors::map_body_io_err("put_object", dst, src, e)`, then `self.data.write_object(&self.bucket_resource, &dst.key, file).send_unbuffered().await.map_err(|e| errors::map_gcs_err("put_object", Some(&dst.key), e))?`. Use `dst.bucket` to validate/override the resource path if the store is single-bucket-scoped — keep the existing `bucket_resource` field (constructed from `cfg.bucket`); the `Object.bucket` should match it. Mirror how s3 uses `dst.bucket` directly; for GCS, either assert `dst.bucket` equals the store's bucket or rebuild the resource path from `dst.bucket` each call. Prefer rebuilding from `dst.bucket` so `Object` fully identifies the target like s3 (comment the choice).
3. `pub async fn get(&self, src: &Object, dest: &File) -> Result<(), GcsErr>` — `read_object(resource, &src.key).send()`, classify not-found via `classify_read_err` → `ObjectNotFoundErr`, create `dest.path()` mapping io failures with `map_body_io_err("get_object", src, dest, e)`, stream `resp.next()` chunks mapping wire errors with `map_gcs_err("get_object", Some(&src.key), e)`, `write_all` + `flush` mapping io with `map_body_io_err`. This mirrors s3's `get` control flow exactly.
4. `pub async fn delete(&self, obj: &Object) -> Result<(), GcsErr>` — control-plane delete; keep the idempotent NOT_FOUND → `Ok(())` behavior (comment: matches s3's idempotent delete).
5. `pub async fn exists(&self, obj: &Object) -> Result<bool, GcsErr>` — control-plane get-metadata; NOT_FOUND → `Ok(false)`, else propagate via `map_gcs_err`.
6. Update the private helpers: `classify_read_err(&self, src: &Object, err)`, drop the old inherent `map_body_io_err` method (now a free fn in `errors.rs` taking `&Object`/`&File`), keep `build_err`. Ensure import ordering: add `use crate::filesys::{file::File, files, path::PathExt};` in the internal-crates group (mirrors s3).
7. If a `PART_SIZE`-style constant is introduced for the `put` size read, add it `pub(crate) const PART_SIZE: u64 = 8 * 1024 * 1024;` with a comment that for GCS it is only the `put`/`put_singlepart` routing boundary and the SDK handles the actual chunking — or omit it and route unconditionally (see Milestone 3 step 1).

### Milestone 4 — `agent/tests/gcs/mod.rs` layout mirror (+ maybe `multipart.rs`)

1. Update imports: `use miru_agent::gcs::{Config, Credentials, Object, GcsErr, Store};` (and `gcs::errors::{…}` for the leaf types used in `error_types`). Remove `use …::GcsStore;`.
2. Add helpers mirroring s3: `fn obj(key: &str) -> Object { Object { bucket: BUCKET.into(), key: key.into() } }`; keep `temp_file_with`. Where s3 uses `files::temp("s3-test")` + `File`, either keep `NamedTempFile` (GCS test currently uses it) or switch to the crate's `files::temp`/`File` to match s3 more closely — prefer switching so `get`/`put` take `&File`/`File` the same way s3's tests do. Confirm `miru_agent::filesys::file::File` + `files` are accessible from the test crate (s3 tests import them, so they are).
3. Rewrite call sites: `store.put(src.to_file(), &obj(key))`, `store.put_singlepart(src.file(), &obj(key))`, `store.get(&obj(key), dest.file())`, `store.delete(&obj(key))`, `store.exists(&obj(key))`.
4. Build the store via the new constructors: HTTP data-path tests build `Store::new(Config { creds: Credentials::default()/{access_token}, bucket })` with the endpoint-override seam; control-path tests build via the renamed `from_stub`-style constructor. Keep the axum router + `run_server` + `HttpRecorder` and the `mockall! StorageControl` block verbatim (only the constructor call and store type change).
5. Reorganize into nested `pub mod`s matching s3's tree where a matching case exists:
   - `put { single, access_denied, source_missing }` — `single::upload_streams_file_body`, `access_denied::upload_error_maps_to_request_failed`, `source_missing::upload_missing_source_maps_to_*` (assert `FileSysErr` if the size-read path is kept, else `LocalIoErr`).
   - `get { success, dest_unwritable, not_found, access_denied }` — mirror s3's `get_streams_body_to_file`, `download_to_unwritable_dest_maps_to_local_io_err` (now `LocalIoErr`, not `InvalidResponseErr`, after Milestone 1), `download_missing_maps_to_not_found`, `download_error_maps_to_request_failed`. (s3's `truncated_body`/`transport_failure` have no clean GCS mock analog — omit or add only if the axum mock can simulate them; comment the omission.)
   - `delete { success, access_denied }` and `exists { present, absent, access_denied }` — as today, renamed.
   - `construction { new_builds_with_valid_token, new_rejects_bad_token }` — via `Store::new(Config { … })`.
   - `error_types` — direct leaf-trait assertions; ADD a `LocalIoErr` case (→ 500, not a network error, Display contains `gs://bucket/key`) to match the new variant.
6. `multipart.rs`: do NOT create it (Decision Log). Add a one-line `//!` comment at the top of `tests/gcs/mod.rs` noting there is no multipart test module because GCS exposes no multipart surface, mirroring the pointer s3's `multipart.rs` docstring gives.

### Milestone 5 — green tests + coverage

Run `./scripts/test.sh`. Fix until the gcs suite and the whole suite pass. Then `./scripts/covgate.sh`: confirm `gcs` stays ≥ `88.00`. If `LocalIoErr` or a new branch dips coverage, add a targeted test (e.g. the `error_types::local_io_err` case and the `get::dest_unwritable` case already exercise it). Never lower the gate without a Decision Log entry.

### Milestone 6 — preflight + push

Run `./scripts/lint.sh` (import order, fmt, machete/diet, audit, clippy) and fix. Then `./scripts/preflight.sh` and confirm the final line is exactly `Preflight clean`. Only then push to the existing branch (PR #103). Move this plan file from `plans/backlog/` to `plans/completed/`.

## Concrete Steps

All commands run from the agent repo root `/home/ben/miru/workbench4/repos/agent`.

1. Edit `agent/src/gcs/errors.rs` per Milestone 1 (add `LocalIoErr`, `map_body_io_err`, rename `RequestFailedErr.key` → `object`, add omission comments, optional `FileSysErr`, mapper `body_mappers` tests).
2. Edit `agent/src/gcs/mod.rs` per Milestones 2–3 (value types, `Store` rename, constructor rename, four re-signatured ops, helper cleanup, import ordering).
3. `cargo build -p miru-agent --features test` — expect success. Fix compile errors (most will be call-site/type mismatches).
4. Edit `agent/tests/gcs/mod.rs` per Milestone 4 (imports, helpers, call sites, nested modules, `error_types` + `LocalIoErr` case).
5. `./scripts/test.sh` — expect `test result: ok.`; zero failures.
6. `./scripts/covgate.sh` — expect a line like `✅ gcs: <NN.NN>% (requires 88.00%)` and overall pass.
7. `./scripts/lint.sh` — expect clean (no new deps → machete/diet/audit unchanged).
8. `./scripts/preflight.sh` — expect final line `Preflight clean`.
9. Commit (push mode, existing branch): e.g. `refactor(agent): align gcs object-storage API with s3 module shape`. Sign per repo convention.
10. `git mv plans/backlog/20260709-gcs-s3-api-alignment.md plans/completed/` and update Progress/Outcomes.

## Validation and Acceptance

Acceptance is the offline test suite plus the coverage gate, all runnable with no network and no GCP project, PLUS an explicit preflight gate.

The refactor is behavior-preserving, so the acceptance bar is: **the same behaviors s3 documents are asserted for gcs through the new API shape.** `tests/gcs/mod.rs` must contain at least:

- `put::single::upload_streams_file_body` — `Store::new(Config{…, bucket})` against `run_server(router)`; `store.put(src.to_file(), &obj("artifacts/hello.txt"))`; assert `Ok(())` and the mock recorded an upload with an `Authorization: Bearer …` header.
- `put::source_missing::*` — missing local source → `FileSysErr` (if size-read path kept) or `LocalIoErr`; assertion must match the chosen implementation.
- `put::access_denied::upload_error_maps_to_request_failed` — mock 403 → `GcsErr::RequestFailedErr`.
- `get::success::download_streams_body_to_file` — canned bytes written to `dest`, `Bearer` header seen.
- `get::not_found::download_missing_maps_to_not_found` — 404 → `GcsErr::ObjectNotFoundErr`, `err.code()` == `Code::ResourceNotFound`, `err.http_status().as_u16()` == 404.
- `get::dest_unwritable::download_to_unwritable_dest_maps_to_local_io_err` — dest parent missing → `GcsErr::LocalIoErr` (the renamed variant; previously `InvalidResponseErr`).
- `get::access_denied::download_error_maps_to_request_failed` — non-404 (403) → `RequestFailedErr`.
- `delete::success::delete_removes_object`; `delete::success::delete_missing_is_idempotent` (NOT_FOUND → `Ok(())`); `delete::access_denied::delete_error_maps_to_request_failed`.
- `exists::present::present_returns_true`; `exists::absent::absent_returns_false`; `exists::access_denied::error_propagates`.
- `construction::new_builds_with_valid_token` (Ok); `construction::new_rejects_bad_token` (newline in token → `GcsErr::InvalidResponseErr`).
- `error_types::*` — `ObjectNotFoundErr` → `Code::ResourceNotFound` + 404; `ConnectionErr` → `is_network_conn_err()`; `RequestFailedErr` default → 500 + Display contains the operation; `InvalidResponseErr` default → 500; `LocalIoErr` default → 500, not a network error, Display contains `gs://bucket/key`.

Run and expected output:

- `./scripts/test.sh` — expect `test result: ok.` for the gcs tests and the overall suite; zero failures.
- `./scripts/covgate.sh` — the `gcs` module MUST stay at/above its gate (`88.00`); expect a line `✅ gcs: <NN.NN>% (requires 88.00%)` and a final all-modules-pass line. This is a hard gate.
- `./scripts/lint.sh` — expect no errors (import linter, fmt, machete, diet, audit, clippy).
- `./scripts/preflight.sh` — **expect the final line to be exactly `Preflight clean`. This is a hard gate: changes must NOT be published/pushed to PR #103 until `./scripts/preflight.sh` reports `clean`.**

Behavioral acceptance a human can verify: `gcs::Store` reads as a sibling of `s3::Store` (same method names/signatures, `Object`/`File` value types, `Config`/`Credentials`), a missing object still yields `Code::ResourceNotFound`, uploads/downloads still stream through bounded memory, the store is still constructible only from a caller-supplied token + bucket, and every SDK-forced deviation (no multipart, `gs://` scheme, fallible async `new`, dual-transport test seam, `map_gcs_err` absorbing the body-read role) carries a `//` comment.

## Idempotence and Recovery

- All edits are to three files (`gcs/mod.rs`, `gcs/errors.rs`, `tests/gcs/mod.rs`) plus possibly `.covgate`; re-running steps overwrites deterministically. Recovery is `git restore`/`git checkout` on the working tree.
- No new dependencies, so `Cargo.lock` should not change; if it does, run `scripts/update-deps.sh` and confirm no `cargo audit` regression.
- Tests bind ephemeral `127.0.0.1:0` ports + `tempfile`; no global state, no `#[serial]`; re-running `./scripts/test.sh` is always safe.
- If a mid-refactor build breaks because `errors.rs` references `Object`/`File` before `mod.rs` defines them, apply Milestones 1–3 together and build once.
- `.covgate` is a single-line file; adjusting it is trivially reversible and requires a Decision Log entry. Never lower below achieved coverage.
- No destructive operations, migrations, or data writes are involved.

## Interfaces and Dependencies

Already-present crate APIs the refactor relies on (no new deps):

- `google_cloud_storage::client::{Storage, StorageControl}`; `Storage::write_object(resource, key, tokio::fs::File).send_unbuffered() -> Result<Object>`; `Storage::read_object(resource, key).send()` → `ReadObjectResponse::next() -> Option<Result<bytes::Bytes>>`; `StorageControl::{delete_object, get_object}` builders `.set_bucket().set_object().send()`; `StorageControl::from_stub<T: google_cloud_storage::stub::StorageControl + 'static>(stub)`.
- `google_cloud_gax::error::Error` with `status()`, `http_status_code()`, `is_timeout()`; `google_cloud_gax::error::rpc::{Code, Status}`; `google_cloud_gax::response::Response`; `google_cloud_gax::options::RequestOptions`.
- `google_cloud_auth::credentials::{CredentialsProvider, Credentials, CacheableResource, EntityTag}` (behind the internal `StaticTokenCredentials`).
- Repo-internal: `crate::errors::{Code, HTTPCode, Trace, Error}`, `crate::impl_error!`, `crate::trace!`, `crate::filesys::{file::File, files, path::PathExt, FileSysErr}`, and (tests) `crate::mocks::http_client::run_server` / `Server { base_url }`, `mockall`.

## Artifacts and Notes

- Canonical shape references throughout: `agent/src/s3/mod.rs`, `agent/src/s3/errors.rs`, `agent/src/s3/multipart.rs`, `agent/tests/s3/mod.rs`, `agent/tests/s3/multipart.rs`, `agent/src/s3/.covgate`.
- Deferred (do NOT do here): the real-cloud GCS integration test noted in the module doc TODO and in `plans/completed/20260704-gcs-object-storage-crud.md`. Keep the TODO comment.
- Key SDK-forced deviations to comment inline: (1) no multipart surface → `put`/`put_singlepart` share one `write_object`; (2) `gs://` Display scheme; (3) fallible async `new`; (4) dual-transport test seam (HTTP endpoint + gRPC stub) vs s3's single `from_http_client`; (5) `map_gcs_err` on a concrete error type absorbing the body-read role s3 splits into `map_body_read_err`; (6) `Config.bucket` occupies `s3::Config.region`'s slot.
