# Safe require_upload default and no duplicate delete job on confirm

Linear: ENG-1544 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

Two retention gating bugs:

1. `RetentionStableFileSink` (`agent/src/data_uploads/retention/sink.rs`) used `require_upload.unwrap_or(false)`. A rule with an upload block that arrived without `require_upload` (for example a stale cached rule) had its files deleted at stability, before they were uploaded.
2. The uploader's confirm path (`enqueue_delete_job` in `agent/src/data_uploads/upload/uploader.rs`) was gated only on `retention.is_some()`. For `require_upload: false` the retention sink had already queued a delete at stability, so confirm added a second one: two delete-queue slots against the 4096 cap and a redundant full-file hash. `plans/completed/20260813-upload-retention-producer.md` says confirm should only enqueue when the upload is required.

## Progress

- [x] M1: add `FileRuleRetention::requires_upload(rule_uploads)` and use it at both gates.
- [x] M2: update the test that pinned the duplicate, add tests for the absent-field defaults.
- [x] M3: test, lint, clippy.

## The Change

- `FileRuleRetention::requires_upload(rule_uploads: bool) -> bool` returns `require_upload.unwrap_or(rule_uploads)`: an absent field defaults to whether the rule uploads, which matches what the backend means when it omits it.
- The retention sink skips the stability-time enqueue when `requires_upload(rule.upload.is_some())`.
- The uploader enqueues a delete job at confirm only when `requires_upload(true)` (upload jobs only exist for rules that upload).

## Validation

- `retention/sink.rs`: `upload_rule_with_absent_require_upload_enqueues_nothing`; the existing retention-only absent case still enqueues.
- `upload/uploader.rs` `retention_producer`: `unrequired_retention_enqueues_nothing` replaces `unrequired_retention_also_enqueues_a_delete_job`; new `absent_require_upload_enqueues_a_delete_job`.
- `models/file_rule.rs`: `requires_upload_honors_an_explicit_value`, `requires_upload_defaults_to_whether_the_rule_uploads`.
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- Rule parsing and the wire format of `FileRuleRetention`.
- Delete-queue capacity (tracked separately).
