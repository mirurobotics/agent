# Drop M2 from mqtt-resubscribe hotfix

## Scope

Single repo, single branch: read-write on `agent/`, branch `fix/mqtt-resubscribe-on-reconnect` (PR #60 against `release/v0.8`).

## Purpose / Big Picture

Drop M2 (`fix(mqtt): set clean_session=false ...`, sha `5cdd092`) from the v0.8.1 hotfix so the PR ships only M1 (re-subscribe on every successful ConnAck) and M3 (connection counter). M1 alone closes the silent-deafness bug for all reconnect paths; skipping M2 retains headroom against the user's EMQX Dedicated tier 1,000-session cap. Operators see the same fix delivered with no change to broker session-table pressure.

## Progress

- [ ] Rebase `fix/mqtt-resubscribe-on-reconnect` onto M1 to drop M2.
- [ ] Update `plans/completed/20260507-mqtt-resubscribe-on-reconnect.md` to record the drop.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: Drop M2 (`fix(mqtt): set clean_session=false ...`, sha `5cdd092`) from the v0.8.1 hotfix.
  Rationale: User's EMQX Dedicated tier caps the session table at 1,000. `clean_session=false` makes sessions persist across disconnects, so session count would trend toward fleet size + churn instead of online device count. M1 alone closes the bug for all reconnect paths; M2 was belt-and-suspenders. Skipping M2 keeps session-cap headroom on the current tier. M2 can land separately if a tier upgrade makes sessions cheap.
  Date/Author: 2026-05-07 / orchestrator on user request.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The branch `fix/mqtt-resubscribe-on-reconnect` is already pushed and has PR #60 open against `release/v0.8`. It currently carries three M-commits — M1 `41e89c8` (re-subscribe on ConnAck), M2 `5cdd092` (clean_session=false, to drop), M3 `635efc5` (connection counter, keep) — plus three plan-related commits and `f5d5f41` ("plan: mark mqtt-resubscribe hotfix complete"), totalling 7 commits over `origin/release/v0.8`.

M2 touches `agent/src/mqtt/options.rs`, `agent/src/mqtt/client.rs`, and `agent/tests/mqtt/options.rs`. M3 touches `agent/src/workers/mqtt.rs` and `agent/tests/workers/mqtt.rs`. The file sets are disjoint so the rebase that drops M2 will replay M3 cleanly. No `clean_session` references exist outside M2's files, so dropping M2 cannot break callers.

The existing plan at `plans/completed/20260507-mqtt-resubscribe-on-reconnect.md` documents all three milestones and must be updated to reflect the drop in its Plan of Work, Concrete Steps, Validation and Acceptance, and Decision Log sections.

## Plan of Work

1. Rebase to drop M2: `git rebase --onto 41e89c8 5cdd092 fix/mqtt-resubscribe-on-reconnect`. M3 replays cleanly given the disjoint file sets.
2. Run `./scripts/test.sh` to confirm tests pass without M2 and its test file.
3. Edit `plans/completed/20260507-mqtt-resubscribe-on-reconnect.md`: add the M2-drop Decision Log entry, change "three milestones" to "two milestones" wherever it appears, remove or strikethrough the M2 milestone block in Plan of Work and Concrete Steps, and remove M2-specific entries from Validation and Acceptance.
4. Commit the plan edits as `plan: drop M2 in light of EMQX 1k session cap` (signed).
5. Run `./scripts/preflight.sh` and confirm clean before push.

## Concrete Steps

### Milestone 1 — drop M2 via rebase

No new commit added — the rebase rewrites M3 onto M1 directly.

    cd /home/ben/miru/workbench5/repos/agent
    git branch --show-current   # expect: fix/mqtt-resubscribe-on-reconnect
    git log --oneline origin/release/v0.8..HEAD   # baseline: 7 commits
    git rebase --onto 41e89c8 5cdd092 fix/mqtt-resubscribe-on-reconnect
    git log --oneline origin/release/v0.8..HEAD   # expect: 6 commits, no 5cdd092
    git diff --stat origin/release/v0.8...HEAD    # expect: only workers/mqtt.rs, tests/workers/mqtt.rs, plans/...
    ./scripts/test.sh   # expect: all tests pass

### Milestone 2 — update plan doc

    # Edit plans/completed/20260507-mqtt-resubscribe-on-reconnect.md per Plan of Work step 3.
    git add plans/completed/20260507-mqtt-resubscribe-on-reconnect.md
    git commit -S -m "plan: drop M2 in light of EMQX 1k session cap"

### Final preflight

    ./scripts/preflight.sh   # expect: Preflight clean

## Validation and Acceptance

1. `git log --oneline origin/release/v0.8..HEAD` shows 6 commits, none with sha `5cdd092` and none whose subject mentions `clean_session`. M1 and M3 subjects (`re-subscribe on every successful ConnAck`, `log mqtt reconnects with a connection counter`) are present.
2. `git diff --name-only origin/release/v0.8...HEAD` lists only `agent/src/workers/mqtt.rs`, `agent/tests/workers/mqtt.rs`, and `plans/completed/20260507-mqtt-resubscribe-on-reconnect.md`.
3. `./scripts/test.sh` exits zero.
4. `./scripts/preflight.sh` reports `Preflight clean`. **This is the gate** — preflight must report clean before changes are pushed.
5. The plan doc no longer references the M2 milestone in Plan of Work, Concrete Steps, or Validation; the Decision Log includes the drop entry.

## Idempotence and Recovery

The rebase can be undone by `git reset --hard origin/fix/mqtt-resubscribe-on-reconnect`, which restores the pre-rebase state with M2 included. Plan-doc edits can be reverted with `git restore plans/completed/20260507-mqtt-resubscribe-on-reconnect.md`. If the rebase reports a conflict (it should not, given file disjointness), abort with `git rebase --abort` and report — that signals an unexpected coupling worth investigating before retrying.

## Non-goals

- Not modifying `fix/mqtt-resubscribe-on-reconnect-main` (PR #61). The same operation will be applied there as a follow-up.
- Not bumping the Cargo workspace version.
- Not opening a new PR — the existing PR #60 will be updated by the task orchestrator.
