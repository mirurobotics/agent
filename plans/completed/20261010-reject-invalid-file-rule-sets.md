# Reject an invalid file rule set whole and deploy no rules

Linear: ENG-1539 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

In `SingleThreadScanner::update_rules` (`agent/src/data_uploads/scan/scanner.rs`), `RuleScanner::new(...).await?` returned early before `self.deployed` was replaced. `files::glob` rejects a pattern like `/logs/[ab.log`, and the backend did not validate glob syntax, so one bad rule meant:

- the previous release's rules stayed deployed, kept uploading, and kept feeding retention deletions under a policy the user had replaced;
- the valid new rules had scanners but were not in `deployed`, so they were pruned on the next tick;
- the same thing repeated on every sync.

The only caller, `workers/sync_scan_bridge.rs`, logs the error and nothing reaches the backend.

## Decision

An invalid rule set is rejected whole and **no** rules stay deployed until a valid one arrives. The alternatives both run a policy nobody approved:

- Keeping the previous rules keeps deleting under a replaced retention policy.
- Deploying only the valid rules applies a subset the user never approved. A valid retention rule can delete files that the skipped rule was meant to upload first.

Deploying nothing stops uploads and deletions alike. Files stay on disk until a valid release arrives, so nothing is lost.

## Progress

- [x] M1: validate the whole rule set before touching any scanner; on failure clear the deployed set and return the error.
- [x] M2: tests, lint, clippy.

## The Change

- New `files::validate_glob(pattern)` compiles a pattern with the same parser as `files::glob`, without walking the filesystem, and returns `FileSysErr::InvalidGlobErr` if it is invalid. Both share `compile_glob`.
- New `validate_rules(rules)` returns the rule IDs, or `DuplicateFileRuleID` / `InvalidGlobErr`.
- `update_rules` runs `validate_rules` first. On error it calls `clear_rules` (clears `deployed`, persists, logs) and returns the error. Otherwise it behaves as before.
- Duplicate rule IDs now also clear the deployed set instead of leaving the previous rules running.

## Validation

- `update_rules_invalid_glob_deploys_nothing`: deploys `old`, then pushes `[new, bad (/logs/[ab.log)]`. The update returns `InvalidGlobErr`, `old` is pruned on the next tick, no scanner is created for `new`, and a newly written file is never reported.
- `update_rules_duplicate_rule_id_deploys_nothing`: same outcome for a duplicate rule ID.
- `filesys::files::validate_glob`: valid and invalid patterns.
- `cargo test --package miru-agent`, `scripts/lint.sh`.

## What does NOT change

- Scan-time handling of a bad glob (`discover_candidates` already logs and skips).
- Error reporting: the failure is still only logged on the device. Reporting it to the backend is separate work.
- Backend-side glob validation is ENG-1540 (`mirurobotics/backend#877`).
