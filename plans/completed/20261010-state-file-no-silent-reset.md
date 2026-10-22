# Don't wipe state files on read or parse errors

Linear: ENG-1541 (File Uploads Tech Debt, M0 data-loss fixes). Also resolves the hazard noted in `plans/active/20260812-retention-queue-structural-durability.md` D4.

## Purpose

`SingleThreadStateFile::new_with_default` (`agent/src/filesys/state_file.rs`) overwrote the file with an empty default on any error from `new`: a transient EIO, EACCES after a service-account change, a truncated file, or one entry that no longer deserialized after an upgrade or downgrade. `upload_queue.json` or `delete_queue.json` was silently emptied with no log line, and because the scanner ledger already marked those files as handled, they were never uploaded or deleted.

## Progress

- [x] M1: per-error handling in `new_with_default`, with a bounded retry for read errors.
- [x] M2: tests, lint, clippy.

## The Change

`SingleThreadStateFile::new_with_default(file, default)`:

- **Missing file:** create it with `default` (unchanged).
- **Parse error:** copy the file aside to `<name>.corrupt-<uuid>`, log at `error!`, then atomically replace it with `default`. The copy is best effort: if it fails, the error is logged and the file is still replaced, so the agent keeps working. Nothing reads the aside copy back; it is kept for inspection.
- **Any other error:** retry the read after 50 ms, 250 ms and 1 s. If it still fails, return the error and leave the file untouched. App state already degrades to running without persistence for that boot (fail-open), so the on-disk queue survives for the next boot.

Per-entry salvage of a corrupt queue was considered and dropped: the real fix for entries that stop parsing after a version change is making the job types tolerant of each other's shapes, which is a separate follow-up.

Behavior change to note: the token file now fails boot on a non-parse read error (EIO/EACCES) that persists through the retries, instead of being reset to an empty token. A parse error still resets it, with the old contents kept aside.

## Validation

- `filesys/state_file.rs` `new_with_default`: `exists_invalid_data` asserts the `.corrupt-` sibling holds the original bytes; `unreadable_file_is_left_untouched` (a directory at the path); `transient_read_error_is_retried`; `repeated_recoveries_keep_every_aside_copy`.
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`.

## What does NOT change

- Snapshot formats.
- Every state file opened through `new_with_default` (queues, scanner snapshot, token) gets the same handling.
