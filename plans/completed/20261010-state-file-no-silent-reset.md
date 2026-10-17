# Don't wipe state files on read or parse errors

Linear: ENG-1541 (File Uploads Tech Debt, M0 data-loss fixes). Also resolves the hazard noted in `plans/active/20260812-retention-queue-structural-durability.md` D4.

## Purpose

`SingleThreadStateFile::new_with_default` (`agent/src/filesys/state_file.rs`) overwrote the file with an empty default on any error from `new`: a transient EIO, EACCES after a service-account change, a truncated file, or one entry that no longer deserialized after an upgrade or downgrade. `upload_queue.json` or `delete_queue.json` was silently emptied with no log line, and because the scanner ledger already marked those files as handled, they were never uploaded or deleted.

## Progress

- [x] M1: `new_or_recover` with per-error handling; `new_with_default` delegates to it.
- [x] M2: `QueueSnapshot::salvage` and `queue::open_snapshot_file`; app state opens both queues through it.
- [x] M3: tests, lint, clippy.

## The Change

`SingleThreadStateFile::new_or_recover(file, default, salvage)`:

- **Missing file:** create it with `default` (unchanged).
- **Parse error:** move the file aside to `<name>.corrupt-<unix-ts>`, log at `error!`, then recreate it from `salvage(bytes)` or, if that recovers nothing, from `default`.
- **Any other error:** return it and leave the file untouched. App state already degrades to running without persistence for that boot (fail-open), so the on-disk queue survives for the next boot.

`new_with_default` is `new_or_recover` with a salvage that recovers nothing. For the queues, `QueueSnapshot::salvage` keeps every entry that still deserializes on its own and logs how many it dropped; `queue::open_snapshot_file` wires it in, and `App` uses it for `upload_queue.json` and `delete_queue.json`.

Behavior change to note: the token file now fails boot on a non-parse read error (EIO/EACCES) instead of being reset to an empty token. A parse error still resets it, with the old contents kept aside.

## Validation

- `filesys/state_file.rs`: `exists_invalid_data` now also asserts the `.corrupt-` sibling holds the original bytes; new `unreadable_file_is_left_untouched` (a directory at the path), `new_or_recover::unparseable_file_is_recreated_from_salvage`, `new_or_recover::missing_file_uses_default`.
- `data_uploads/upload/queue.rs` `wire::unparseable_entry_is_dropped_and_the_rest_kept`.
- `data_uploads/queue` unit tests: `salvage_rejects_bytes_that_are_not_a_snapshot`, `salvage_keeps_only_parseable_entries`.
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- Snapshot formats.
- The scanner snapshot gets the move-aside behavior but no per-entry salvage.
