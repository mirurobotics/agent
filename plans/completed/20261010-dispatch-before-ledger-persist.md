# Dispatch stable files before persisting the scanner ledger

Linear: ENG-1543 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

`SingleThreadScanner::scan` (`agent/src/data_uploads/scan/scanner.rs`) called `persist_snapshot()` with the new ledger entries before `dispatch_stable_files()` handed them to the upload and retention sinks. If the agent was killed in that window, or partway through a long dispatch, those files were recorded as reported but never reached the upload or delete queues, and were never offered again.

## Progress

- [x] M1: swap the order so the sinks get the files first and the ledger is persisted afterwards.
- [x] M2: test, lint, clippy.

## The Change

`scan()` now calls `dispatch_stable_files()` and then `persist_snapshot()`. A crash mid-dispatch leaves the files as candidates in the on-disk snapshot, so the next process re-emits them. The cost is a possible duplicate enqueue, which both queues already tolerate; a duplicate is cheaper than a lost file.

## Validation

- `sinks::ledger_is_persisted_after_dispatch`: a probe sink reads the snapshot file at delivery time and sees zero ledgered files; after the tick the snapshot holds one. With the old order the probe sees one (verified by temporarily restoring the old order).
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- What gets dispatched, the sinks, and the snapshot format.
- The alternative in the issue (a "pending dispatch" ledger state) was not needed.
