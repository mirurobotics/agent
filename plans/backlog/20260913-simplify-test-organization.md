# Simplify Cargo test organization and CI

This ExecPlan is a living document. Update Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective during implementation.

## Scope


Read-write: `/home/ben/miru/workbench4/repos/agent` (the independent `mirurobotics/agent` checkout), limited to test organization, test-only support, coverage/lint wrappers, `.github/workflows/ci.yml`, current documentation, and this plan. Read-only: `/home/ben/miru/workbench4` for shared workflow instructions. The plan belongs in the agent repository because all implementation belongs there.

Continue branch `codex/remove-test-feature` and existing PR #239 against `main`. Starting HEAD is `61b9df8af9089852957e922ce8c6c202ab5c8ae4`; researched main is `a0e7afb33fd8964539e9e309f0c83943b14c5033`. Do not touch PR #237, `.agents/`, generated `libs/`, unrelated dependencies, or `plans/completed/20260913-remove-test-feature.md`. Do not merge.

## Purpose / Big Picture


Ordinary Cargo commands should discover conventional integration tests while tests needing private implementation details live beside their owners. Preserve production behavior, all 1,997 current test cases, and all 37 coverage thresholds. CI should exercise those cases once through the coverage gate and generate HTML from the same recorded execution, with Linux and Windows production checks retained.

## Progress


- [ ] Populate milestone progress during implementation.

## Surprises & Discoveries


Add entries with evidence as work proceeds.

## Decision Log


Add dated decisions and reasons as work proceeds.

## Outcomes & Retrospective


Complete after implementation and delivery validation.

## Context and Orientation


The agent is a Rust device service; this change concerns its test harness, not its runtime. A unit test is compiled inside the library and can reach private state. An integration test is a separate executable using the normal library. Currently `agent/src/lib.rs` mounts the whole `agent/tests/mod.rs` tree under `#[cfg(test)]`, aliases itself as `miru_agent`, and `agent/Cargo.toml` sets `autotests = false`. This makes public-behavior tests appear as library cases and requires `crate::tests::...` fixture paths.

The narrow private seams are in S3/GCS storage, `sync::syncer`, upload transfer, and one upload executor case. Existing inline tests elsewhere already have appropriate ownership. Shared helpers currently live in `agent/tests/test_utils/`, `agent/tests/mocks/`, and `agent/tests/sync/helpers.rs`; server tests also import `create_storage` and `create_token_manager` from `agent/tests/sync/syncer.rs`.

`scripts/lib/covgate.sh` uses coverage regions (instrumented executable source locations) under each `.covgate` directory. Moving tests under `agent/src/` can otherwise change those denominators. `scripts/lint.sh` already checks imports in both source and tests but checks assertion quality only in `agent/tests/`; the linter automatically exempts directories named `tests` from function-length limits. `.github/workflows/ci.yml` currently repeats ordinary tests three times, inventories four targets, and executes tests independently for HTML and gates.

## Plan of Work


Milestone 1 — prepare source and CI infrastructure. Restore Cargo discovery by removing `autotests = false` and the explicit logging target declarations from `agent/Cargo.toml`; retain the explicit `http_retry` target at `tests/http/retry.rs`. Remove the whole-suite mount from `agent/src/lib.rs`. Add private `#[cfg(test)]` module declarations for the destinations below and narrowly include only required shared fixtures. Prefer small path-based inclusions; do not create a fixture framework, new crate, wholesale mock mount, or hundreds of copied fixture lines to avoid an adapter. Remove the global alias if simple; otherwise explain the smallest necessary compatibility alias. Never widen production visibility or change production algorithms/client construction.

This workflow deliberately separates source preparation from test implementation. The source worker may prepare declarations pointing at pending files, plus wrapper/CI/documentation edits, but must not move test suites or rewrite test bodies/imports. Temporary compilation failure between the two local stage commits is expected; do not push either stage alone. Source review must recognize the explicitly pending test work. A fresh test-analysis worker then resolves the exact fixture/module layout before the test worker performs all relocations, fixture extraction, import migration, and test rewrites.

Configure one optional coverage filename-exclusion setting, defaulting empty for other wrapper users and set by the agent wrappers to `/agent/src/(.*/)?tests/`. Apply precisely the same exclusion to JSON gates and HTML, adding exact support-file paths only if executable fixtures require it. Share this setting rather than duplicating the regex in CI. Add a small `./scripts/coverage.sh --report-only` path running `cargo llvm-cov report --html --output-dir target/coverage --package miru-agent` with that exclusion, without rerunning tests; the ordinary no-argument coverage/test wrappers remain independently usable. Update only the needed `scripts/coverage.sh`, `scripts/covgate.sh`, and `scripts/lib/{coverage,covgate}.sh` plumbing.

In `.github/workflows/ci.yml`, keep `cargo check --package miru-agent --locked` and the Windows check unchanged. Replace repeated agent runs/inventories with `env -u RUST_LOG ./scripts/covgate.sh`, report-only HTML, and `test -f target/coverage/html/index.html`; leave the separate tools job intact. Reduce the agent test timeout from 45 to 30 minutes, revisiting only if measured CI runtime requires it. Extend `ASSERT_LINT_PATHS` in `scripts/lint.sh` to the five new unit-test directories below, not every existing inline test. Update `AGENTS.md` and `ARCHITECTURE.md` to describe the resulting layout and ordinary commands accurately.

Milestone 2 — migrate and simplify tests. Move exactly the following 101 cases, keeping their names, assertions, serial-resource guards, and ordinary production dependencies. Use the table's intended Rust module names; record any necessary mapping adjustment before inventory comparison.

| Existing location under `agent/tests/` | Destination under `agent/src/` | Cases | New library prefix |
| --- | --- | ---: | --- |
| `s3/{mod,multipart}.rs` | `s3/tests/{mod,multipart}.rs` | 39 | `s3::tests::` |
| `gcs/mod.rs` | `gcs/tests/store.rs`, declared inside existing inline GCS `tests` | 27 | `gcs::tests::store::` |
| `sync/syncer.rs` | `sync/syncer/tests/mod.rs` | 22 | `sync::syncer::tests::` |
| `data_uploads/upload/transfer.rs` | `data_uploads/upload/transfer/tests/mod.rs` | 12 | `data_uploads::upload::transfer::tests::` |
| Executor's `end_to_end_with_sdk_transfer_over_replayed_s3` only | `data_uploads/upload/executor/tests/mod.rs` | 1 | `data_uploads::upload::executor::tests::` |

Keep the other 13 executor cases external. Remove moved module declarations from the integration tree; `agent/tests/http/mod.rs` must continue excluding `retry`. Convert remaining integration fixture imports from `crate::tests::...` to the integration crate's direct modules. Adapt source-inline test imports such as `crate::tests::test_utils` to the selected fixture-only module. Extract `create_storage`/`create_token_manager` into `agent/tests/test_utils/` before moving syncer tests, and update external `server/handlers.rs` and `server/sse.rs` consumers. Share `sync/helpers.rs` with external `sync/deployments.rs` without mounting suites. Include only needed filesystem, HTTP client/server, token-manager, and sync fixtures; unused mounts must not introduce denied warnings.

Keep `agent/tests/app/state.rs` external. In `init::scanner_spawned` and `scanner_degrades_when_snapshot_path_unwritable`, replace the two liveness-only `get_rules().await.unwrap()` calls with public `ScannerExt::scan().await.unwrap()` on the empty scanner; preserve snapshot and shutdown assertions. Leave existing inline scanner/state/rule tests in place.

In `agent/tests/http/retry.rs`, retain all six case names. Directly await `with_retry` in paused Tokio tests, recording `Instant::now()` on every attempt in a `RefCell<Vec<Instant>>` or mutex. Assert an immediate first attempt, each subsequent gap in 500..=1,000 ms, completion at the final attempt, exact counts, and the same success/error classifications. Tokio 1.53.1 rounds timer deadlines to milliseconds; nominal jitter remains 500..999 ms. Remove `poll_without_advancing`, its 128 yields, manual `advance`, pinning, and polling imports. Preserve the existing pure `retry_delay_from_nanos_respects_jitter_boundaries` case and its 0→500, 499→999, 500→500 assertions; production retry code needs no change.

Optionally remove only the tangential new sleep-history assertions in `agent/tests/workers/scan.rs` and now-unused `SleepController::{get_attempted_sleeps,get_completed_sleeps}` in `agent/tests/mocks/error.rs`. Retain the used `get_last_*` methods and worker behavior assertions. Consider token-manager `get_calls` similarly only after confirming its remaining uses; do not delete cases or weaken their original purpose.

Milestone 3 — review, CI, and delivery. Use fresh leaf workers for review/test analysis as required by the enclosing workflow; the root owns git operations. Delivery mode is push to PR #239, with one task budget of three CI rounds, all initially unused. Keep the PR draft until preflight is CLEAN for the pushed exact commit. Existing PR synchronization triggers CI; `workflow_dispatch` is unsupported. Preserve complete identity-comparison and coverage-selection evidence in this plan, then complete it and push the documentation commit. That final head also needs passing delivery checks before marking the PR ready. No merge is authorized.

## Concrete Steps


All local commands below run from `/home/ben/miru/workbench4/repos/agent`; CI commands run from that repository's checked-out root. Local work is limited to inspection, edits, and git operations: no builds, tests, lint, dependency refresh, or coverage runs. The root stages only reviewed task changes and creates each commit from this repository, preserving unrelated work.

Before implementation, inspect `git status --short`, `git rev-parse HEAD`, and `gh pr view 239 --json headRefName,headRefOid,isDraft,baseRefName`. Obtain baseline evidence with `gh api repos/mirurobotics/agent/actions/jobs/103782713868/logs` (run `34779109472` at the starting HEAD). Its inventory is 1,989 library cases, six `http_retry` cases, and one case in each logging target: 1,997 total.

For milestone 1, promote the approved plan using `mkdir -p plans/active` and `git mv plans/backlog/20260913-simplify-test-organization.md plans/active/20260913-simplify-test-organization.md`, then perform the source-stage edits. Inspect `git diff --stat` and `git diff --check`; expected result is only intended files and no whitespace errors. Root stages the reviewed paths, runs `git diff --cached --check`, and explicitly commits with `git commit -m "refactor(test): prepare conventional test targets and shared coverage reporting"`. Do not push yet.

For milestone 2, complete fresh test analysis and the listed migration/retry edits, then review all changed test identities and module declarations. Inspect `git diff --check` and `git diff --stat`, stage only reviewed paths, and explicitly commit with `git commit -m "test: relocate private suites and simplify retry timing assertions"`. The complete source-plus-test result is now eligible for CI.

For milestone 3, ensure PR #239 is draft (`gh pr ready 239 --undo` only if it is currently ready). Start every preflight CI round with a fresh-context `$refine` pass; the root commits any reviewed fixes, then runs `git fetch origin main` followed by `git rebase origin/main` and resolves any conflicts within the task's scope. The root pushes exactly once per round: normally `git push origin HEAD:codex/remove-test-feature`, or `git push --force-with-lease origin HEAD:codex/remove-test-feature` when the rebase requires rewriting the remote branch. Record `git rev-parse HEAD`; inspect `gh run list --workflow ci.yml --branch codex/remove-test-feature --limit 10 --json databaseId,headSha,status,conclusion,event` and select the run matching that exact SHA. Use `gh run view RUN_ID --json headSha,status,conclusion,jobs` and `gh run view RUN_ID --log-failed` for progress/failures, substituting the observed run ID. Record every consumed CI round. If a round fails and budget remains, diagnose all failing jobs from their logs, apply scoped fixes in one local batch, and have the root commit them; return to fresh `$refine` before the next publish. Do not use local heavyweight checks to bypass CI.

The agent CI test step executes the following once, in order; report-only mode must preserve the common exclusion setting:

    env -u RUST_LOG ./scripts/covgate.sh
    env -u RUST_LOG ./scripts/coverage.sh --report-only
    test -f target/coverage/html/index.html

Expected results: 1,997 combined cases pass; 37 module gates meet unchanged thresholds; HTML exists. Lint, Windows production, Linux production, and the tools job must also pass. Capture runtime identities with `gh run view RUN_ID --log`; inspect the report-only JSON file-selection list from the same profiles in CI if needed, without another test execution.

After preflight is CLEAN, fill living sections, record the checked SHA/run and inventory proof, and use `mkdir -p plans/completed` followed by `git mv plans/active/20260913-simplify-test-organization.md plans/completed/20260913-simplify-test-organization.md`. Stage that plan and any reviewed completion documentation, run `git diff --cached --check`, and explicitly commit with `git commit -m "docs: complete test organization simplification plan"`. Push, select the new exact-head CI run as above, and require green delivery checks. Only after the new head is green, run `gh pr ready 239`; `gh pr view 239 --json headRefOid,isDraft,statusCheckRollup` must show the delivered SHA, `isDraft: false`, and passing checks. If delivery checks fail, stop immediately with `CAPPED` regardless of unused preflight rounds: record the failed SHA/run, leave PR #239 draft, and report all failing jobs and incomplete delivery; do not make another repair push.

## Validation and Acceptance


Compare complete `(target, runtime test name)` identities from the baseline and final canonical run, not just totals. Preserve all original inline library identities. For broad-suite cases left external, map `(lib, tests::X)` to `(mod, X)`. For each moved group, map its old `tests::s3::`, `tests::gcs::`, `tests::sync::syncer::`, or `tests::data_uploads::upload::transfer::` prefix to the table's library prefix, preserving the suffix. Map the single executor case explicitly; all other executor cases use the external mapping. Retry/logging target identities remain unchanged. Require exactly 1,997 cases, zero lost identities, zero duplicates, and zero newly ignored tests. Reconcile interleaved log lines against source and adjacent result markers before treating names as missing; if necessary use one temporary aggregate `cargo test --package miru-agent -- --list` migration step in CI, then remove it before final delivery.

Retry cases `success_on_first_attempt`, `retries_on_network_error_then_succeeds`, `no_retry_on_app_error`, `exhausts_retries_on_persistent_network_error`, `network_error_then_app_error_stops_immediately`, and `recovers_on_last_attempt` must make respectively 1, 3, 1, 3, 2, and 3 attempts. The first and third complete immediately; retrying cases observe only allowed gaps and no sleep after their final result. This checks preserved behavior while removing scheduler-polling machinery.

Audit coverage JSON filenames against the unchanged production file set under `agent/src/`: exclude only new test directories and any explicitly justified executable fixture file, never whole production modules. Confirm JSON and HTML use the same filter and all 37 original `.covgate` files/values remain unchanged. Conventional `cargo test`, `cargo test --package miru-agent`, `./scripts/test.sh`, and `./scripts/coverage.sh` remain supported without a custom feature or pre-set logging environment; inspect discovery and wrapper arguments and use the canonical CI execution as verification rather than adding permanent duplicate runs.

## Idempotence and Recovery


Inspect the current plan path and git status before repeating moves or commits; skip already-completed operations. Keep intermediate stage commits local until migration is complete. Fix preflight CI failures with scoped follow-up commits while budget remains; do not reset user work, use an unconditional force-push, lower gates, disable tests, refresh unrelated dependencies, or change production behavior to satisfy this refactor. Use `--force-with-lease` when the prescribed rebase requires it. A moved test must retain its prior assertions and mapped identity. If logs are ambiguous, preserve the evidence and resolve it using the bounded inventory fallback.

If all three preflight CI rounds are exhausted with failures, or final delivery checks fail regardless of unused rounds, stop with `CAPPED`, leave PR #239 draft, and report the exact failing SHA/run, all failing jobs, and remaining work; do not claim completion or merge. A passing earlier head does not validate later source or documentation commits. The final response must identify the delivered SHA and its passing checks, or state precisely why delivery remains incomplete.
