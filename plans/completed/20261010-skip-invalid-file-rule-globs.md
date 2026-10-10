# Skip file rules whose scanner cannot be built instead of aborting the rule update

Linear: ENG-1539 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

In `SingleThreadScanner::update_rules` (`agent/src/data_uploads/scan/scanner.rs`), `RuleScanner::new(...).await?` returned early before `self.deployed` was replaced. `files::glob` rejects a pattern like `/logs/[ab.log`, and the backend did not validate glob syntax, so one bad rule meant:

- the previous release's rules stayed deployed, kept uploading, and kept feeding retention deletions under a policy the user had replaced;
- the valid new rules had scanners but were not in `deployed`, so they were pruned on the next tick;
- the same thing repeated on every sync.

## Progress

- [x] M1: build each rule's scanner through `ensure_scanner`, which logs and reports failure; leave failed rules out of `deployed` and always commit it.
- [x] M2: test, lint, clippy.

## The Change

- New `SingleThreadScanner::ensure_scanner(deployment, rule, now) -> bool` retargets an existing scanner or creates one. On a creation error it logs at `error!` and returns `false`.
- `update_rules` removes rules that returned `false` from the new deployed set, then commits `deployed` and persists as before. The duplicate-rule-id check still rejects the whole update before any mutation.
- The applied-rules log line reports how many rules were skipped.

## Validation

- `update_rules::update_rules_skips_invalid_glob_and_applies_the_rest`: deploys rule `old`, then pushes `[bad (/logs/[ab.log), new]`. The update succeeds, `old` is pruned on the next tick, only `new` remains active, and only `new` reports a newly written file.
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- Scan-time handling of a bad glob (`discover_candidates` already logs and skips).
- Duplicate rule id rejection.
- Backend-side glob validation is ENG-1540, in `mirurobotics/backend`.
