# Add a `gcs` object-storage module mirroring the `s3` module

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | New `agent/src/gcs/` module (`mod.rs`, `errors.rs`, `.covgate`), new `agent/tests/gcs/mod.rs`, registrations in `agent/src/lib.rs` and `agent/tests/mod.rs`, dependency additions in `Cargo.toml` and `agent/Cargo.toml`. |
| `agent/src/s3/` | read-only | The sibling module being mirrored. Do not modify it. |
| `agent/tests/mocks/http_client.rs` | read-only | Provides the `run_server(router) -> Server { base_url }` axum mock-server helper reused by the download/upload tests. Do not modify it. |

This plan lives in `agent/plans/` because all code changes are made in the `agent` repo.

Note on git: the working branch is `feat/gcs-object-storage-crud`, which is **stacked on `feat/s3-object-storage-crud`**. The PR base for this work is `feat/s3-object-storage-crud`, NOT `main`. Do not rebase onto `main` and do not open the PR against `main`.

## Purpose / Big Picture

The agent already has an `s3` module (`agent/src/s3/`) that streams very large artifacts to and from an AWS S3 bucket using short-lived, backend-minted credentials. We now need the same capability against **Google Cloud Storage (GCS)** for devices whose artifacts live in GCS. After this change, a caller can construct a `GcsStore` from a short-lived OAuth2 access token plus a bucket name and call four operations that mirror `S3Store` exactly:

- `put_object(&self, key: &str, path: &Path)` — streams a file off disk into a GCS object (memory-bounded; never buffers the whole file).
- `get_object(&self, key: &str, dest: &Path)` — streams an object's body to a destination file.
- `delete_object(&self, key: &str)` — deletes an object.
- `object_exists(&self, key: &str) -> bool` — returns whether an object exists.

A missing object surfaces as `Code::ResourceNotFound` (HTTP 404), exactly as in `s3`. You can see it working by running the module's offline unit tests (`./scripts/test.sh`), which exercise every operation without any network access or real GCP project, and by confirming the coverage gate passes (`./scripts/covgate.sh`).

This plan delivers the **module and its offline unit tests only**. A real-cloud GCS integration test (mirroring `backend/tests/pkg/gcp/gcs/gcs_test.go`, run against a live bucket via `google-github-actions/auth` + Workload Identity Federation) is a **separate, deferred follow-up** blocked on infra (a WIF condition extension and an agent service account, being drafted as a Terraform PR). Do not implement the integration test here.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Milestone 1 — add dependencies; `cargo build` succeeds.
- [ ] Milestone 2 — implement `agent/src/gcs/errors.rs` (mirrors `s3/errors.rs`).
- [ ] Milestone 3 — implement `agent/src/gcs/mod.rs` (`GcsStore` + static-token credentials + streaming ops).
- [ ] Milestone 4 — register module in `agent/src/lib.rs`; add `agent/src/gcs/.covgate`.
- [ ] Milestone 5 — implement `agent/tests/gcs/mod.rs` offline tests; register in `agent/tests/mod.rs`.
- [ ] Milestone 6 — `./scripts/test.sh`, `./scripts/lint.sh` (cargo audit clean), `./scripts/covgate.sh` all pass.
- [ ] Milestone 7 — `./scripts/preflight.sh` prints `Preflight clean`; ready to open PR against `feat/s3-object-storage-crud`.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Use the official `google-cloud-storage` crate v1.15.0 (googleapis/google-cloud-rust), not a third-party crate.
  Rationale: Task requirement; it is the officially supported SDK and exposes both the HTTP data client (`Storage`) and the gRPC control client (`StorageControl`) needed for the four ops.
  Date/Author: 2026-07-04 / plan author.

- Decision: Authenticate with a caller-supplied static OAuth2 access token via a small custom `CredentialsProvider` impl, not ADC/metadata/env.
  Rationale: The backend mints short-lived tokens and hands them to the agent, mirroring how `s3` takes short-lived STS credentials. The crate's default credentials search ADC; a custom provider is the crate-documented way to supply your own token (see Context and Orientation).
  Date/Author: 2026-07-04 / plan author.

- Decision: Test the HTTP data path (upload/download) against the existing axum mock server via `.with_endpoint(...)`, and test the gRPC control path (delete/exists) via the crate's `from_stub` seam with a `mockall` mock of `gcs::stub::StorageControl` — no real gRPC server.
  Rationale: Both are fully offline. The crate's own tests do exactly this (see Context and Orientation for file/line evidence). This avoids the fragility of standing up a tonic server and needing proto stubs the crate does not export for reuse.
  Date/Author: 2026-07-04 / plan author.

- Decision: Set the gcs `.covgate` threshold to `85.00` (below s3's `88.00`).
  Rationale: The gRPC delete/exists paths cannot be exercised as thoroughly offline as the s3 replay-client path (the control `send()` layer wraps the stub and some glue is not reachable through the stub seam). 85 is achievable while still forcing real coverage of the credentials provider, both streaming paths, error mapping, and not-found classification. Revisit upward once the deferred real-cloud integration test lands. See Validation for the exact figure to confirm after implementation and adjust if the measured number is higher.
  Date/Author: 2026-07-04 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

This section gives a novice everything needed to implement the module without prior repo knowledge. Read it fully before starting.

### The module to mirror: `agent/src/s3/`

`agent/src/s3/mod.rs` defines `S3Store`, a struct holding an `aws_sdk_s3::Client` and a `bucket: String`. Its public shape (mirror it):

- `pub fn new(creds, region, bucket) -> Self` — builds the client from caller-supplied temporary credentials; does no network I/O.
- `#[cfg(feature = "test")] pub fn with_http_client(...)` — a test-only constructor injecting a mock HTTP client.
- `#[cfg(feature = "test")] pub fn set_single_put_threshold(&mut self, bytes: u64)` — a test seam to force the multipart/streaming path with a small file.
- `pub async fn put_object(&self, key: &str, path: &Path) -> Result<(), S3Err>`
- `pub async fn get_object(&self, key: &str, dest: &Path) -> Result<(), S3Err>`
- `pub async fn delete_object(&self, key: &str) -> Result<(), S3Err>`
- `pub async fn object_exists(&self, key: &str) -> Result<bool, S3Err>`

`agent/src/s3/errors.rs` defines the error taxonomy (mirror it, renaming `S3` → `Gcs`):

- Leaf structs `ObjectNotFoundErr` (→ `Code::ResourceNotFound`, HTTP 404), `ConnectionErr` (`is_network_conn_err()` → true), `RequestFailedErr` (custom `Display`; default code `InternalServerError`/500), `InvalidResponseErr` (default 500). Each derives `thiserror::Error` and implements `crate::errors::Error`.
- Enum `S3Err` aggregating the four leaves, wired via `crate::impl_error!`.
- A `map_sdk_err_common` helper. For GCS this becomes a `map_gcs_err` that maps the single crate error type (`google_cloud_gax::error::Error`) rather than a generic `SdkError<E>` (see below), so the signature differs from s3's.

`agent/src/s3/.covgate` contains the single line `88.00` — the minimum per-module coverage percentage.

`agent/tests/s3/mod.rs` holds offline tests using `aws_smithy_http_client::test_util::StaticReplayClient`. The gcs test file uses different mechanisms (endpoint override + stub) but the same structure: nested `pub mod` groups per operation, `#[tokio::test]` async tests, `temp_file_with(&[u8]) -> NamedTempFile` helper, and direct assertions on the leaf error types.

### Repo conventions you must follow

Read `agent/AGENTS.md`. The key rules:

1. **Import ordering** in every source file: three groups separated by a blank line and a comment — `// standard crates`, then `// internal crates`, then `// external crates`. There is a custom import linter (run in `scripts/lint.sh`) that enforces this.
2. **Error handling**: leaf errors derive `thiserror::Error` and implement `crate::errors::Error` (trait defined in `agent/src/errors/`). Aggregating enums use the `crate::impl_error!` macro. The `crate::trace!()` macro produces the `Box<Trace>` each leaf carries.
3. **Feature flags**: `#[cfg(feature = "test")]` gates test-only code (constructors/seams). The `test` feature is declared in `agent/Cargo.toml` under `[features]`.
4. **Adding a module** (from AGENTS.md "Adding a new module"): create `agent/src/gcs/mod.rs` (+ `errors.rs`), add `pub mod gcs;` to `agent/src/lib.rs`, create `agent/tests/gcs/mod.rs`, add a `.covgate` file, and register the test module in `agent/tests/mod.rs` (the integration-test entry point lists `pub mod <name>;` for each test dir).
5. **Tests require `--features test`.** Always run via `./scripts/test.sh` (it runs `RUST_LOG=off cargo test --features test`). Tests that bind shared OS resources use `#[serial]`; our gcs tests bind only ephemeral `127.0.0.1:0` ports and temp files, so they do NOT need `#[serial]`.

### The crate: `google-cloud-storage` v1.15.0 (with `google-cloud-auth` v1.13.0)

The crate exposes two clients (both re-exported under `google_cloud_storage::client`):

- `Storage` — the **object data** client. Its `write_object` and `read_object` operations go over **HTTP (JSON/reqwest)**. This is what we use for `put_object` and `get_object`.
- `StorageControl` — the **control-plane** client. Its `delete_object` and `get_object` (metadata) operations go over **gRPC (tonic)**. This is what we use for `delete_object` and `object_exists`.

Bucket naming: both clients take the bucket as a **resource path** in the form `projects/_/buckets/<bucket_id>`, and the object as the bare key. `GcsStore` stores the full resource path (`format!("projects/_/buckets/{bucket}")`) once at construction.

#### Problem 1 — Token-based client construction (verified)

Default credentials search ADC. To supply a pre-obtained OAuth2 access token, implement the crate's **public** `CredentialsProvider` trait and wrap it with `Credentials::from(...)`. This is the crate-documented path (`google-cloud-auth-1.13.0/src/credentials.rs`: "Application developers who directly use the Auth SDK can use this trait, along with `Credentials::from()` to mock the credentials.").

Exact types/paths (all public):

- Trait: `google_cloud_auth::credentials::CredentialsProvider` with two methods:
  - `fn headers(&self, extensions: http::Extensions) -> impl Future<Output = google_cloud_auth::credentials::Result<google_cloud_auth::credentials::CacheableResource<http::HeaderMap>>> + Send`
  - `fn universe_domain(&self) -> impl Future<Output = Option<String>> + Send`
- `google_cloud_auth::credentials::CacheableResource<T>` — enum with `NotModified` and `New { entity_tag: EntityTag, data: T }`.
- `google_cloud_auth::credentials::EntityTag` — `EntityTag::new()` mints a fresh opaque tag.
- `google_cloud_auth::credentials::Credentials` — has `impl<T: CredentialsProvider + Send + Sync + 'static> From<T> for Credentials`, so `Credentials::from(provider)` works.
- The header to emit is `Authorization: Bearer <token>` (confirmed in `google-cloud-auth-1.13.0/src/headers_util.rs`: `HeaderType::Bearer => AUTHORIZATION`).

The provider we implement (call it `StaticTokenCredentials`, private to `gcs/mod.rs`), templated on the crate's own `AnonymousCredentials` (`google-cloud-auth-1.13.0/src/credentials/anonymous.rs`):

    #[derive(Debug)]
    struct StaticTokenCredentials {
        header_value: http::HeaderValue, // pre-built "Bearer <token>"
        entity_tag: EntityTag,
    }

    impl CredentialsProvider for StaticTokenCredentials {
        async fn headers(
            &self,
            extensions: http::Extensions,
        ) -> google_cloud_auth::credentials::Result<CacheableResource<http::HeaderMap>> {
            match extensions.get::<EntityTag>() {
                Some(tag) if self.entity_tag.eq(tag) => Ok(CacheableResource::NotModified),
                _ => {
                    let mut headers = http::HeaderMap::new();
                    headers.insert(http::header::AUTHORIZATION, self.header_value.clone());
                    Ok(CacheableResource::New {
                        data: headers,
                        entity_tag: self.entity_tag.clone(),
                    })
                }
            }
        }

        async fn universe_domain(&self) -> Option<String> {
            None
        }
    }

Note: the public trait uses `async fn` sugar for `impl Future`; write the impl with `async fn` as above (the crate's own `anonymous.rs` uses `#[async_trait]` because it implements the crate-private `dynamic` trait; external code implements the public trait with plain `async fn` and the crate's blanket impl bridges it). If the token contains a byte that is invalid in an HTTP header value, `HeaderValue::from_str(&format!("Bearer {token}"))` returns an error at construction time — surface it as `GcsErr::InvalidResponseErr` from the constructor. Because of this, the production constructor `GcsStore::new` returns `Result<Self, GcsErr>` (unlike s3's infallible `new`).

Client construction with the token and optional endpoint override:

    let credentials = Credentials::from(StaticTokenCredentials { header_value, entity_tag });
    let mut data_builder = Storage::builder().with_credentials(credentials.clone());
    let mut control_builder = StorageControl::builder().with_credentials(credentials);
    if let Some(ep) = endpoint {           // test override, e.g. "http://127.0.0.1:PORT"
        data_builder = data_builder.with_endpoint(ep.clone());
        control_builder = control_builder.with_endpoint(ep);
    }
    let data = data_builder.build().await?;
    let control = control_builder.build().await?;

Both builders expose `with_credentials<V: Into<Credentials>>` and `with_endpoint<V: Into<String>>` (`google-cloud-storage-1.15.0/src/storage/client.rs:392,440` and `.../src/control/client.rs`). `.build()` is async and returns `google_cloud_gax::client_builder::Result<_>`; map its error to `GcsErr::InvalidResponseErr`. Building does perform some async setup but no real GCS call — safe to run in a `#[tokio::test]`.

#### Streaming ops (verified)

Upload (`put_object`) — HTTP, streams off disk, resumable automatically, no whole-file buffering:

    let file = tokio::fs::File::open(path).await?;          // map io::Error -> InvalidResponseErr
    self.data
        .write_object(&self.bucket_resource, key, file)
        .send_unbuffered()
        .await
        .map_err(|e| map_gcs_err("put_object", Some(key), e))?;

`impl From<tokio::fs::File> for Payload<FileSource>` exists (`.../src/storage/streaming_source.rs:162`), so passing a `tokio::fs::File` streams it in 256 KiB chunks with `Seek` support (resumable). `send_unbuffered(self) -> Result<Object>` is at `.../src/storage/write_object.rs:1021`. `unstable-stream` is NOT needed.

Download (`get_object`) — HTTP, streams to disk chunk-by-chunk:

    let mut resp = self.data.read_object(&self.bucket_resource, key).send().await
        .map_err(classify_read_err)?;   // NOT_FOUND -> ObjectNotFoundErr, else map_gcs_err
    let mut dest_file = tokio::fs::File::create(dest).await?;   // io -> InvalidResponseErr
    use tokio::io::AsyncWriteExt as _;
    while let Some(chunk) = resp.next().await {
        let bytes = chunk.map_err(|e| map_gcs_err("get_object", Some(key), e))?;
        dest_file.write_all(&bytes).await?;   // io -> InvalidResponseErr
    }
    dest_file.flush().await?;

`ReadObject::send(self) -> Result<ReadObjectResponse>` is at `.../src/storage/read_object.rs:490`. `ReadObjectResponse::next(&mut self) -> Option<Result<bytes::Bytes>>` is at `.../src/read_object.rs:110`.

Delete (`delete_object`) — gRPC control client:

    self.control.delete_object().set_bucket(&self.bucket_resource).set_object(key).send().await
        .map_err(|e| map_gcs_err("delete_object", Some(key), e))?;

`StorageControl::delete_object()` returns a `DeleteObject` builder with `.set_bucket(...).set_object(...).send() -> Result<()>` (`.../src/generated/gapic/builder.rs:946-1010`). GCS delete of a missing object returns a NOT_FOUND error (unlike S3's idempotent delete). Mirror s3's contract: treat NOT_FOUND from delete as **success** (idempotent) — classify it the same way as `object_exists` below and return `Ok(())` on NotFound.

Exists (`object_exists`) — gRPC control client, get-object-metadata:

    match self.control.get_object().set_bucket(&self.bucket_resource).set_object(key).send().await {
        Ok(_) => Ok(true),
        Err(e) if is_not_found(&e) => Ok(false),
        Err(e) => Err(map_gcs_err("get_object", Some(key), e)),
    }

`StorageControl::get_object()` returns a `GetObject` builder → `.send() -> Result<Object>` (`.../src/generated/gapic/client.rs:431`, builder in `builder.rs`).

#### Error classification (verified)

The single crate error type is `google_cloud_gax::error::Error` (re-exported as `google_cloud_storage::Error`). Introspection methods (confirmed via docs.rs):

- `err.status() -> Option<&google_cloud_gax::error::rpc::Status>`; `Status` has a `code` field of type `google_cloud_gax::error::rpc::Code`. `Code::NotFound` (value 5) is the not-found code.
- `err.http_status_code() -> Option<u16>` — fallback for HTTP-path errors (the data client is HTTP; a 404 there surfaces here).
- `err.is_timeout() -> bool` — maps to `ConnectionErr` (network).

Define a helper `is_not_found(err) -> bool` = `err.status().map(|s| s.code == Code::NotFound).unwrap_or(false) || err.http_status_code() == Some(404)`.

Define `map_gcs_err(operation: &str, key: Option<&str>, err: google_cloud_gax::error::Error) -> GcsErr`:

- `err.is_timeout()` → `GcsErr::ConnectionErr` (`is_network_conn_err()` true).
- else `GcsErr::RequestFailedErr { operation, key, status: err.http_status_code(), msg: err.to_string(), trace }`.

Not-found is classified by the caller (get/delete/exists) BEFORE calling `map_gcs_err`, exactly like s3 classifies `NoSuchKey`/404 before delegating to `map_sdk_err_common`.

#### Problem 2 — Offline testing (the critical risk; RESOLVED with evidence)

There are two transports; both are tested fully offline:

HTTP data path (upload/download) — point the `Storage` client at a local axum server. The crate's own transport tests do exactly this: `.../src/storage/transport.rs` builds the client with `.with_endpoint(format!("http://{}", server.addr()))` against an `httptest::Server` and asserts the transport span is `"http"` (lines 361, 405, 446, 482, 463, 499). We reuse the agent's existing helper `crate::mocks::run_server(router) -> Server { base_url }` (in `agent/tests/mocks/http_client.rs`), which binds `127.0.0.1:0` and returns `base_url` like `http://127.0.0.1:PORT`. Build the `GcsStore` with `endpoint = Some(server.base_url)` and route the GCS JSON upload/download paths with an axum `Router`.

  - Upload lands as `POST` to `/upload/storage/v1/b/<bucket>/o` (JSON API multipart/simple upload) with `uploadType` query, or resumable `PUT`s; download lands as `GET` on the object media URL. Rather than hand-writing exact GCS URL matchers (brittle), use a **permissive fallback router** that returns a canned success JSON `Object` for any upload path and canned bytes for the download path, and assert on observable outcomes (upload returns `Ok`; download writes the expected bytes to `dest`). This mirrors how s3's `put_streams_file_body_bytes` asserts method+path rather than byte-comparing the streamed body. Keep matchers as loose as needed to be robust: match on method + a path prefix. Add a catch-all `.fallback(...)` handler that records the request and returns 200 with a minimal JSON object body so a single test failure surfaces as a clear assertion rather than a routing panic.
  - For the not-found download test, return HTTP 404 with a GCS JSON error body; assert the mapped error is `GcsErr::ObjectNotFoundErr` and `err.code()` is `Code::ResourceNotFound`.
  - For a transport-failure test, build the store pointed at an endpoint with no server (or shut the server down) and assert `GcsErr::ConnectionErr` / `RequestFailedErr`.

  IMPORTANT caveat to resolve during implementation: the GCS `Storage` HTTP client may compute checksums (CRC32C) and expect specific response JSON. If the permissive mock's canned `Object` response does not satisfy the client's parsing/finalization, the upload test may fail. If that happens, the fallback is to (a) capture a real GCS success-response JSON shape from the crate's own upload tests under `.../src/storage/perform_upload/` fixtures and return that exact JSON, or (b) reduce the upload assertion to "the client issued the expected POST/PUT to the mock" (assert on the recorded request in the fallback handler) and accept that the `Ok(Object)` decode is covered indirectly. Prefer (a). This is the single most likely implementation snag; budget time for it and record findings in Surprises & Discoveries.

gRPC control path (delete/exists) — use the crate's `from_stub` seam with a `mockall` mock. The crate's own `tests/mocking.rs` does exactly this (lines 240-290): `mockall::mock! { StorageControl {} impl gcs::stub::StorageControl for StorageControl { async fn delete_object(...); async fn get_object(...); /* ...all 33 methods... */ } }`, then `let client = gcs::client::StorageControl::from_stub(mock);` and `mock.expect_delete_object().returning(|_,_| Err(Error::service(Status::default().set_code(Code::Aborted))))`. No network, no tonic server, no proto plumbing.

  - `gcs::client::StorageControl::from_stub<T>(stub) -> StorageControl` where `T: gcs::stub::StorageControl + 'static` (`.../src/control/generated/client.rs:546`) returns a **concrete** `StorageControl` (the stub is boxed internally) — so `GcsStore` can hold a concrete `StorageControl` field with no generics.
  - `gcs::stub::StorageControl` is the public trait (`google_cloud_storage::stub::StorageControl`, re-exported at `.../src/lib.rs:120`). Its `delete_object`/`get_object` signatures:
    - `async fn delete_object(&self, _req: gcs::model::DeleteObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<()>>`
    - `async fn get_object(&self, _req: gcs::model::GetObjectRequest, _options: google_cloud_gax::options::RequestOptions) -> google_cloud_gax::Result<google_cloud_gax::response::Response<gcs::model::Object>>`
  - The `mockall::mock!` block MUST list ALL 33 trait methods (mockall requires the full trait surface). Copy the exact list from `google-cloud-storage-1.15.0/tests/mocking.rs` lines 240-289 (reproduced in Interfaces and Dependencies below). Only `delete_object` and `get_object` need `.expect_*` setups; the rest are never called.
  - Building canned errors: `use google_cloud_gax::error::{Error, rpc::{Code, Status}};` then `Error::service(Status::default().set_code(Code::NotFound))` for not-found and `Code::PermissionDenied`/`Code::Aborted` for a generic failure. Building a canned success: `Ok(google_cloud_gax::response::Response::from(gcs::model::Object::default()))` (or the crate's `Response::new(...)`; confirm the exact constructor during implementation — the crate's tests use `Response::new(...)` in `quota_project_control.rs`).

  How the tests reach `GcsStore`: because `object_exists`/`delete_object` call `self.control`, provide a `#[cfg(feature = "test")]` constructor `GcsStore::with_control_stub(stub, bucket)` that builds `GcsStore` with `control = StorageControl::from_stub(stub)` and a `data` client pointed at an unused endpoint (never called by these two ops). Then delete/exists tests inject a `MockStorageControl` and assert behavior. This mirrors s3's `with_http_client` test constructor.

Net offline coverage achieved: custom credentials provider (`headers` both branches + `universe_domain`), both streaming ops (upload success + download success/not-found/io-error), delete (success + idempotent-not-found + error), exists (true/false/error), the full `map_gcs_err` branch set, and direct leaf-error trait assertions. This is realistic against an **85.00** covgate (see Decision Log).

#### Problem 3 — Deps / TLS / cargo audit (verified clean)

`scripts/lint.sh` runs `cargo audit` (via `rustsec/audit-check` in CI, and `cargo audit` locally) and it rejects known-vulnerable crates. The s3 PR had to avoid `rustls-webpki` 0.101.x/0.102.x (RUSTSEC-2026-0098/0099). Evidence that `google-cloud-storage` v1.15.0 does NOT reintroduce a vulnerable webpki:

- The default feature `default-rustls-provider` pulls `google-cloud-auth` with `rustls = 0.23.38` + `reqwest 0.13.4` (`rustls-no-provider`) + `rustls-pki-types 1.14` and the `aws-lc-rs` crypto provider (`google-cloud-auth-1.13.0/Cargo.toml`).
- rustls 0.23.x resolves `rustls-webpki` to the **0.103.x** line — the FIXED line. The crate's vendored `Cargo.lock` pins `rustls-webpki 0.103.13`, `rustls 0.23.40`, `hyper-rustls 0.27.9`, `tonic 0.14.6`, `aws-lc-rs 1.17.0`, `ring 0.17.14`. None of these are covered by RUSTSEC-2026-0098/0099 (which are 0.101/0.102-only) and `ring 0.17.14` has no open advisory.
- This is the SAME modern stack the workspace `Cargo.toml` comment already prefers for aws-sdk-s3 ("rustls 0.23 + hyper-rustls 0.27 → rustls-webpki 0.103.x, unaffected").

Feature flags to add (keep audit clean, avoid unneeded surface):

- In `[workspace.dependencies]` of the root `Cargo.toml`:
      google-cloud-storage = { version = "1.15.0" }
      google-cloud-auth = { version = "1.13.0" }
      google-cloud-gax = { version = "1.11.0" }
  Use default features (which is `default-rustls-provider`). Do NOT disable defaults — disabling would drop the rustls provider and require installing a crypto provider manually. `google-cloud-gax` is needed directly for the error/`RequestOptions`/`Response`/`Code`/`Status` types used in mapping and tests.
- `google-cloud-auth` is a direct dependency because the credentials provider (`CredentialsProvider`, `Credentials`, `CacheableResource`, `EntityTag`) lives there.
- Dev-dependency `mockall = "0.14"` (matches the crate's own dev-dep version) for the control-stub mock.

Watch items during implementation (record in Surprises & Discoveries):

- After adding deps, run `scripts/update-deps.sh` (refreshes `Cargo.lock`) then `cargo audit`. If the workspace resolver picks an OLDER `rustls-webpki` (e.g. because some other workspace crate constrains it downward), pin `rustls-webpki` to `>=0.103.x` via a workspace constraint or a direct dev/build note. Expected outcome: no new advisory. If `cargo audit` flags anything, resolve by bumping the offending transitive crate, mirroring the pattern already documented in the root `Cargo.toml` comments.
- `cargo machete` / `cargo diet` (run by `scripts/lint.sh`) flag unused deps. Ensure every added crate is actually referenced in `agent/src/gcs/` or `agent/tests/gcs/`, or machete will fail. `google-cloud-gax` is referenced in both src (error mapping) and tests (canned errors); `google-cloud-auth` in src (provider); `google-cloud-storage` in both; `mockall` in tests only.

## Plan of Work

Work proceeds in seven milestones; commit after each.

Milestone 1 — Dependencies. Add the three `google-cloud-*` crates to `[workspace.dependencies]` in `/home/ben/miru/workbench5/repos/agent/Cargo.toml`, add them (via `{ workspace = true }`) to `[dependencies]` in `agent/Cargo.toml`, and add `mockall = "0.14"` to `[dev-dependencies]` in `agent/Cargo.toml`. Run `scripts/update-deps.sh` then `cargo build -p miru-agent`. Confirm no `cargo audit` regression.

Milestone 2 — `agent/src/gcs/errors.rs`. Mirror `agent/src/s3/errors.rs` renaming `S3` → `Gcs` throughout: leaf structs `ObjectNotFoundErr`, `ConnectionErr`, `RequestFailedErr` (keep the custom `Display` printing `GCS <operation> request ...`), `InvalidResponseErr`; enum `GcsErr` with `crate::impl_error!(GcsErr { ... })`. Replace `map_sdk_err_common<E>(...SdkError<E>)` with `pub fn map_gcs_err(operation: &str, key: Option<&str>, err: google_cloud_gax::error::Error) -> GcsErr` and add `pub fn is_not_found(err: &google_cloud_gax::error::Error) -> bool` as described in Context. Keep the `#[cfg(test)] mod tests` unit tests for the mapper (adapt the two s3 mapper tests to construct `google_cloud_gax::error::Error` values — a timeout error and a service error — and assert the mapped variant).

Milestone 3 — `agent/src/gcs/mod.rs`. Module doc comment mirroring s3's (explain: talks to GCS over the network; distinct from `crate::storage`; constructed only from a caller-supplied short-lived OAuth2 access token + bucket; never reads ADC/metadata/env; bodies streamed to/from disk). Define the private `StaticTokenCredentials` provider (Context, Problem 1). Define `pub struct GcsStore { data: Storage, control: StorageControl, bucket_resource: String }`. Implement: `pub async fn new(access_token: String, bucket: String, endpoint: Option<String>) -> Result<Self, GcsErr>`; `#[cfg(feature = "test")] pub fn with_control_stub<T: gcs::stub::StorageControl + 'static>(stub: T, bucket: String, endpoint: String) -> ... ` (builds `data` via endpoint override and `control` via `from_stub`; note this constructor is async because the data builder is async — make it `async` and return `Result<Self, GcsErr>`); the four ops; the `map_gcs_err`/`is_not_found` usage. Follow import ordering.

Milestone 4 — Register + covgate. Add `pub mod gcs;` to `agent/src/lib.rs` (alphabetical position: between `events`/`filesys`... actually between `filesys` and `http`; place it to keep the list sorted — after `filesys`, before `http`). Create `agent/src/gcs/.covgate` containing exactly `85.00`.

Milestone 5 — Tests. Create `agent/tests/gcs/mod.rs` with the nested-module structure (see Validation for the full test list). Add `pub mod gcs;` to `agent/tests/mod.rs` (sorted: after `filesys`, before `http`). Use `crate::mocks::run_server` for HTTP tests and `mockall` for control tests.

Milestone 6 — Green. Run `./scripts/test.sh`, `./scripts/lint.sh`, `./scripts/covgate.sh`; fix until all pass. Adjust `.covgate` only if the measured gcs coverage is comfortably higher than 85 (raise it) — never lower it below what the tests achieve without justification in the Decision Log.

Milestone 7 — Preflight + PR. Run `./scripts/preflight.sh` and confirm it prints `Preflight clean`. Do not open the PR until it does. PR base = `feat/s3-object-storage-crud`.

## Concrete Steps

All commands run from the agent repo root `/home/ben/miru/workbench5/repos/agent` unless stated.

Milestone 1 — dependencies:

1. Edit `Cargo.toml` `[workspace.dependencies]`: add

       google-cloud-storage = { version = "1.15.0" }
       google-cloud-auth = { version = "1.13.0" }
       google-cloud-gax = { version = "1.11.0" }

2. Edit `agent/Cargo.toml` `[dependencies]`: add `google-cloud-storage = { workspace = true }`, `google-cloud-auth = { workspace = true }`, `google-cloud-gax = { workspace = true }`. In `[dev-dependencies]` add `mockall = "0.14"`.
3. Run `./scripts/update-deps.sh` (refreshes `Cargo.lock`).
4. Run `cargo build -p miru-agent`. Expect a successful build (first build downloads/compiles the google-cloud crates; may take several minutes).
5. Run `cargo audit` and confirm no new advisory (expect the same clean result as before; `rustls-webpki` should resolve to 0.103.x). Transcript to expect: `Success No vulnerable packages found` (or the pre-existing baseline with zero new entries).
6. Commit: from `agent/`, `git add -A && git commit` with message `build(agent): add google-cloud-storage and auth deps for gcs module` (sign per repo convention).

Milestone 2 — errors:

7. Create `agent/src/gcs/errors.rs` per Plan of Work.
8. `cargo build -p miru-agent` — expect success (errors.rs compiles once `mod.rs` exists; if referencing `gcs::errors` before `mod.rs` is registered, temporarily add `pub mod gcs;` with a stub or do Milestones 2-4 together then build). Practical approach: write `mod.rs` + `errors.rs` + lib registration together, then build once.
9. Commit: `feat(agent): add gcs error taxonomy mirroring s3`.

Milestone 3-4 — module + registration + covgate:

10. Create `agent/src/gcs/mod.rs`; add `pub mod errors;` and `pub use errors::GcsErr;` at the top of it (mirror s3's re-exports: s3 does `pub mod errors; pub use errors::S3Err;`).
11. Add `pub mod gcs;` to `agent/src/lib.rs` in sorted position.
12. Create `agent/src/gcs/.covgate` with content `85.00`.
13. Run `cargo build -p miru-agent --features test` — expect success (this compiles the `#[cfg(feature="test")]` constructor too).
14. Commit: `feat(agent): add gcs object-storage streaming CRUD module`.

Milestone 5 — tests:

15. Create `agent/tests/gcs/mod.rs`; add `pub mod gcs;` to `agent/tests/mod.rs` in sorted position.
16. Run `./scripts/test.sh`. Expect all gcs tests to pass along with the rest of the suite. If the upload HTTP test fails on response decoding, apply the Problem 2 fallback (return a real GCS `Object` JSON shape) and re-run.
17. Commit: `test(agent): offline gcs CRUD tests via endpoint override and control stub`.

Milestone 6 — green gates:

18. Run `./scripts/lint.sh`. Fix import-order, clippy, machete/diet, and audit findings until clean.
19. Run `./scripts/covgate.sh`. Expect a line like `✅ gcs: <NN.NN>% (requires 85.00%)`. If gcs is below 85, add targeted tests (or, if genuinely unreachable offline, lower the gate with a Decision Log entry — but exhaust test additions first). If comfortably above (e.g. ≥88), raise `.covgate` to the achieved floor rounded down and re-run.
20. Commit any test/threshold adjustments: `test(agent): tune gcs coverage to meet gate`.

Milestone 7 — preflight:

21. Run `./scripts/preflight.sh`. Expect the final line `Preflight clean`. This runs lint + covgate + tools lint + tools tests in parallel; all four must pass.
22. If `Preflight clean` prints, the branch is ready. Do NOT open the PR against `main`; the PR base is `feat/s3-object-storage-crud`.

## Validation and Acceptance

Primary acceptance is the offline unit test suite plus the coverage gate, all runnable with no network and no GCP project.

Test file `agent/tests/gcs/mod.rs` must contain at least these `#[tokio::test]` (async) / `#[test]` cases, grouped in nested `pub mod`s mirroring `agent/tests/s3/mod.rs`:

- `put::upload_streams_file_body` — build `GcsStore` against `crate::mocks::run_server(router)`; call `put_object("artifacts/hello.txt", src)` where `src` is a temp file; assert `Ok(())` and (in the mock's recorded requests) that an upload request was issued.
- `get::download_streams_body_to_file` — mock returns canned bytes; assert `dest` file contents equal the payload.
- `get::download_missing_maps_to_not_found` — mock returns 404 JSON; assert `GcsErr::ObjectNotFoundErr`, `err.code()` == `Code::ResourceNotFound`, `err.http_status().as_u16()` == 404.
- `get::download_to_unwritable_dest_maps_to_invalid_response` — dest parent dir missing; assert `GcsErr::InvalidResponseErr`.
- `delete::delete_removes_object` — inject `MockStorageControl` with `expect_delete_object().returning(|_,_| Ok(Response::from(())))`; assert `Ok(())`.
- `delete::delete_missing_is_idempotent` — mock returns `Err(Error::service(Status::default().set_code(Code::NotFound)))`; assert `Ok(())` (idempotent).
- `delete::delete_error_maps_to_request_failed` — mock returns a `PermissionDenied` service error; assert `GcsErr::RequestFailedErr`.
- `exists::present_returns_true` — mock `get_object` returns `Ok(Response::from(Object::default()))`; assert `true`.
- `exists::absent_returns_false` — mock `get_object` returns `NotFound`; assert `false`.
- `exists::error_propagates` — mock `get_object` returns `PermissionDenied`; assert `Err(GcsErr::RequestFailedErr)`.
- `construction::new_builds_and_rejects_bad_token` — `GcsStore::new` with a valid token + bucket + `None` endpoint returns `Ok`; with a token containing a newline returns `Err(GcsErr::InvalidResponseErr)`.
- `credentials::provider_emits_bearer_header` — construct `StaticTokenCredentials` (via a small `#[cfg(feature="test")]` accessor or by testing through the public path) and assert the emitted `HeaderMap` contains `authorization: Bearer <token>` on the `New` branch and `NotModified` when the same `EntityTag` is presented. (If `StaticTokenCredentials` stays private, cover it indirectly by asserting the mock HTTP server received an `Authorization: Bearer ...` header on upload/download — the loose router can record and assert this.)
- `error_types::*` — direct trait assertions on each leaf, mirroring s3's `error_types` module: `ObjectNotFoundErr` → `Code::ResourceNotFound` + 404; `ConnectionErr` → `is_network_conn_err()`; `RequestFailedErr` default → 500 + Display contains operation; `InvalidResponseErr` default → 500.

Run and expected output:

- `./scripts/test.sh` — expect `test result: ok.` for the gcs tests and the overall suite; zero failures.
- `./scripts/covgate.sh` — expect a line `✅ gcs: <NN.NN>% (requires 85.00%)` and a final `✅ All modules meet minimum coverage requirement`. Every new test must contribute: verify a not-found download test fails (wrong error variant) if the classification code is removed, and passes with it — this demonstrates the test exercises the behavior, not just the happy path.
- `./scripts/lint.sh` — expect it to finish with no errors (import linter, fmt, machete, diet, audit, clippy all clean).
- `./scripts/preflight.sh` — expect the final line to be exactly `Preflight clean`. **This is a hard gate: the PR must not be opened until `./scripts/preflight.sh` prints "Preflight clean".**

Behavioral acceptance summary a human can verify: after implementation, `GcsStore` compiles, the four operations behave as specified against mocks, a missing object yields `Code::ResourceNotFound`, uploads/downloads stream through bounded memory, and no `cargo audit` advisory is introduced.

## Idempotence and Recovery

- Editing `Cargo.toml`/`agent/Cargo.toml` and running `scripts/update-deps.sh` is idempotent; re-running regenerates `Cargo.lock` deterministically. If a resolution picks a vulnerable transitive crate, add a workspace constraint (mirroring existing `Cargo.toml` comments) and re-run; revert the constraint if a later crate bump makes it unnecessary.
- Creating new files (`agent/src/gcs/*`, `agent/tests/gcs/mod.rs`) is safe to repeat; overwrite on retry. The two registration edits (`lib.rs`, `tests/mod.rs`) add one line each — if run twice, remove the duplicate line.
- All tests bind ephemeral `127.0.0.1:0` ports and use `tempfile::NamedTempFile`; they leave no global state and need no `#[serial]`, so re-running `./scripts/test.sh` is always safe.
- The `.covgate` threshold is a single-line file; adjusting it is trivially reversible. Never lower it below the achieved coverage without a Decision Log entry.
- If `cargo build` fails midway (e.g. `errors.rs` referenced before `mod.rs`/lib registration exists), complete Milestones 2-4 together and build once — the recovery is to add all three files plus the `pub mod gcs;` line before building.
- No destructive operations, migrations, or data writes are involved; recovery is `git restore`/`git checkout` on the working tree.

## Interfaces and Dependencies

Types/functions that must exist and their exact paths (all verified against the crate source in the cargo registry and docs.rs):

- `google_cloud_storage::client::Storage`, `::StorageControl`; builders via `Storage::builder()` / `StorageControl::builder()` with `.with_credentials(...)`, `.with_endpoint(...)`, async `.build()`.
- `google_cloud_storage::client::StorageControl::from_stub<T: google_cloud_storage::stub::StorageControl + 'static>(stub) -> StorageControl`.
- `google_cloud_storage::stub::StorageControl` — trait to mock; 33 methods (full list to copy into `mockall::mock!` from `google-cloud-storage-1.15.0/tests/mocking.rs` lines 240-289): delete_bucket, get_bucket, create_bucket, list_buckets, lock_bucket_retention_policy, get_iam_policy, set_iam_policy, test_iam_permissions, update_bucket, compose_object, **delete_object**, restore_object, **get_object**, update_object, list_objects, rewrite_object, move_object, create_folder, delete_folder, get_folder, list_folders, rename_folder, get_storage_layout, create_managed_folder, delete_managed_folder, get_managed_folder, list_managed_folders, create_anywhere_cache, update_anywhere_cache, disable_anywhere_cache, pause_anywhere_cache, resume_anywhere_cache, get_anywhere_cache, list_anywhere_caches, get_folder_intelligence_config, update_folder_intelligence_config, get_project_intelligence_config, update_project_intelligence_config, get_organization_intelligence_config, update_organization_intelligence_config, get_operation. (Count the actual list in the crate; it is the authoritative source — copy verbatim.)
- `google_cloud_storage::model::{DeleteObjectRequest, GetObjectRequest, Object}`.
- `google_cloud_storage::streaming_source::Payload` with `impl From<tokio::fs::File>`.
- `google_cloud_storage::read_object::ReadObjectResponse` with `async fn next(&mut self) -> Option<Result<bytes::Bytes>>`.
- `google_cloud_gax::error::Error` (`= google_cloud_storage::Error`); `Error::service(Status)`, `err.status() -> Option<&Status>`, `err.http_status_code() -> Option<u16>`, `err.is_timeout() -> bool`.
- `google_cloud_gax::error::rpc::{Code, Status}` with `Code::NotFound`, `Status::default().set_code(...)`.
- `RequestOptions` — the crate's own `tests/mocking.rs` imports it as `google_cloud_storage::request_options::RequestOptions` (alias of `google_cloud_gax::options::RequestOptions`); use the `gcs::request_options::RequestOptions` form inside the `mockall::mock!` block to match the crate's working example exactly. `google_cloud_gax::response::Response` (aliased `google_cloud_gax::Result`) for the stub return types.
- `google_cloud_auth::credentials::{CredentialsProvider, Credentials, CacheableResource, EntityTag}`; header emitted is `Authorization: Bearer <token>`.
- Repo-internal: `crate::errors::{Code, HTTPCode, Trace, Error}`, `crate::impl_error!`, `crate::trace!`, and (tests) `crate::mocks::run_server` / `Server { base_url }`.

## Artifacts and Notes

- Deferred follow-up (do NOT do here): real-cloud GCS integration test mirroring `backend/tests/pkg/gcp/gcs/gcs_test.go` (`RunStorageTests` against a live bucket, CI auth via `google-github-actions/auth` + Workload Identity Federation). Blocked on a Terraform PR adding a WIF condition extension + an agent service account. Note it in the module doc comment as a TODO referencing this plan; leave the code out.
- The s3 module is the canonical shape reference throughout: `agent/src/s3/mod.rs`, `agent/src/s3/errors.rs`, `agent/tests/s3/mod.rs`, `agent/src/s3/.covgate`.
