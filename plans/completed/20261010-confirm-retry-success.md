# Treat an already-confirmed upload as success on confirm retry

Linear: ENG-1545 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

`LiveExecutor::confirm_upload` (`agent/src/data_uploads/upload/executor.rs`) wraps `POST /uploads/{id}/confirm` in `http::with_retry`, which re-sends on network errors. If the backend committed the confirm but the response was lost, and the retry got a terminal 4xx, the job was dropped as a failure and `enqueue_delete_job` never ran, so a `require_upload: true` file was never deleted.

The backend's confirm is idempotent today (`internal/configs/services/uploads/confirm.go`: an already-uploaded row returns 200 without changes), so a retry currently gets 200. The fix guards the agent against a backend that answers a repeated confirm with 409 Conflict.

## Progress

- [x] M1: count confirm sends; when a retried send answers 409, log and return success.
- [x] M2: tests, lint, clippy.

## The Change

- `confirm_upload` counts sends with an `AtomicU32` inside the retry closure.
- A 409 after more than one send is treated as "already confirmed" and returns `Ok(())`, so the uploader goes on to enqueue the delete job.
- A 409 on the first send cannot be our own lost commit, so it still fails as before (terminal).

## Validation

New tests in `agent/tests/data_uploads/upload/executor.rs`:

- `lost_confirm_response_then_conflict_is_success`
- `lost_confirm_response_then_ok_is_success`
- `first_confirm_conflict_is_terminal`
- `lost_confirm_response_then_other_4xx_is_terminal`

Plus `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- Create and transfer handling, retry policy, and error classification for every other status.
