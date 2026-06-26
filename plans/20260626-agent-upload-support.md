# Agent upload support

This ExecPlan is a living document. Sections Progress, Surprises & Discoveries,
Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent` (workspace root) | read-write | Implement device-side file upload support: discover upload rules, collect eligible files, mint presigned uploads, stream them to the destination bucket, confirm, and apply delete policy. Delivered as a sequence of small, independently reviewable PRs on branch `claude/agent-upload-support-9ldrq2`. |
| `openapi` | read-only (reference) | Source of the upload API contract (agent audience). The `feat: add data-uploads` surface (PR #133) is already merged to `main`. |

## Purpose

The backend now exposes an agent-audience upload API
(`openapi/apis/apps/backend-server/agent/`). The agent must consume it to push
captured device files (logs, MCAP, parquet, etc.) to customer buckets, governed
by per-release **upload rules**.

The API defines a four-call lifecycle:

| Step | Call | Notes |
|------|------|-------|
| Discover | `GET /upload_rules` | Rules in the device's currently-deployed release. |
| Mint | `POST /uploads` (`CreateUploadRequest`) | Backend creates a ledger entry and returns a short-lived V4-presigned PUT URL (`PresignedUpload`) plus `required_headers`. `already_uploaded=true` ⇒ skip the PUT. Device identity is derived from the session token, never the body. |
| PUT | the presigned `upload_url` | Raw bytes to the bucket (e.g. GCS). External URL, **no** auth header, `required_headers` echoed as HTTP headers. |
| Confirm | `POST /uploads/{upload_id}/confirm` | Marks the upload `uploaded`. |

**Upload rule shape** (`UploadRuleSource` / `UploadRuleDestination`):
- `source`: `glob` (absolute, `^/`), `poll_interval` (`^\d+(s|m|h)$`), `stability_window` (same).
- `destination`: `bucket_id`, `bucket_name`, `path` template (must contain `{device_id}`; supports `{filename}`), `delete_policy` (`never` | `after_upload`).

**Mint request** needs: `upload_rule_id`, `source{file_path, file_modified_at}`,
`digest` (sha256), `size`, `incomplete`, `release_id`, `deployment_id`.

### Observable outcome

When complete and enabled, a device on a release that defines upload rules
periodically scans each rule's glob, detects finished files, uploads each one
exactly once (deduplicated by digest), confirms it, and deletes the local copy
when `delete_policy=after_upload`. Until the final wiring PR, none of this is
active in production.

## How it maps onto the existing architecture

- **Generated models** (`libs/backend-api`) — currently contain **no** upload
  types. Vendored spec is `api/specs/backend/v04.yaml`; the upload schemas live
  in the openapi agent bundle but are not yet in the agent's pinned copy. The
  agent vendors by **copying the bundle** + `api/regen.sh` (per `ARCHITECTURE.md`),
  not by pinning a release tag — so no openapi release is required.
- **`http/`** — hand-written per-resource clients (`releases.rs`, `deployments.rs`).
  The request builder (`http/request.rs`) supports only GET/POST/PATCH against
  `base_url` **with** an auth token. The presigned PUT is an external URL, no
  auth, custom headers, large streaming body — a genuinely new client primitive.
- **`services/`** — `BackendFetcher` seam + per-resource services. Upload
  orchestration belongs here, behind a trait so it is mock-testable.
- **`storage/`** — typed file-backed stores with capacities (`releases.rs`). A
  local upload ledger (dedup by digest; avoid re-digesting large files every
  poll) mirrors this.
- **`workers/`** — `poller.rs` is the template for a timer-driven upload worker.
  No MQTT event exists for upload rules; discovery rides the existing release/sync.
- Code-gen emits **models only**; all client/service/worker code is hand-written.

## Milestones (one reviewable PR each)

Dependency-ordered. Nothing is user-visible until M7.

### M0 — Vendor spec + regen models
- [ ] Copy the upload schemas into the agent's vendored backend spec
  (`api/specs/backend/`) — uploads are part of the existing v0.4 surface.
- [ ] Run `api/regen.sh`; commit generated `libs/backend-api` types
  (`Upload`, `PresignedUpload`, `UploadRequiredHeaders`, `UploadRule`,
  `CreateUploadRequest`, `UploadRuleList`, upload status/delete-policy enums).
- [ ] No behavior change. Mechanical review.

### M1 — `models::upload`
- [ ] Domain wrappers over the generated types (rule, source, destination,
  delete-policy enum via `impl_status_enum!` if applicable).
- [ ] Helpers: duration parsing (`60s`/`5m`/`1h`), `{device_id}`/`{filename}`
  path-template rendering. `From<backend_client::…>` conversions. Pure + tests.

### M2 — Backend HTTP calls
- [ ] `http::upload_rules::list` → `GET /upload_rules`.
- [ ] `http::uploads::create` → `POST /uploads`.
- [ ] `http::uploads::confirm` → `POST /uploads/{id}/confirm`.
- [ ] Mirror `http/releases.rs`; uniform with existing patterns.

### M3 — Presigned object PUT primitive
- [ ] New client capability: streaming PUT to an arbitrary URL, no auth header,
  caller-supplied header map. Isolated PR — it is the one novel networking piece
  (large-file streaming, `expires_at` handling, external host).

### M4 — File collection / eligibility
- [ ] Glob expansion (absolute, anchored at `/`).
- [ ] Stability detection: size + mtime quiescent over `stability_window`.
- [ ] sha256 digest + size. Finalization-marker detection (MCAP/parquet) may
  stub initially behind the stability fallback. Temp-dir tests.

### M5 — `services::upload`
- [ ] Orchestrate: rule + eligible file → mint → (skip if `already_uploaded`)
  → stream PUT with `required_headers` → confirm → apply delete policy.
- [ ] Consume M2–M4 behind a trait; mock-tested like existing services.

### M6 — Local upload ledger (`storage::uploads`)
- [ ] Persist uploaded digests/paths so polls don't re-mint/re-digest.
- [ ] Add to `Capacities` / `Layout` / `Storage`; mirror `storage/releases.rs`.

### M7 — Upload worker + wiring (activation)
- [ ] `workers::uploads`: per-rule timers on `poll_interval`, scan→collect→
  upload loop, graceful shutdown (mirror `poller.rs`).
- [ ] Spawn from `AppState` / `app/run.rs`; config flag to enable; respect
  shutdown ordering. This PR turns the feature on.

### M8 — (optional) socket-server visibility
- [ ] Expose upload status via the Unix socket server for CLI/frontend.

## Surprises & Discoveries

- The upload API surface is merged to openapi `main` (PR #133
  "feat: add data-uploads"). No openapi release/tag is needed — the agent
  vendors by copying the bundle.
- `http/request.rs` supports only GET/POST/PATCH against `base_url` with a token.
  The presigned PUT needs a new primitive (external URL, no auth, custom headers,
  streaming body) — split into its own milestone (M3).
- No MQTT event signals upload-rule changes; rule discovery rides the existing
  release/sync path.

## Decision Log

- Decision: Ship as the M0–M7 PR sequence above, each independently reviewable,
  with the feature inert until M7.
  Rationale: Keeps diffs small and reviewable; foundation (models/HTTP) lands
  before orchestration; nothing user-visible until fully wired and tested.
  Date/Author: 2026-06-26 / Claude.

## Open questions

- **Streaming**: stream file bodies, never buffer (robots produce GB-scale MCAP/
  logs). Confirm the HTTP client can stream a file body.
- **Expiry/retry**: re-mint when `expires_at` passes mid-upload; retry policy for
  failed PUTs.
- **Concurrency / bandwidth**: how many uploads in flight; throttle on-device.
- **Dedup**: trust `already_uploaded` plus the local digest ledger (M6).
- **`incomplete`**: how the agent determines a file was never closed cleanly.
- **Delete policy**: `after_upload` deletes only after a *successful* confirm.

## Outcomes & Retrospective

_To be completed as milestones land._
