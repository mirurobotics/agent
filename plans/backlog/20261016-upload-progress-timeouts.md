# Make upload timeouts follow byte progress, not file size (ENG-1303)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent/` (`/home/ben/miru/workbench5/repos/agent`) | read-write | GCS and S3 store upload paths, upload transfer wiring, Cargo manifests, uploader doc comments. |

This plan lives in `agent/plans/` because every change is in the agent repo. Work happens on the already checked-out branch `fix/eng-1303-upload-progress-timeouts`; do not switch branches. All commands below run from `/home/ben/miru/workbench5/repos/agent` (the repo root, written `agent/` below; source paths are relative to it, e.g. `agent/src/gcs/store.rs`).

## Purpose / Big Picture

Today an upload attempt (create, transfer, confirm) has one deadline of 120 s plus one second per 64 KiB of file (about 4.6 h for 1 GiB, about 22.8 h for 5.4 GB). A connection that silently dies mid-upload is only noticed when that deadline expires, and the failure counts against the job's attempt budget. In the field, a 5.4 GB GCS upload stalled for about 9 h after a laptop suspend/resume, logged nothing, and only restarted (from byte 0) when the agent restarted.

After this change, any upload request that makes no byte progress for 60 s (or, after the full body, gets no response within a bounded tail) is aborted and surfaces as the existing retryable network error (`GcsErr::ConnectionErr` or `S3Err::ConnectionErr`). The uploader already requeues network errors after a flat 10 s wait without counting an attempt. S3 also gets an explicit 15 s connect timeout, per-call time limits (SDK retries included) scaled to the body length and always shorter than the backstop, stalled-stream protection that actually works on uploads, and one long-lived HTTPS client shared by all uploads. The whole-attempt deadline stays unchanged as a backstop. Normal uploads behave exactly as before.

Resuming a GCS resumable session or S3 multipart upload across agent attempts (so a retry does not restart from byte 0) is out of scope.

## Progress

- [ ] Move this plan to `plans/active/` and commit it.
- [ ] Milestone 1: GCS progress source and idle watchdog for `gcs::Store::put`, with tests; commit.
- [ ] Milestone 2: S3 timeouts, stalled-stream protection, chunked request bodies, uploader backstop docs, with tests; commit.
- [ ] Milestone 3: shared S3 HTTPS client wired into `SdkTransfer::default()`, with tests; commit.
- [ ] Push the branch and run `$preflight` until it reports CLEAN.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: Only bytes handed to the HTTP stack count as GCS progress; `seek` (called by the SDK on every attempt and retry) does not tick (it only clears the end-of-body flag).
  Rationale: An SDK retry loop against a dead network must not keep the watchdog alive. SDK retries are capped at 3, so a healthy retry produces new bytes well inside 60 s.
  Date/Author: 2026-10-16 / plan author.
- Decision: After the GCS source reports end of body, the watchdog waits up to `FINALIZE_TIMEOUT` (120 s) instead of the 60 s idle window; a `seek` after end of body (SDK retry) returns it to the idle window.
  Rationale: After the last frame, hyper's write buffer and the kernel send buffer (up to ~4 MiB) drain with no ticks before GCS answers. A 60 s window would fail every attempt on slow links, and each retry restarts from byte 0, uncounted, until `max_job_age`. 120 s drains a full 4 MiB buffer at the backstop's 64 KiB/s minimum rate, and equals the backstop floor. Both GCS upload paths send an unknown-length (chunked) body, so hyper always polls the source to `None`.
  Date/Author: 2026-10-16 / plan author.
- Decision: S3 bodies of 32 KiB or less stay single in-memory buffers. Only larger bodies use the chunked streaming body.
  Rationale: A body of one frame or less cannot benefit from stalled-stream protection, which needs several frames. Keeping small bodies in memory keeps the checksum-in-header wire format, so the existing small-body tests are unchanged.
  Date/Author: 2026-10-16 / plan author.
- Decision: Each S3 call gets one time limit, set as both its operation timeout (all SDK attempts and backoff) and its attempt timeout: 60 s for bodyless calls, 10 min for `CompleteMultipartUpload`, and `60 s + 1 s per 64 KiB` of body for PutObject/UploadPart (188 s for an 8 MiB part). SDK retries stay on.
  Rationale: SSP stops watching once the last frame is pulled, so a tail or response stall ends only at the limit. At the backstop's own 64 KiB/s rate with half its 120 s floor, the limit ends before `UploaderOptions::attempt_deadline` for the same size, as an uncounted `ConnectionErr`. Capping the whole operation stops up to 3 SDK attempts stacking past the backstop while keeping part-level SDK retries (fast 5xx, SSP trips) instead of restarting a multipart upload from byte 0. Completing a multipart upload can take minutes (AWS sends whitespace keepalives meanwhile).
  Date/Author: 2026-10-16 / plan author.
- Decision: The shared S3 client uses `aws_smithy_http_client::Builder::build_with_connector_fn` (public but `#[doc(hidden)]`) instead of the documented `build_https()`.
  Rationale: `build_https()` turns HTTP proxies off. The SDK default client that we replace honors `HTTP(S)_PROXY` from the environment, and builds itself through this same function. Dropping proxy support would silently break proxied devices. Cargo.lock pins the version.
  Date/Author: 2026-10-16 / plan author.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The agent uploads local files to object storage. Its upload pipeline lives in `agent/src/data_uploads/upload/`:

- `uploader.rs`: the upload actor. `UploaderOptions::attempt_deadline` (around line 80) computes `attempt_timeout_floor (120 s) + size / attempt_timeout_bytes_per_sec (64 KiB/s)`. `Worker::attempt_upload` wraps one attempt in `tokio::time::timeout`. A timeout becomes `UploadErr::AttemptTimeoutErr`, which is counted against the 30-attempt budget. `run_attempt` (line ~202) sends errors whose `is_network_conn_err()` is true to `handle_network_failure` (line ~250). That handler requeues the job after the flat `backoff.base_secs` (10 s) without counting an attempt. It drops the job only once the job is older than `max_job_age` (7 days).
- `errors.rs`: `classified_executor_err` copies `is_network_conn_err` from the store error. `gcs::GcsErr::ConnectionErr` and `s3::S3Err::ConnectionErr` are both network-classified (unit tests near errors.rs:136-159).
- `transfer.rs`: `SdkTransfer` (`#[derive(Default)]`, fields `s3_http_client: Option<SharedHttpClient>`, `s3_force_path_style`, `gcs_endpoint`) builds a fresh `s3::Store` or `gcs::Store` per upload via `s3_store()` / `gcs_store()`. `agent/src/app/state.rs` (~line 226) constructs `SdkTransfer::default()` once, for the life of the app.

GCS (`agent/src/gcs/`, coverage gate `.covgate` 88.00):

- `store.rs` defines `CONTROL_ATTEMPT_TIMEOUT` (30 s, gRPC delete/exists) and `READ_IDLE_TIMEOUT` (60 s). `get` wraps the initial `send()` and every body `next()` in `tokio::time::timeout(READ_IDLE_TIMEOUT, ..)`; on expiry `idle_timeout_err` returns `GcsErr::ConnectionErr { msg: "no data received for 60s" }`. `put` (line ~139) opens the file with `tokio::fs::File::open` and calls `self.data.write_object(&resource_name, &dst.key, file).set_metadata(..).send_unbuffered()` with no timeout at all. Its doc comment says so, and points at the stale path `agent/src/upload/uploader.rs` (the real path is `agent/src/data_uploads/upload/uploader.rs`).
- The comment on `CONTROL_ATTEMPT_TIMEOUT` says the client has "no TCP keepalive". That is accurate for the gRPC control client: `tonic` 0.14.6 `Endpoint` defaults to no connect timeout and no keepalive. The HTTP data client is different. It is a `reqwest` 0.13.4 client built by `google-cloud-gax-internal` 0.7.16 (`src/http.rs:85`), with keepalive 15 s ×3, `TCP_USER_TIMEOUT` 30 s on Linux only, and no connect timeout. The data-client SDK exposes no knob for a connect timeout or a custom HTTP client.
- `google-cloud-storage` 1.17.0, module `google_cloud_storage::streaming_source`: trait `StreamingSource { type Error; async fn next(&mut self) -> Option<Result<bytes::Bytes, Self::Error>>; async fn size_hint(&self) -> Result<SizeHint, Self::Error> }`, trait `Seek { type Error; fn seek(&mut self, offset: u64) -> impl Future<Output = Result<(), Self::Error>> + Send }`, public `Payload<T>` and `FileSource`. `Payload::<FileSource>::from(tokio::fs::File)` reads 256 KiB per `next()` and reports an exact size hint. `impl<S: StreamingSource> From<S> for Payload<S>` lets any wrapper type plug into `write_object`. `send_unbuffered` requires the payload type to be `StreamingSource + Seek + Send + Sync + 'static`. Below 16 MiB it does one multipart POST; at or above 16 MiB it uses a resumable session (one PUT of the whole remaining body per attempt, `seek(offset)` before each). Both paths call `seek` before sending, and the body is pulled lazily as hyper writes.
- Tests are inline in `store.rs` (`mod tests`, from line ~275). They use an axum mock (`crate::test_utils::http_client::run_server`, which binds 127.0.0.1:0) with an `HttpRecorder`, `upload_handler` (POST `/upload/storage/v1/b/{*rest}`) and `download_handler`, plus `Store::from_endpoint`. No test currently exercises `READ_IDLE_TIMEOUT` firing.

S3 (`agent/src/s3/`, coverage gate 94.00; aws-sdk-s3 1.141.0, aws-smithy-runtime 1.13.1, aws-smithy-runtime-api 1.14.0, aws-smithy-types 1.6.2, aws-smithy-http-client 1.3.0):

- `mod.rs`: `Store::build(cfg, http_client, force_path_style)` (line ~116) sets `BehaviorVersion::latest()`, region and static credentials, and optionally `.http_client(..)`. With `None`, the SDK installs a new default HTTPS client per `Store`, so every upload pays a fresh TCP+TLS handshake. `Store::from_http_client` is the test constructor. `put` sends files of `PART_SIZE` (8 MiB) or less through `put_singlepart` (reads the whole file into a `Vec`, `ByteStream::from(vec)`); larger files go through `put_multipart`.
- `multipart.rs`: `upload_part` (line ~224) sends `ByteStream::from(buf)`; `complete_multipart_upload`; `abort_multipart_upload` (best-effort cleanup, currently unbounded).
- `errors.rs::map_sdk_err` maps `SdkError::TimeoutError` and `SdkError::DispatchFailure` to `S3Err::ConnectionErr`. Attempt timeouts surface as `TimeoutError`. Connect timeouts and stalled-stream violations surface as `DispatchFailure`.
- Defaults today: connect timeout 3.1 s, no attempt timeout, standard retry (3 attempts).
- Stalled-stream protection (SSP) is the SDK's minimum-throughput guard (`aws_sdk_s3::config::StalledStreamProtectionConfig`; minimum 1 B/s, grace period 5 s by default, enabled for uploads). It is ineffective for our uploads. It observes the request body only when hyper pulls frames from it. Our body is a single in-memory buffer, so hyper pulls everything in one poll, SSP sees end-of-stream and marks the transfer complete, and the stall (hyper blocked writing to a dead socket) is invisible. SSP only works if the body is yielded in small frames.
- Per-operation overrides use `.customize().config_override(aws_sdk_s3::Config::builder().timeout_config(..))`. The orchestrator merges `TimeoutConfig` layers (`MergeTimeoutConfig`, aws-smithy-runtime `orchestrator.rs:200-208`), so unset fields inherit the client values. The HTTP client caches one connector (and so one connection pool) per `(connect_timeout, read_timeout)` pair (aws-smithy-http-client `client.rs` `CacheKey`), so keep those two values constant across all requests.
- S3 request checksums: for a streaming body (no in-memory bytes) the SDK sends `aws-chunked` content encoding with a CRC32 trailer. This needs an exact `size_hint`. Recorded request bodies in `StaticReplayClient` tests are then aws-chunked and no longer expose `.bytes()`.
- Tests are inline in `mod.rs` (`mod tests`, shared helpers `store_with`, `store_expecting`, `req`, `resp`, `obj`, `temp_file_with`, `actual_shapes`) and `multipart.rs`. They use `aws_smithy_http_client::test_util::{StaticReplayClient, ReplayEvent, NeverClient}`. `NeverClient` never responds.

Conventions (from `AGENTS.md`):

- Import groups headed `// standard crates`, `// internal crates`, `// external crates`, in tests as well.
- Unit tests go in inline `#[cfg(test)] mod tests` at the bottom of each file and import library items as `miru_agent::...`. Fixtures come from `crate::test_utils::...`, e.g. `test_files::temp`, `files::write_bytes(.., WriteOptions::OVERWRITE_NONATOMIC)`, not `std::fs`.
- Production functions stay at or under 50 non-blank, non-comment lines.
- No test may make 4+ field-by-field `assert_eq!` calls on one variable.
- No `#[cfg(unix)]` unless the test asserts Unix semantics.
- `cargo machete` requires every declared dependency to be imported somewhere.

Covered by existing tests (keep passing, do not duplicate): `agent/tests/data_uploads/upload/uploader.rs` `attempt_deadline_formula`, `hung_attempt_times_out_and_is_retried`, `network_failures_do_not_consume_attempt_budget`, `network_failure_uses_flat_cooldown`.

## Plan of Work

### Milestone 1: GCS progress watchdog

Add `bytes = "1"` to root `Cargo.toml` `[workspace.dependencies]` and `bytes = { workspace = true }` to `agent/Cargo.toml` `[dependencies]` (bytes 1.12.1 is already in `Cargo.lock`).

Create `agent/src/gcs/progress.rs` and declare it `mod progress;` (private) in `agent/src/gcs/mod.rs`. It holds:

- `pub(super) const FRAME_SIZE: usize = 32 * 1024`.
- `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub(super) struct Progress { pub(super) sent: u64, pub(super) eof: bool }`: the watch-channel value — cumulative bytes yielded and whether the inner source has ended.
- `#[derive(Clone, Copy)] pub(super) struct Limits { pub(super) idle: Duration, pub(super) finalize: Duration }`.
- `pub(super) struct ProgressSource<S> { inner: S, pending: bytes::Bytes, progress: watch::Sender<Progress> }` with `new(inner, progress)`.
- `impl<S: StreamingSource + Send + Sync> StreamingSource for ProgressSource<S>`: `next` refills `pending` from `inner.next()` while empty; `Err` passes through unchanged; on `None` it calls `send_if_modified(|p| !std::mem::replace(&mut p.eof, true))` and returns `None`; otherwise yields `pending.split_to(min(FRAME_SIZE, pending.len()))` and calls `send_modify(|p| p.sent += len)` before returning the frame. `size_hint` forwards to `inner`.
- `impl<S: Seek> Seek for ProgressSource<S>`: `seek` clears `pending`, calls `send_if_modified(|p| std::mem::replace(&mut p.eof, false))` (notifies only when leaving end of body), then returns `self.inner.seek(offset)`. It never changes `sent`.
- `pub(super) async fn watch_progress<T>(fut: impl Future<Output = Result<T, GcsErr>>, progress: watch::Receiver<Progress>, limits: Limits, obj: &Object) -> Result<T, GcsErr>`: `tokio::select! { biased; r = fut => r, bound = idle_elapsed(&mut progress, limits) => Err(..) }`, error `GcsErr::ConnectionErr(ConnectionErr { object: obj.clone(), msg: format!("no upload progress for {}s", bound.as_secs()), trace: trace!() })`.
- Private `async fn idle_elapsed(progress, limits) -> Duration`: each loop pass sets `bound = if progress.borrow_and_update().eof { limits.finalize } else { limits.idle }`, awaits `tokio::time::timeout(bound, progress.changed())`; `Ok(Ok(()))` continues, `Err(_)` returns `bound`, `Ok(Err(_))` (sender dropped) sleeps `bound` then returns it instead of spinning.

The 32 KiB frames matter: hyper pulls the next frame only after buffering the previous one. With the SDK's 256 KiB reads a 60 s window would fail a link slower than about 4.3 KiB/s mid-stream; with 32 KiB frames the floor is about 550 B/s. The tail after the last frame is covered by the finalize bound.

Edit `agent/src/gcs/store.rs`:

- Rename `READ_IDLE_TIMEOUT` to `IDLE_TIMEOUT`, documented as the maximum time `get` or `put` may go without byte progress. Update `get` and the `idle_timeout_err` doc to match.
- Add `const FINALIZE_TIMEOUT: Duration = Duration::from_secs(120)` (doc: "longest wait after the last body byte for send buffers to drain and GCS to answer") and `const UPLOAD_LIMITS: Limits = Limits { idle: IDLE_TIMEOUT, finalize: FINALIZE_TIMEOUT }`.
- Move `put`'s body into a private `async fn put_with_limits(&self, src: File, dst: &Object, metadata: &HashMap<String, String>, limits: Limits) -> Result<(), GcsErr>`; `put` becomes `self.put_with_limits(src, dst, metadata, UPLOAD_LIMITS).await` (tests call `put_with_limits` with millisecond limits). After opening the file:

      let (tx, rx) = tokio::sync::watch::channel(Progress::default());
      let payload = ProgressSource::new(Payload::<FileSource>::from(file), tx);
      let send = async {
          self.data.write_object(&resource_name, &dst.key, payload)
              .set_metadata(metadata.clone()).send_unbuffered().await
              .map_err(|e| map_gcs_err("put_object", dst, e))
      };
      progress::watch_progress(send, rx, limits, dst).await?;

- Rewrite the `put` doc comment so it states:
  - Every request is bounded by progress: no body bytes accepted by the HTTP stack for `IDLE_TIMEOUT`, or no response within `FINALIZE_TIMEOUT` after the last body byte, aborts the call with a retryable `GcsErr::ConnectionErr`.
  - The idle window also covers connect/TLS (the SDK offers no connect-timeout knob) and SDK-internal retries and backoff.
  - Dropping the future orphans any resumable session (GCS expires it server-side); the caller's next attempt restarts from byte 0.
  - `UploaderOptions::attempt_deadline` in `agent/src/data_uploads/upload/uploader.rs` remains only as a backstop.
- Reword the `CONTROL_ATTEMPT_TIMEOUT` comment to say "the gRPC control client" explicitly (it has no connect timeout or keepalive). Add one line noting that the HTTP data client does have reqwest's TCP keepalive but no connect timeout, which is why `put`/`get` use idle windows.

### Milestone 2: S3 timeouts, stalled-stream protection, chunked bodies

Add dependencies:

- Root `Cargo.toml` `[workspace.dependencies]`: `http-body = "1"`.
- `agent/Cargo.toml` `[dependencies]`: `http-body = { workspace = true }`, `aws-smithy-types = { workspace = true, features = ["http-body-1-x"] }`. Remove the now-redundant plain `aws-smithy-types` dev-dependency.
- `agent/Cargo.toml` `[dev-dependencies]`: `aws-smithy-runtime-api = { version = "1.14", features = ["client", "http-1x", "test-util"] }` (used by the SSP test and the Milestone 3 connector test).

All of these crates are already in `Cargo.lock`, so no new code or advisories enter the build.

Create `agent/src/s3/body.rs` (declare `mod body;` in `s3/mod.rs`):

    pub(crate) const FRAME_SIZE: usize = 32 * 1024;

    /// Request body for PutObject/UploadPart. Larger bodies are yielded in
    /// FRAME_SIZE frames so stalled-stream protection sees per-frame progress;
    /// retryable because each retry rebuilds from a cheap Bytes clone.
    pub(crate) fn chunked(bytes: Bytes) -> ByteStream {
        if bytes.len() <= FRAME_SIZE {
            return ByteStream::from(bytes);
        }
        ByteStream::new(SdkBody::retryable(move || {
            SdkBody::from_body_1_x(ChunkedBytes { remaining: bytes.clone() })
        }))
    }

`ChunkedBytes { remaining: Bytes }` implements `http_body::Body<Data = Bytes, Error = std::convert::Infallible>`:

- `poll_frame` returns `Ready(None)` when `remaining` is empty, otherwise `Ready(Some(Ok(Frame::data(remaining.split_to(min(FRAME_SIZE, len))))))`.
- `is_end_stream` returns `remaining.is_empty()`.
- `size_hint` returns `SizeHint::with_exact(remaining.len() as u64)`. An exact hint is required for Content-Length and the aws-chunked checksum trailer.

Import `SdkBody` from `aws_smithy_types::body` (not the `aws_sdk_s3` re-export) so `cargo machete` sees the dependency used.

Edit `agent/src/s3/mod.rs`:

- Constants:
  - `CONNECT_TIMEOUT = 15 s`.
  - `ATTEMPT_TIMEOUT = 60 s` (bodyless calls).
  - `STALL_GRACE = 60 s` (doc: "matches `gcs::store::IDLE_TIMEOUT`").
  - `MIN_UPLOAD_BYTES_PER_SEC: u64 = 64 * 1024` (doc: "the uploader backstop's minimum rate, so a body call ends before `attempt_deadline` for its size").
  - `pub(crate) COMPLETE_ATTEMPT_TIMEOUT = 600 s`.
- Helpers:
  - `fn timeout_config(limit: Duration) -> TimeoutConfig`: always sets `connect_timeout(CONNECT_TIMEOUT)`, `operation_timeout(limit)` and `operation_attempt_timeout(limit)`, so SDK retries and backoff fit inside the same limit (connector cache key is only (connect, read), so unaffected).
  - `pub(crate) fn body_attempt_timeout(len: u64) -> Duration` (`ATTEMPT_TIMEOUT + len.div_ceil(MIN_UPLOAD_BYTES_PER_SEC)` seconds; 188 s for an 8 MiB part).
  - `pub(crate) fn attempt_override(limit: Duration) -> aws_sdk_s3::config::Builder` (`aws_sdk_s3::Config::builder().timeout_config(timeout_config(limit))`).
- `Store::build` adds `.timeout_config(timeout_config(ATTEMPT_TIMEOUT))` and `.stalled_stream_protection(StalledStreamProtectionConfig::enabled().upload_enabled(true).grace_period(STALL_GRACE).build())`.
  - This bounds create, complete, abort, list, head, delete and the header phase of get, SDK retries included; the previously unbounded best-effort `abort_multipart_upload` is now capped at 60 s.
  - The grace period also applies to S3 downloads (was 5 s). That is acceptable and matches GCS.
- `put_singlepart`:
  - Build the body with `body::chunked(Bytes::from(src.read(..)?))`.
  - Insert `.customize().config_override(attempt_override(body_attempt_timeout(size)))` before `.send()`.
  - Update its doc: the body is in memory and retryable, and is yielded in frames for stall detection.

Edit `agent/src/s3/multipart.rs`:

- `upload_part` takes the `Vec<u8>`, computes `len`, uses `body::chunked(Bytes::from(buf))`, and adds `.customize().config_override(attempt_override(body_attempt_timeout(len)))`.
- `complete_multipart_upload` adds `.customize().config_override(attempt_override(COMPLETE_ATTEMPT_TIMEOUT))`.

Edit `agent/src/data_uploads/upload/uploader.rs` doc comments only (no formula or value change):

- `attempt_timeout_floor`, `attempt_timeout_bytes_per_sec` and `attempt_deadline` become a backstop that guarantees every attempt terminates.
- Stalls are caught first by the stores — GCS's no-progress watchdog (`gcs::Store::put`), and for S3 stalled-stream protection plus per-call time limits sized below this backstop — which surface as uncounted network errors.

### Milestone 3: shared S3 HTTPS client

In `agent/Cargo.toml` `[dependencies]` add `aws-smithy-http-client = { workspace = true, features = ["rustls-aws-lc"] }`. The dev-dependency entry with `test-util` stays. The feature is already compiled via `aws-sdk-s3/default-https-client`, so the TLS stack is unchanged (rustls 0.23 + aws-lc).

In `agent/src/s3/mod.rs` add:

    /// One HTTPS client for every S3 store built from it, so uploads reuse pooled
    /// connections instead of a fresh TCP+TLS handshake per upload. Mirrors the
    /// SDK default client: rustls with aws-lc, proxy settings from the environment.
    pub fn http_client() -> SharedHttpClient {
        Builder::new().build_with_connector_fn(|settings, components| {
            let mut conn = ConnectorBuilder::default()
                .tls_provider(tls::Provider::Rustls(tls::rustls_provider::CryptoMode::AwsLc));
            conn.set_connector_settings(settings.cloned());
            if let Some(components) = components {
                conn.set_sleep_impl(components.sleep_impl());
            }
            conn.set_proxy_config(Some(proxy::ProxyConfig::from_env()));
            conn.build()
        })
    }

Here `Builder`, `ConnectorBuilder`, `tls` and `proxy` come from `aws_smithy_http_client`. This is a copy of `aws-smithy-runtime-1.13.1/src/client/http.rs::default_https_client`.

In `agent/src/data_uploads/upload/transfer.rs`, replace `#[derive(Default)]` on `SdkTransfer` with a manual `impl Default` that sets `s3_http_client: Some(s3::http_client())`, `s3_force_path_style: false`, `gcs_endpoint: None`, with a one-line doc saying the client is built once per transfer, which lives for the app's lifetime. `s3_store()` already clones it into every `Store::build`. Use no global static: the app builds exactly one `SdkTransfer`, and tests stay isolated. Sharing the GCS client is out of scope, because its token is baked in at build time.

If any verification step shows Milestone 3 needs more than about 40 lines or touches `app/state.rs`, stop. Record it in Surprises & Discoveries and move it to a follow-up instead.

## Concrete Steps

All commands run from `/home/ben/miru/workbench5/repos/agent`. Never run cargo build, test, clippy, fmt or covgate locally; CI does validation.

0. Activate the plan:

       git mv plans/backlog/20261016-upload-progress-timeouts.md plans/active/
       git commit -m "docs(plans): activate ENG-1303 upload progress timeouts plan"

1. Milestone 1 code: apply the Milestone 1 edits above, then `cargo update --workspace --offline` and `git diff --stat Cargo.lock`. Expected: only `miru-agent`'s dependency list changes (gains `bytes`). If other packages change, `git checkout Cargo.lock` and retry.

2. Milestone 1 tests:

   In `agent/src/gcs/progress.rs`, add `mod tests` with private-access `use super::*`. Use `Payload::from(bytes::Bytes)` and `Payload::from(Vec<Bytes>)` as inner sources.
   - `rechunks_into_bounded_frames`: 100 KiB input yields frame lengths `[32768, 32768, 32768, 4096]`, and their concatenation equals the input.
   - `preserves_bytes_across_inner_chunks`: inner chunks of 40 KiB and 10 KiB yield `[32768, 8192, 10240]` with identical bytes.
   - `forwards_exact_size_hint`.
   - `seek_discards_pending_and_rewinds`: take one frame, `seek(0)`, then collect everything; the result equals the full input.
   - `progress_tracks_bytes_and_eof`: after draining, `*rx.borrow() == Progress { sent: total, eof: true }`; after `seek(0)` it equals `Progress { sent: total, eof: false }`.

   Watchdog tests under `#[tokio::test(start_paused = true)]` with pure futures and no sockets, using `Limits { idle: 60 s, finalize: 120 s }`:
   - `fires_after_idle`: `std::future::pending()`, sender kept alive. It returns `GcsErr::ConnectionErr` whose `msg` is `"no upload progress for 60s"`, after exactly `IDLE` of virtual time, measured with `tokio::time::Instant`.
   - `keeps_waiting_while_progress_arrives`: a spawned task ticks every 30 s five times; the future resolves `Ok(42)` at 150 s, and the result is `Ok(42)`.
   - `returns_inner_result`: an immediate inner `Err(RequestFailedErr)` passes through unchanged.
   - `dropped_sender_still_waits_full_window`: drop the sender; it fires `ConnectionErr` after `IDLE`, not immediately.
   - `eof_switches_to_finalize_bound`: drain a `ProgressSource` to `None`; `watch_progress(pending(), ..)` fires `ConnectionErr` after exactly 120 s, not 60 s.
   - `seek_after_eof_restores_idle_bound`: drain to `None`; a spawned task calls `seek(0)` at 30 s; the watchdog fires at 90 s.

   In `agent/src/gcs/store.rs` tests:
   - Add `upload_delay: Option<Duration>` and `download_stall: bool` to `HttpRecord`. `upload_handler` (its `Bytes` extractor has already read the body) sleeps for the delay before responding; `download_handler` awaits `std::future::pending::<()>()` when `download_stall` is set. Read the fields in a scoped block that drops the `MutexGuard` (not `Send`) before awaiting.
   - `put::progress::slow_response_after_full_body_succeeds` (real time): `upload_delay = 600 ms`; call `store.put_with_limits(..)` with `Limits { idle: 200 ms, finalize: 5 s }`, assert `Ok`. Fails unless `put` wires its `ProgressSource` into the watchdog (without the end-of-body signal the 200 ms idle fires).
   - `put::progress::unanswered_upload_maps_to_connection_err` (real time): `upload_delay = 1 h`, `Limits { idle: 200 ms, finalize: 500 ms }`; assert `GcsErr::ConnectionErr` and `err.is_network_conn_err()`.
   - `get::stall::stalled_download_maps_to_connection_err` (`start_paused = true`, `download_stall = true`): assert `GcsErr::ConnectionErr`, msg `"no data received for 60s"`, `is_network_conn_err()`. tokio may auto-advance to the 60 s timer before the request reaches the handler; either way `get`'s idle timeout is what fires.
   - Existing put/get tests stay unchanged.

3. Commit Milestone 1:

       git add Cargo.toml Cargo.lock agent/Cargo.toml agent/src/gcs
       git commit -m "fix(gcs): abort uploads that make no byte progress for 60s"

4. Milestone 2 code: apply the Milestone 2 edits. Then refresh only the lockfile entries for workspace members. This resolves dependencies and does not build anything:

       cargo update --workspace --offline
       git diff --stat Cargo.lock

   Expected: only `miru-agent`'s dependency list changes, gaining `aws-smithy-runtime-api` and `http-body 1.1.0`. If other packages change, `git checkout Cargo.lock` and retry. CI's `windows-check` runs with `--locked`, so the lockfile must be committed.

5. Milestone 2 tests:

   In `agent/src/s3/body.rs` add `mod tests`. Collect frames with `http_body_util::BodyExt::frame`.
   - `small_body_stays_in_memory`: a 32 KiB input gives `into_inner().bytes()` equal to `Some(input)`.
   - `large_body_yields_bounded_frames`: 100 KiB gives frame lengths `[32768, 32768, 32768, 4096]`, and the concatenation equals the input.
   - `size_hint_is_exact_and_end_stream_tracks_remaining`: check `size_hint().exact()` before and after one frame, and `is_end_stream()` only after the last frame.
   - `retry_clone_replays_full_body`: drain the body, then `try_clone()` the `SdkBody`; the clone yields the full input again.

   In `agent/src/s3/mod.rs` tests:
   - `construction::build_sets_timeouts_and_stall_protection`: `Store::new(cfg)`, then read `store.client.config()`. `timeout_config()` has connect 15 s, operation 60 s and attempt 60 s. `stalled_stream_protection()` has `upload_enabled()` true and `grace_period()` 60 s. Use at most three asserts per variable, or compare whole values.
   - `body_attempt_timeout_stays_under_uploader_backstop`: 0 bytes gives 60 s; `PART_SIZE` gives 188 s; for each of 0, 1 MiB and `PART_SIZE` the value is `<` `miru_agent::data_uploads::upload::uploader::UploaderOptions::default().attempt_deadline(len)`.
   - `timeouts::stalled_put_maps_to_connection_err` (`start_paused = true`): `Store::from_http_client(NeverClient::new(), cfg)`, `put_singlepart` of a small file. Assert `S3Err::ConnectionErr` and `is_network_conn_err()`. Whether the attempt timeout or SSP fires first, both map to `ConnectionErr`. SSP's throughput clock is wall-clock, so under paused time the attempt timeout normally wins. Also assert virtual time elapsed (`tokio::time::Instant`) is at most `body_attempt_timeout(size)`, proving SDK retries don't stack past the limit.
   - `stall_protection::chunked_body_trips_stall_protection`, in real time (not paused: SSP measures throughput with the wall-clock time source).
     - A test-only `HangAfterFirstFrame` client implements `aws_smithy_runtime_api::client::http::{HttpClient, HttpConnector}`. `call` moves the request body into a future that awaits one `frame()` and then `std::future::pending()`. `http_connector` returns `self.clone().into_shared()`, the same shape as `NeverClient`.
     - Build `Store::from_http_client(HangAfterFirstFrame, cfg)`. Send `store.client.put_object()` with `.body(body::chunked(256 KiB))`, using `.customize().config_override(aws_sdk_s3::Config::builder().stalled_stream_protection(StalledStreamProtectionConfig::enabled().upload_enabled(true).grace_period(Duration::from_secs(1)).build()).retry_config(RetryConfig::disabled()))`.
     - Map the error through `errors::map_sdk_err` and assert `S3Err::ConnectionErr` within a `tokio::time::timeout` of 30 s real time.
     - This proves frame-wise bodies make SSP fire. A single-buffer body would have been marked complete after the first poll.
     - If this proves unreliable, keep the attempt-timeout coverage, delete this test (the dev-dependency stays for Milestone 3's connector test), and record it in Surprises.

   In `agent/src/s3/multipart.rs` tests:
   - `stalled_upload_part_maps_to_connection_err` (`start_paused = true`, `NeverClient`): call `store.upload_part(&obj, "upload-1", 1, vec![0u8; 64 * 1024])` directly and assert `ConnectionErr`.
   - Fix the two tests that read the 8 MiB part 1 with `.bytes()`: `path_replaced_after_open...` (~763) and `large_file_routes_to_multipart` (~1137). Part 1 is now an aws-chunked stream; part 2 (≤ 1 KiB) stays in memory and is unchanged.
     - Add a `pub(super) async fn recorded_payload(req: &HttpRequest) -> Vec<u8>` helper in `s3::tests`. It returns `body().bytes()` when present. Otherwise it `try_clone()`s the body, collects it with `http_body_util::BodyExt::collect`, and decodes aws-chunked framing: repeat `<hex-len>\r\n<data>\r\n` until a zero length.
     - In both tests assert `recorded_payload(&requests[1]).await` equals the first `PART_SIZE` bytes of the source.
     - Fallback if collecting the recorded body is impractical: assert the `x-amz-decoded-content-length` header equals `PART_SIZE`, and note it in Surprises.

6. Milestone 2 uploader docs: edit the comments in `agent/src/data_uploads/upload/uploader.rs`. The existing uploader tests listed in Context already prove that `ConnectionErr` is retried uncounted and that the backstop still fires; add none.

7. Commit Milestone 2:

       git add Cargo.toml Cargo.lock agent/Cargo.toml agent/src/s3 agent/src/data_uploads/upload/uploader.rs
       git commit -m "fix(s3): bound upload connects and attempts and detect stalled bodies"

8. Milestone 3 code: apply the Milestone 3 edits. Run `cargo update --workspace --offline` again; expect no `Cargo.lock` change, because `aws-smithy-http-client` is already listed.

9. Milestone 3 tests:
   - In `agent/src/s3/mod.rs` tests, `construction::http_client_builds_connector`: build `let store = Store::build(cfg, Some(s3::http_client()), false)` and check `store.client.config().http_client().is_some()`; then call `HttpClient::http_connector(&s3::http_client(), &HttpConnectorSettings::builder().connect_timeout(Duration::from_secs(15)).build(), &RuntimeComponentsBuilder::for_tests().with_time_source(store.client.config().time_source()).build().unwrap())`. `for_tests()` installs a time source that panics on `now()`, which `http_connector` reads; the built S3 config always carries the system clock. Import `HttpClient`, `HttpConnectorSettings` and `RuntimeComponentsBuilder` from `aws_smithy_runtime_api::client::{http, runtime_components}` so the dev-dependency stays referenced. This runs the connector closure (rustls with native roots and an env proxy) without network I/O and covers it for the 94.00 gate.
   - In `agent/src/data_uploads/upload/transfer.rs` tests, `default_carries_shared_s3_http_client`: `SdkTransfer::default().s3_http_client.is_some()`. "Same instance across stores" cannot be observed through public APIs (`SharedHttpClient` has no identity accessor); the clone-into-`Store::build` path is a single line in `s3_store()`.

10. Commit Milestone 3:

        git add agent/Cargo.toml agent/src/s3/mod.rs agent/src/data_uploads/upload/transfer.rs
        git commit -m "perf(s3): reuse one HTTPS client across uploads"

11. Push and validate: `git push -u origin fix/eng-1303-upload-progress-timeouts`, then run `$preflight` and iterate until it reports CLEAN. Fix failures from CI logs (`gh run view --log-failed`), never by running checks locally. Update Progress, Surprises, Outcomes and move the plan to `plans/completed/` when done.

Commits are signed by the repo's default git config; add no signing flags. Estimated total diff: about 600-700 lines, roughly 230 production and the rest tests.

## Validation and Acceptance

Validation is CI-only. Do not run `cargo build`, `test`, `clippy`, `fmt`, `scripts/lint.sh` or `scripts/covgate.sh` locally. Push the branch and let `.github/workflows/ci.yml` run:

- `lint` (`LINT_FIX=0 ./scripts/lint.sh`: import linter, function length, fmt, clippy `-D warnings`, machete, rustsec).
- `test` (`./scripts/covgate.sh`). The gates `agent/src/gcs/.covgate` 88.00, `agent/src/s3/.covgate` 94.00 and `agent/src/data_uploads/upload/.covgate` 96.00 must still pass.
- `windows-check` (`cargo test --package miru-agent --locked` on Windows).
- The remaining `ci.yml` jobs (`windows_package_scope`, `windows-agent-build`, `windows-package`, `tools`, `debian-package`).

The `$preflight` skill must report CLEAN (CI green on the pushed branch head) before the PR leaves draft or the task is reported complete.

Acceptance behaviors and the tests that prove them:

- A GCS upload whose connection stops accepting bytes fails with `GcsErr::ConnectionErr` about 60 s after the last frame was taken (or 120 s after the full body if GCS never answers) instead of hanging for hours. Proved by the `progress.rs` watchdog tests (`fires_after_idle`, `dropped_sender_still_waits_full_window`, `eof_switches_to_finalize_bound`, `seek_after_eof_restores_idle_bound`) and `put::progress::unanswered_upload_maps_to_connection_err`.
- The uploader requeues that error after 10 s without counting an attempt. Proved by the existing `network_failures_do_not_consume_attempt_budget` and `network_failure_uses_flat_cooldown` together with the classification tests in `agent/src/data_uploads/upload/errors.rs`.
- A slow but moving GCS upload is not cut off while bytes flow or while its buffered tail drains. Proved by `keeps_waiting_while_progress_arrives`, `rechunks_into_bounded_frames` and `put::progress::slow_response_after_full_body_succeeds`.
- A stalled GCS download fails with "no data received for 60s". Proved by `get::stall::stalled_download_maps_to_connection_err`.
- S3 PutObject/UploadPart requests that stall fail as `S3Err::ConnectionErr` before the uploader backstop: mid-stream within ~60 s of SSP grace (retried by the SDK inside the call's limit), otherwise at the call limit of 60 s + 1 s per 64 KiB. Proved by `stalled_put_maps_to_connection_err`, `stalled_upload_part_maps_to_connection_err`, `chunked_body_trips_stall_protection`, `body_attempt_timeout_stays_under_uploader_backstop` and `build_sets_timeouts_and_stall_protection`. S3 connects are bounded at 15 s.
- Normal uploads are unaffected: all existing GCS put/get and S3 put/multipart/get tests pass, with only the body-inspection change described in step 5.
- `UploaderOptions::attempt_deadline` is unchanged and still fires as a counted backstop. Proved by the existing `attempt_deadline_formula` and `hung_attempt_times_out_and_is_retried`.
- One S3 HTTPS client is created per `SdkTransfer` and passed to every store. Proved by `default_carries_shared_s3_http_client` and `http_client_builds_connector`.

## Idempotence and Recovery

Every edit is a plain source change and is safe to reapply. `cargo update --workspace --offline` is idempotent. If it touches packages other than `miru-agent`, restore with `git checkout Cargo.lock` and rerun. Each milestone is one commit, so a failing milestone can be fixed with follow-up commits on the branch, or dropped with `git revert <sha>` without affecting earlier milestones. Milestone 3 is independent of Milestones 1-2 and can be reverted alone. If the paused-time download-stall test or a step 5 test is flaky, log it in Surprises and use step 5's fallback or drop the download-stall test; do not weaken the production timeouts to make a test pass. No data migrations or persistent state are involved. The uploader's on-disk queue format is untouched.
