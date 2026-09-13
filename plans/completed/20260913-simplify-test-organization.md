# Simplify Cargo test organization and CI

This ExecPlan is a living document. Update Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective during implementation.

## Scope


Read-write: `/home/ben/miru/workbench4/repos/agent` (the independent `mirurobotics/agent` checkout), limited to test organization, test-only support, coverage/lint wrappers, `.github/workflows/ci.yml`, current documentation, and this plan. Read-only: `/home/ben/miru/workbench4` for shared workflow instructions. The plan belongs in the agent repository because all implementation belongs there.

Continue branch `codex/remove-test-feature` and existing PR #239 against `main`. Starting HEAD is `61b9df8af9089852957e922ce8c6c202ab5c8ae4`; researched main is `a0e7afb33fd8964539e9e309f0c83943b14c5033`. Do not touch PR #237, `.agents/`, generated `libs/`, unrelated dependencies, or `plans/completed/20260913-remove-test-feature.md`. Do not merge.

## Purpose / Big Picture


Ordinary Cargo commands should discover conventional integration tests while tests needing private implementation details live beside their owners. Preserve production behavior, all 1,997 current test cases, and all 37 coverage thresholds. CI should exercise those cases once through the coverage gate and generate HTML from the same recorded execution, with Linux and Windows production checks retained.

## Progress


- [x] 2026-09-13: Complete source preparation (`2832969`), test migration (`28fd942`), and coverage simplification (`675882d`). Restore ordinary Cargo discovery, relocate 101 private cases, share only required fixtures, simplify all six retry cases, reuse coverage profiles for HTML, and update current documentation.
- [x] 2026-09-13: Complete static checks and source comparisons: changed-file formatting, individual shell syntax checks, and `git diff --check` passed. Moved cases, the other 13 external executor cases, nine extracted factory bodies, six retry names, and all 37 `.covgate` files are preserved. No local build, test, lint, or coverage run was performed.
- [x] 2026-09-13: Diagnose round 1 (`34782193470`) from complete failed-job logs and repair filesystem fixture child resolution plus 17 import-anchor violations in `5c3a77f`. Linux/Windows production checks and tools had passed.
- [x] 2026-09-13: Diagnose round 2 (`34782521058`): only lint failed, with 65 `private_bounds` reports for the shared `TempDir` fixture. Repair its test-only type visibility in `85b9dc6`. All tests, gates, HTML, production checks, and tools had passed; the complete mapped inventory already matched baseline.
- [x] 2026-09-13: Complete fresh reviews and round 3 (`34782780878`) at exact code head `85b9dc61d03beec66a88f9961496209730b77839`. All four CI jobs passed; preflight is CLEAN with all three rounds used. The canonical test log confirms every mapped baseline identity, all 37 gates, one agent test execution, and report-only HTML; evidence is recorded under Validation and Acceptance.
- [x] 2026-09-13: Complete coverage-selection audit using actual source paths, wrapper arguments, and cargo-llvm-cov 0.8.4's default filter. All new test directories and shared fixtures are excluded; production paths and thresholds are unchanged. No coverage JSON filename artifact was downloaded or inspected.
- [ ] Final delivery: root moves this plan to `plans/completed/`, commits and pushes the completion documentation, synchronizes the PR description, and validates CI for that new exact head before marking PR #239 ready. No further code work is pending.

## Surprises & Discoveries


- 2026-09-13: Shared filesystem fixtures and other test helpers already import production APIs through `miru_agent`. The two-line `#[cfg(test)] extern crate self as miru_agent` alias preserves those imports for both compilation contexts without mounting any test suites; the unit adapter includes only the required fixture leaves.
- 2026-09-13: The existing GCS inline `tests` module owns four credential-provider cases. Adding its `mod store;` declaration preserves their identities while reserving `gcs::tests::store::` for the 27 relocated cases.
- 2026-09-13: The shared filesystem fixture leaves were the exception to the existing `miru_agent` import convention: they still used `crate::filesys` and `crate::trace`. Both now use the production crate name so they resolve correctly in library and integration test builds. After relocation, `TempFile::to_file` has callers only in library tests; make the fixture type and this method public within the test-only namespace so the external fixture build does not report dead code. Fixture bodies and production visibility are unchanged.
- 2026-09-13: Installed cargo-llvm-cov 0.8.4 already excludes nested workspace `tests/` directories with `^workspace_root(/.*)?/(tests|examples|benches)/`, covering both owner-local tests and `agent/tests/` support files. Custom regexes augment this default; neither wrapper uses `--dep-coverage` or `--no-default-ignore-filename-regex`. The removed regex was redundant, not a coverage bug. See [upstream file-selection implementation](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.8.4/src/report.rs#L766-L817).
- 2026-09-13: CI round 1 exposed that the explicit `#[path]` mount of the filesystem fixture file made Rust look for `dirs.rs` and `files.rs` beside that file, outside their existing directory. A conventional `filesys/mod.rs` keeps child resolution inside `filesys/` for both the integration declaration and unit adapter. The import linter also requires one statement per top-level `crate` anchor; preserve nested groups within each anchor, including the shared `test_utils` group in SSE tests.
- 2026-09-13: CI round 2 reached the external cache helpers and denied `private_bounds`: their public future output bounds name the crate-private `test_utils::filesys::dirs::TempDir`. Its visibility must match that existing helper API. The shared fixture is mounted by the integration test target and the library's `#[cfg(test)]` adapter only; making this fixture type public adds no ordinary production API.

## Decision Log


- 2026-09-13: Retain the existing two-line test-only self alias as the smallest compatibility adapter for shared fixture imports; remove the entire `../tests/mod.rs` library mount. Test-stage analysis selected only the required fixture leaves.
- 2026-09-13: Superseded the initial `COV_IGNORE_FILENAME_REGEX` setting and `/agent/src/(.*/)?tests/` wrapper exports: rely on cargo-llvm-cov's default file selection for JSON and HTML, with no replacement configuration.
- 2026-09-13: Report-only mode invokes `cargo llvm-cov report`, retains package and feature selection plus default file filtering, and omits test-harness arguments. Ordinary coverage still runs tests. CI now executes the gate once followed by report-only HTML and an index check, with a 30-minute timeout; Linux/Windows production checks and the tools job are unchanged.
- 2026-09-13: Use `#[cfg(test)] #[path = "../tests/test_utils/unit.rs"] pub mod test_utils` with explicit paths for all seven fixture leaves: filesystem helpers at `filesys/mod.rs`, HTTP client/server mock, token-manager mock, error harness, sync assertion helpers, sync factories, and upload factories. The public fixture namespace exists only in library test builds, allowing reusable public fixture helpers without dead-code suppressions. Production inspection seams remain private or `pub(crate)`; no test suite, testdata tree, or wholesale mock tree is mounted.
- 2026-09-13: Keep all five relocation prefixes exactly as planned. Complete canonical CI logs for rounds 2 and 3 confirm 456 library cases (355 original inline plus 101 moved), 1,533 `mod` integration cases, six `http_retry` cases, and two separate logging cases: 1,997 total, with no identity loss or additions.
- 2026-09-13: Retry tests record every attempt in `RefCell<Vec<Instant>>`, directly await ordinary `with_retry`, and assert immediate first attempt, the prescribed exact count/outcome, 500..=1,000 ms between attempts, and completion at the last attempt. Preserve the production jitter function and its existing 0/499/500 boundary case. Remove only the two tangential scanner sleep-history assertions and their unused vector accessors; keep scanner/shutdown behavior assertions, the last-sleep accessors, and token-refresh call ordering.
- 2026-09-13: Fix round 2 with the single `TempDir` fixture declaration, from `pub(crate)` to `pub`, consistent with the public test-fixture namespace and `TempFile`. Keep its fields and methods unchanged; no cache helper rewrite, lint suppression, or production visibility change is needed.
- 2026-09-13: Accept coverage-selection proof from unchanged production paths and gates, inspection of actual test locations and the shared default filter, and passing runtime gates/HTML. This is a static selection audit supported by CI, not an inspection of a downloaded JSON filename list; no additional diagnostic CI framework or inventory execution is needed.

## Outcomes & Retrospective


Implementation, fresh reviews, and preflight are complete at `85b9dc61d03beec66a88f9961496209730b77839`; round 3 is fully green and all three preflight rounds are used. Ordinary Cargo integration discovery is restored, 101 private cases live beside their owners, and the library exposes only a small fixture namespace and compatibility alias in test builds. All six paused-time retry cases now await production retry directly and assert recorded attempt times; the 128-poll helper is gone. Production retry and S3/GCS credential behavior, all 1,997 cases, and all 37 thresholds are preserved. CI executes agent tests once, reuses their profiles for HTML, and retains Linux/Windows production checks.

The two CI repairs showed that shared fixtures need conventional child-module paths and visibility consistent with their existing public test-helper APIs. Both repairs remained within test support. Default coverage filtering already handles owner-local test directories, so custom exclusion plumbing was unnecessary.

Final delivery remains: the root must move and commit this completion record, push it, synchronize the PR description, and require passing CI for that new exact head before marking PR #239 ready. The documentation delivery head does not exist yet; this record does not claim its checks have passed or that the PR is ready or merged.

## Context and Orientation


The agent is a Rust device service; this change concerns its test harness, not its runtime. A unit test is compiled inside the library and can reach private state. An integration test is a separate executable using the normal library. At the starting HEAD, `agent/src/lib.rs` mounted the whole `agent/tests/mod.rs` tree under `#[cfg(test)]`, aliased itself as `miru_agent`, and `agent/Cargo.toml` set `autotests = false`. Public-behavior tests therefore appeared as library cases. The implemented layout restores Cargo discovery and mounts only `agent/tests/test_utils/unit.rs` for library-test fixtures; the compatibility alias also remains test-only.

The narrow private seams are in S3/GCS storage, `sync::syncer`, upload transfer, and one upload executor case. Their 101 cases now live under their owners' `tests/` directories; existing inline tests remain in place. Shared helpers live in `agent/tests/test_utils/`, `agent/tests/mocks/`, and `agent/tests/sync/helpers.rs`. Sync and upload factories were extracted into `agent/tests/test_utils/` so external and owner-local cases share fixtures without mounting suites.

`scripts/lib/covgate.sh` uses coverage regions (instrumented executable source locations) under each `.covgate` directory; default filename filtering excludes the relocated test directories. `scripts/lint.sh` now checks assertion quality in those directories as well as `agent/tests/`; import checks cover both source and tests, and the linter exempts directories named `tests` from function-length limits. `.github/workflows/ci.yml` now replaces repeated ordinary runs, inventories, and independent HTML/gate executions with one canonical coverage run followed by report-only HTML.

## Plan of Work


Milestone 1 — prepare source and CI infrastructure. Restore Cargo discovery by removing `autotests = false` and the explicit logging target declarations from `agent/Cargo.toml`; retain the explicit `http_retry` target at `tests/http/retry.rs`. Remove the whole-suite mount from `agent/src/lib.rs`. Add private `#[cfg(test)]` module declarations for the destinations below and narrowly include only required shared fixtures. Prefer small path-based inclusions; do not create a fixture framework, new crate, wholesale mock mount, or hundreds of copied fixture lines to avoid an adapter. Remove the global alias if simple; otherwise explain the smallest necessary compatibility alias. Never widen production visibility or change production algorithms/client construction.

This workflow deliberately separates source preparation from test implementation. The source worker may prepare declarations pointing at pending files, plus wrapper/CI/documentation edits, but must not move test suites or rewrite test bodies/imports. Temporary compilation failure between the two local stage commits is expected; do not push either stage alone. Source review must recognize the explicitly pending test work. A fresh test-analysis worker then resolves the exact fixture/module layout before the test worker performs all relocations, fixture extraction, import migration, and test rewrites.

Rely on cargo-llvm-cov's default filename filtering for JSON gates and HTML. Add a small `./scripts/coverage.sh --report-only` path running `cargo llvm-cov report --html --output-dir target/coverage --package miru-agent`, without rerunning tests; the ordinary no-argument coverage/test wrappers remain independently usable. No custom exclusion setting is needed.

In `.github/workflows/ci.yml`, keep `cargo check --package miru-agent --locked` and the Windows check unchanged. Replace repeated agent runs/inventories with `env -u RUST_LOG ./scripts/covgate.sh`, report-only HTML, and `test -f target/coverage/html/index.html`; leave the separate tools job intact. Reduce the agent test timeout from 45 to 30 minutes, revisiting only if measured CI runtime requires it. Extend `ASSERT_LINT_PATHS` in `scripts/lint.sh` to the five new unit-test directories below, not every existing inline test. Update `README.md`, `AGENTS.md`, and `ARCHITECTURE.md` to describe the resulting layout and ordinary commands accurately.

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

Milestone 3 — review, CI, and delivery. Fresh leaf reviews and all three preflight CI rounds are complete; the root owns remaining git and PR operations. Delivery mode is push to PR #239. Keep the PR draft through exact-head delivery validation. Existing PR synchronization triggers CI; `workflow_dispatch` is unsupported. Preserve identity-comparison and coverage-selection evidence in this plan, then move it to completed and push the documentation commit. That final head also needs passing delivery checks before marking the PR ready. No merge is authorized.

## Concrete Steps


All local commands below run from `/home/ben/miru/workbench4/repos/agent`; CI commands run from that repository's checked-out root. Local work is limited to inspection, edits, and git operations: no builds, tests, lint, dependency refresh, or coverage runs. The root stages only reviewed task changes and creates each commit from this repository, preserving unrelated work.

Before implementation, inspect `git status --short`, `git rev-parse HEAD`, and `gh pr view 239 --json headRefName,headRefOid,isDraft,baseRefName`. Obtain baseline evidence with `gh api repos/mirurobotics/agent/actions/jobs/103782713868/logs` (run `34779109472` at the starting HEAD). Its inventory is 1,989 library cases, six `http_retry` cases, and one case in each logging target: 1,997 total.

For milestone 1, promote the approved plan using `mkdir -p plans/active` and `git mv plans/backlog/20260913-simplify-test-organization.md plans/active/20260913-simplify-test-organization.md`, then perform the source-stage edits. Inspect `git diff --stat` and `git diff --check`; expected result is only intended files and no whitespace errors. Root stages the reviewed paths, runs `git diff --cached --check`, and explicitly commits with `git commit -m "refactor(test): prepare conventional test targets and shared coverage reporting"`. Do not push yet.

For milestone 2, complete fresh test analysis and the listed migration/retry edits, then review all changed test identities and module declarations. Inspect `git diff --check` and `git diff --stat`, stage only reviewed paths, and explicitly commit with `git commit -m "test: relocate private suites and simplify retry timing assertions"`. The complete source-plus-test result is now eligible for CI.

For milestone 3, ensure PR #239 is draft (`gh pr ready 239 --undo` only if it is currently ready). Start every preflight CI round with a fresh-context `$refine` pass; the root commits any reviewed fixes, then runs `git fetch origin main` followed by `git rebase origin/main` and resolves any conflicts within the task's scope. The root pushes exactly once per round: normally `git push origin HEAD:codex/remove-test-feature`, or `git push --force-with-lease origin HEAD:codex/remove-test-feature` when the rebase requires rewriting the remote branch. Record `git rev-parse HEAD`; inspect `gh run list --workflow ci.yml --branch codex/remove-test-feature --limit 10 --json databaseId,headSha,status,conclusion,event` and select the run matching that exact SHA. Use `gh run view RUN_ID --json headSha,status,conclusion,jobs` and `gh run view RUN_ID --log-failed` for progress/failures, substituting the observed run ID. Record every consumed CI round. If a round fails and budget remains, diagnose all failing jobs from their logs, apply scoped fixes in one local batch, and have the root commit them; return to fresh `$refine` before the next publish. Do not use local heavyweight checks to bypass CI.

The agent CI test step executes the following once, in order; both reports use default filename filtering:

    env -u RUST_LOG ./scripts/covgate.sh
    env -u RUST_LOG ./scripts/coverage.sh --report-only
    test -f target/coverage/html/index.html

Expected results: 1,997 combined cases pass; 37 module gates meet unchanged thresholds; HTML exists. Lint, Windows production, Linux production, and the tools job must also pass. Capture complete runtime identities with `gh api repos/mirurobotics/agent/actions/jobs/JOB_ID/logs`, substituting the selected run's primary test job ID. Audit coverage selection against actual source paths and the installed tool's default filter without another test execution; inspect a report-only JSON filename artifact only if available and needed to resolve uncertainty.

After preflight is CLEAN, fill living sections, record the checked SHA/run and inventory proof, and use `mkdir -p plans/completed` followed by `git mv plans/active/20260913-simplify-test-organization.md plans/completed/20260913-simplify-test-organization.md`. Stage that plan and any reviewed completion documentation, run `git diff --cached --check`, and explicitly commit with `git commit -m "docs: complete test organization simplification plan"`. Push, select the new exact-head CI run as above, and require green delivery checks. Only after the new head is green, run `gh pr ready 239`; `gh pr view 239 --json headRefOid,isDraft,statusCheckRollup` must show the delivered SHA, `isDraft: false`, and passing checks. If delivery checks fail, stop immediately with `CAPPED` regardless of unused preflight rounds: record the failed SHA/run, leave PR #239 draft, and report all failing jobs and incomplete delivery; do not make another repair push.

## Validation and Acceptance


Verified code head: `85b9dc61d03beec66a88f9961496209730b77839`, CI run `34782780878`. All four jobs passed: test `103792697075` (1m46s), lint `103792697010` (51s), Windows `103792696760` (1m42s), and tools `103792696966` (50s). The Linux production check passed inside the test job. Final documentation delivery will create a later head that requires its own green run.

The complete primary test log was compared with baseline job `103782713868`, run `34779109472`, at `61b9df8af9089852957e922ce8c6c202ab5c8ae4`. Baseline contained 1,997 unique passing cases: 1,989 library (355 inline plus 1,634 broad-suite), six retry, and two logging. Applying the mappings below yields exactly the final 1,997 unique passing identities: 456 library, 1,533 external `mod`, six `http_retry`, one `logs_init_locked`, and one `logs_init_smoke`. All 355 original inline identities remain, and the 101 relocated identities differ only by their planned prefixes. There are zero missing, extra, duplicate, or ignored cases. Round 2's complete primary test log independently produced the same result; no temporary inventory step was needed.

Compare complete `(target, runtime test name)` identities from the baseline and final canonical run, not just totals. Preserve all original inline library identities. For broad-suite cases left external, map `(lib, tests::X)` to `(mod, X)`. For each moved group, map its old `tests::s3::`, `tests::gcs::`, `tests::sync::syncer::`, or `tests::data_uploads::upload::transfer::` prefix to the table's library prefix, preserving the suffix. Map the single executor case explicitly; all other executor cases use the external mapping. Retry/logging target identities remain unchanged. Require exactly 1,997 cases, zero lost identities, zero duplicates, and zero newly ignored tests. Reconcile interleaved log lines against source and adjacent result markers before treating names as missing; if necessary use one temporary aggregate `cargo test --package miru-agent -- --list` migration step in CI, then remove it before final delivery.

Retry cases `success_on_first_attempt`, `retries_on_network_error_then_succeeds`, `no_retry_on_app_error`, `exhausts_retries_on_persistent_network_error`, `network_error_then_app_error_stops_immediately`, and `recovers_on_last_attempt` must make respectively 1, 3, 1, 3, 2, and 3 attempts. The first and third complete immediately; retrying cases observe only allowed gaps and no sleep after their final result. This checks preserved behavior while removing scheduler-polling machinery.

Coverage acceptance is supported by a static audit of actual source locations and cargo-llvm-cov 0.8.4's default filter, plus canonical CI results. Its nested `tests/` exclusion covers all five owner-local test directories and `agent/tests/` support files. Production paths are unchanged, both wrappers retain the same default filter, and comparison against the baseline shows no changes to any of the 37 `.covgate` files or values. The primary test job reports all 37 gates passing, exactly one agent test execution, HTML saved under `target/coverage/html`, and a passing HTML index check. No JSON filename artifact was downloaded or inspected; the audit does not claim runtime filename-list inspection.

Conventional `cargo test`, `cargo test --package miru-agent`, `./scripts/test.sh`, and `./scripts/coverage.sh` remain supported without a custom feature or pre-set logging environment. Discovery and wrapper argument inspection plus the canonical CI execution verify this layout without permanent duplicate runs.

## Idempotence and Recovery


Inspect the current plan path and git status before repeating moves or commits; skip already-completed operations. Keep intermediate stage commits local until migration is complete. Fix preflight CI failures with scoped follow-up commits while budget remains; do not reset user work, use an unconditional force-push, lower gates, disable tests, refresh unrelated dependencies, or change production behavior to satisfy this refactor. Use `--force-with-lease` when the prescribed rebase requires it. A moved test must retain its prior assertions and mapped identity. If logs are ambiguous, preserve the evidence and resolve it using the bounded inventory fallback.

If all three preflight CI rounds are exhausted with failures, or final delivery checks fail regardless of unused rounds, stop with `CAPPED`, leave PR #239 draft, and report the exact failing SHA/run, all failing jobs, and remaining work; do not claim completion or merge. A passing earlier head does not validate later source or documentation commits. The final response must identify the delivered SHA and its passing checks, or state precisely why delivery remains incomplete.

Revision note (2026-09-13): Record completed implementation, fresh review, and CLEAN preflight with the exact passing head and full inventory proof. Clarify the static coverage-selection evidence and retain the final documentation commit, exact-head CI recheck, and PR readiness as pending delivery work.
