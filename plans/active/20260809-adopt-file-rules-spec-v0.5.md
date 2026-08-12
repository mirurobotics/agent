# Adopt agent spec v0.5.0-beta.1: upload rules → file rules

High-level roadmap for refactoring the agent to the `agent/v0.5.0-beta.1` backend spec
(openapi repo, tag `agent/v0.5.0-beta.1`). This is a multi-PR umbrella plan; each PR gets
its own ExecPlan when work starts.

## What the spec changes (agent-facing surface)

Source: `openapi` repo, `git log agent/v0.4.1..agent/v0.5.0-beta.1` (#212, #230, #232, #234, #214, #219).

**1. Upload rule → file rule (rename + restructure, breaking):**

- Schemas: `BaseUploadRule` → `BaseFileRule`; ID prefix `upl_rule_` → `file_rule_`;
  `Release.upload_rules` → `Release.file_rules`; expansion token `upload_rules` → `file_rules`;
  `CreateUploadRequest.upload_rule_id` → `file_rule_id`; `Upload.upload_rule_id` → `file_rule_id`.
- New required `name` on the rule (participates in the digest; not unique per workspace).
- `upload_collection_id` / `upload_collection_name` move off the top level into a new
  **optional `upload` block** alongside `bucket_id` / `bucket_name` / `path`.
- `destination` block is gone. `delete_policy` (`never`/`after_upload`) and the enum
  `UploadDeletePolicy` are gone, replaced by an **optional `retention` block**:
  `{ require_upload: bool, ttl_secs: u64 }`.

Final semantics (per #234, which supersedes the intermediate policy-enum shapes):

- **No `retention` block** → Miru never deletes matching files.
- **`retention` present** → delete each matching file `ttl_secs` after it becomes
  *eligible*: file quiescent past `source.stability_window_secs`, and — iff
  `require_upload: true` — its upload durably confirmed. `ttl_secs: 0` = delete at
  eligibility. `require_upload` is required exactly when the rule has an `upload` block.
- **A rule may be retention-only** (no `upload` block): the agent must delete files it
  never uploads. It may also be upload-only (no `retention`): upload, never delete.
- Old→new semantic mapping: `delete_policy: never` ≡ retention absent;
  `delete_policy: after_upload` ≡ `retention { require_upload: true, ttl_secs: 0 }`.

**2. Config instance slots (#214, mostly additive for the agent):**

- `ConfigInstance` gains **required `slot_key`**. The agent bundle has no `ConfigSchema`
  at all, so the `instance_filepath` → `instance_slots[]` change does not touch the agent.
  Impact: regenerated `BaseConfigInstance` requires `slot_key`; test fixtures that build
  backend config-instance JSON must include it. The agent's own domain `ConfigInstance`
  can ignore it initially (instances still carry `filepath`).

## Current agent architecture (mapped 2026-08-09)

- **Wire → domain:** sync (`sync/deployments.rs`) and services (`services/release/get.rs`)
  request `expand=release.upload_rules` / `upload_rules`, hard-error if absent, convert
  `BaseUploadRule` → `models::UploadRule` (`models/upload_rule.rs`).
- **Persistence:** `resources/upload_rules.json` (FileCache keyed by rule ID),
  `resources/releases.json` (`upload_rule_ids`), `scanner.json` (embeds *whole* rules per
  collection + `StableFile.upload_rule_id`/`delete_policy` ledger), `upload_queue.json`
  (`Job.upload_rule_id` + `delete_policy`). Snapshot loaders start fresh on parse failure.
- **Scan:** `scan/scanner.rs` keys collections by `upload_collection_id` (rejects
  duplicates); `scan/collection.rs` stamps `destination.delete_policy` onto `StableFile`s.
- **Upload:** `upload/executor.rs` builds `CreateUploadRequest { upload_rule_id, .. }` and
  — the *only* deletion path today — deletes the source file after upload confirm iff
  `delete_policy == AfterUpload`.
- **Codegen:** `api/specs/backend/v04.yaml` is a hand-stamped vendored copy of the openapi
  agent bundle; `api/regen.sh` wholesale-replaces `libs/backend-api/src/models/`.
  Template: `plans/completed/20260713-revendor-backend-spec-bucket-name.md`.
- **Version plumbing:** base path is already `/agent/v1`; the version rides in the
  `Miru-Version` header from generated `ApiVersion` (currently `"v0.4"`). Device API
  (`libs/device-api`, local SDK server) has zero upload-rule surface — untouched.

## Gaps the refactor must close (behavioral, not renames)

1. **Retention engine.** There is no TTL clock, no persisted eligibility timestamps, no
   deletion decoupled from upload confirm. Needed: track per-file eligibility (stability
   reached; upload confirmed when required), schedule deletion `ttl_secs` later, survive
   restarts (persist eligibility in the scanner ledger), and handle the wedged-upload case
   (`require_upload: true` + upload permanently failed → file is never eligible; that is
   the spec'd behavior — no expiry backstop survived into the final shape).
2. **Retention-only rules.** The scan pipeline assumes every rule uploads: scanners are
   keyed by `upload_collection_id` and every stable file becomes an upload job. Rules
   without an `upload` block need scanning + deletion with no upload job ever minted.
   Re-key scanners by **rule ID** (rule IDs are unique; collection IDs are optional now).
3. **Upload-only rules** (no retention): today's `delete_policy: never` behavior — keep.

## Rollout constraints

- The upload feature has **zero production users** (stated in openapi #212; backend
  migration comment concurs). On-device persisted state may be reset rather than
  migrated: snapshot loaders already start fresh on parse failure — lean on that.
- **The backend has not landed its v0.5 half yet** (openapi #234 notes backend #583–#585
  still carry an older shape). The wire flip PR cannot merge-and-release until the
  backend serves v0.5. Everything internal can land before that.
- Old agents cannot deserialize new-shape rules (`destination` required in the custom
  deserializer) — fine given zero users, but the wire flip must ship as a coordinated
  breaking release (`v0.5.0-beta.x` agent release train).

## PR sequence

Internal-first, wire-last: PRs 1–3 restructure the agent against the *existing* v0.4 wire
models via an adapter, so each lands green with no backend dependency; PR 4 flips the wire
when the backend is ready.

### PR 1 — Internal domain restructure: `UploadRule` → `FileRule` (mechanical, behavior-preserving)

- New domain model `models/file_rule.rs`: `FileRule { id, name, digest, source, upload: Option<FileRuleUpload>, retention: Option<FileRuleRetention>, created_at, updated_at }`.
- Adapter from the current generated `BaseUploadRule`: `destination` + top-level
  collection fields → `upload` block; `delete_policy: never` → `retention: None`;
  `after_upload` → `Some { require_upload: true, ttl_secs: 0 }`. `name` ← collection name
  (placeholder until the wire carries it).
- Thread the new type through disk cache, scanner config, bridges, upload job. Preserve
  today's behavior exactly (delete-after-confirm iff `retention.require_upload` with
  `ttl_secs: 0`). Rename internal field names (`upload_rule_id` → `file_rule_id`) and
  disk cache file (`resources/upload_rules.json` → `file_rules.json`; stale snapshots
  reset via existing parse-failure paths — acceptable, zero users).
- Update the extensive fixture/test surface mechanically.

### PR 2 — Scanner re-keying + upload-less rules

- Key `scan/scanner.rs` collections by **rule ID**, drop the duplicate-collection-ID
  invariant, and make the scan→upload bridge mint upload jobs only when
  `rule.upload.is_some()`.
- Scanning (glob + stability window + ledger) now runs for every rule regardless of
  upload. `StableFile` carries the rule's retention config instead of `delete_policy`.

### PR 3 — Retention engine (architecture revised 2026-08-12, Ben's call)

**The scanner stays retention-unaware.** The original sketch (eligibility in the scanner
ledger, confirm fed back to the scanner) is superseded: eligibility lives in a standalone
delete subsystem — draft #191's architecture, modernized to `FileRuleRetention`. The
scanner's job ends at emitting stable files (its events carry the full `FileRule` since
PR 2's follow-ups, which is what makes retention-unaware producers possible).

- `delete/` module: a persisted pending-delete queue (`delete_queue.json`) of
  event-agnostic `PendingDelete` records ("this exact file — path, size, mtime, digest —
  became deletable at `eligible_at`; delete it `ttl_secs` later") and a `Deleter` actor
  whose sweep re-stats each due entry: size+mtime match → delete; mtime-only change →
  re-hash and delete only on digest match; otherwise drop without deleting.
- Two producers, one per eligibility trigger:
  1. **Upload confirm** (`retention.require_upload: true`): `LiveExecutor` enqueues on
     confirm instead of deleting inline; enqueue failure never fails the upload.
  2. **Stability** (retention present, `require_upload` false or absent — includes
     retention-only rules): a scan-event subscriber enqueues from the stable-file event,
     taking `ttl_secs` from the event's rule.
- `workers/delete.rs` interval driver (60s default, mirroring `workers/scan.rs`);
  fail-open init (deleter spawn failure degrades to uploads-without-deletion).
- Ships as two PRs: **3a** = queue + actor + worker + the upload-confirm producer
  (behavior-preserving modulo timing: ttl-0 deletes on the next sweep instead of inline);
  **3b** = the stability producer + drop `StableFile.retention` (the ledger keeps only
  file facts; retention travels on the event's rule).
- Draft #191 is closed as superseded; its queue/actor/sweep code is ported, its
  `delete_delay_secs` threading is not (superseded by `FileRuleRetention.ttl_secs`).
- Known gap, accepted: a crash between a stable-file emit and the enqueue persisting
  loses that pending delete — fail-open (file retained), never fail-deadly. The ledger
  dedups re-emits, so the record is not reproduced on restart.

### PR 4 — Wire flip: re-vendor spec v0.5.0-beta.1 + regen

Blocked on backend serving v0.5. Follow `plans/completed/20260713-revendor-backend-spec-bucket-name.md`:

- Copy `openapi/apis/apps/backend-server/agent/openapi.gen.yaml` → `api/specs/backend/v05.yaml`
  (delete `v04.yaml`); re-stamp the `info:` header (`version: v0.5`,
  `x-release-version: v0.5.0-beta.1`, `x-git-commit` = tag commit) and substitute both
  `$API_VERSION$` placeholders → `v0.5`. Update the Makefile/regen path if the filename
  is referenced.
- `api/regen.sh`: models become `BaseFileRule` / `FileRuleSource` / `FileRuleUpload` /
  `FileRuleRetention`; `ReleaseExpansion` → `file_rules`; `CreateUploadRequest.file_rule_id`;
  `BaseUpload.file_rule_id`; `BaseConfigInstance.slot_key` (required); `ApiVersion` → `v0.5`
  (drives the `Miru-Version` header automatically).
- Replace the PR-1 adapter with a direct `BaseFileRule → FileRule` mapping (real `name`,
  real optional blocks, real `ttl_secs`). Update expansion string literals
  (`sync/deployments.rs`, `services/backend.rs`), error variants/messages
  (`UploadRulesNotExpanded` → `FileRulesNotExpanded`), and test fixtures (including
  `slot_key` in backend config-instance builders).
- No base-path change (`/agent/v1` is version-agnostic); no device-api change.

### PR 5 — Cleanup + docs (optional, can fold into PR 4)

- Purge remaining "upload rule" vocabulary from logs/comments/ARCHITECTURE.md, archive
  this plan, note the state-reset behavior in release notes for the beta train.

## Open decisions (recommendations inline)

1. **Internal-first vs wire-first.** Recommended: internal-first (above) — the backend
   isn't ready, and PRs 1–3 are independently landable and testable.
2. **Persisted-state migration.** Recommended: reset-on-parse-failure, no migration code
   (zero users). Revisit only if beta users appear before PR 4 ships.
3. **`slot_key` threading.** Recommended: fixtures-only in PR 4; thread `slot_key` into
   the agent's domain `ConfigInstance` in separate future work if/when the agent needs
   slot awareness (instances still carry `filepath`, which is all the agent uses).
4. **Upload-confirm feedback path for retention** (PR 3): executor → scanner ledger
   update. Exact mechanism (event bus vs direct ledger write) decided in PR 3's ExecPlan.

## Risks

- **PR 3 concurrency/persistence design** — deletion driven by persisted timestamps must
  not double-delete or delete pre-eligibility after a clock-skewed restart; interaction
  with ledger pruning (a pruned entry must not resurrect a file's eligibility state).
- **Wedged uploads with `require_upload: true` hold files forever** — spec'd behavior
  (the expiry backstop was dropped in #234); worth a log/metric so fleets can see it.
- **No CI drift gate between spec and generated models** — PR 4 review must eyeball the
  regen diff; consider adding a drift check as follow-up tech debt.
- **Backend timing** — PR 4 is externally blocked; track backend #583–#585.
