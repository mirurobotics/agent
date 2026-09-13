# Remove the custom Cargo test feature


This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective up to date during implementation.

## Scope


| Repository | Access | Work |
| --- | --- | --- |
| `/home/ben/miru/workbench4/repos/agent` | Read-write | Test organization, test support, dependency injection, retry tests, wrappers, CI, and current documentation. |
| `/home/ben/miru/workbench4` | Read-only | Workspace instructions and shared skill policies. This coordinator and the agent are independent Git checkouts, not submodules. |

This plan belongs in the agent repository because that repository owns every implementation change. It was authored on `codex/remove-test-feature`, based on `main` at `a0e7afb33fd8964539e9e309f0c83943b14c5033`. Before implementation, verify the latest `origin/main` and use that base for a separate cleanup PR. Do not merge, cherry-pick, or copy the Windows test-portability edits from PR #237. Preserve the Windows production compile check already on main from PR #234; Windows test portability remains separate. Do not modify generated `libs/backend-api/`, `libs/device-api/`, `.agents/`, or coverage thresholds.

Keep this document in `plans/backlog/` during authoring. When implementation is approved, move it to `plans/active/20260913-remove-test-feature.md`. The final documentation commit provisionally moves it to `plans/completed/`; completion takes effect only after all acceptance conditions, including CI on that commit's published head, pass.

## Purpose / Big Picture


Developers can run ordinary `cargo test` and `cargo test --package miru-agent` successfully without a custom feature or preconfigured logging environment. The repository's test and coverage wrappers exercise the same retry and cloud-client behavior as production, with tests controlling time and external dependencies. Existing substantive tests and per-module `.covgate` safeguards remain effective.

## Progress


- [x] Capture baseline evidence and migrate the private-helper suite and fixtures.
- [x] Unify retry and cloud construction, remove the Cargo feature, and add focused timing/arithmetic and signing regressions (CI verification pending).
- [x] Update wrappers, CI, and operative documentation (execution evidence pending).
- [ ] Complete inventory comparison, refinement, and preflight on the pushed final commit.

## Surprises & Discoveries


- 2026-09-13: The successful base CI run is [34647851771](https://github.com/mirurobotics/agent/actions/runs/34647851771), at unchanged `origin/main` SHA `a0e7afb33fd8964539e9e309f0c83943b14c5033`. The jobs logs API supplied the complete execution inventory for test job `103422810288` when `gh run view --log` returned empty output. Evidence is retained locally in `/tmp/agent-test-feature-evidence.HAKWKx/baseline.json` and `baseline-test.log` in that directory: 354 library tests, 1,640 `mod` integration tests, and one case in each logging target, totaling 1,996 with no ignored cases or duplicates.
- 2026-09-13: Both filesystem guard tests can stay in their original source-inline modules by importing the relocated support functions. Their target and names remain unchanged; the six retry cases alone leave the broad suite for the `http_retry` integration target.
- 2026-09-13: Advancing paused Tokio time does not guarantee that an expired sleep completes on its first subsequent poll. The isolated retry target uses bounded runnable rescheduling and non-idling polls, checking that the clock stays frozen while ready timer work is processed before each timing assertion.

## Decision Log


- 2026-09-13: Keep test fixtures beneath the private library `tests` module and make the new filesystem guard types and helpers crate-private. Existing production imports retain their API paths through the test-only self alias; suite and macro harness references explicitly use `crate::tests` or `$crate::tests`.
- 2026-09-13: Use one ordinary S3 builder with caller credentials, optional HTTP transport, and a path-style setting. Default store and transfer construction keep the normal HTTPS connector, endpoint resolution, and `force_path_style = false`; replay constructors set transport and path style as data. GCS transfer construction calls its existing shared builder with an optional endpoint, retaining token validation, retry limits, and timeouts.
- 2026-09-13: Retain both logging integration processes and configure `RUST_LOG` inside each before subscriber work. Increase the Linux test-job timeout from 20 to 45 minutes because that job now executes both ordinary Cargo invocations, the test wrapper, and both coverage entry points while reusing cached builds.
- 2026-09-13: Strengthen the existing six public retry cases with paused time only in their separate executable. Check both retry windows at 499/1,000 and 1,499/2,000 ms, immediate success/application failures without elapsed time, and the mixed network/application stop at 1,000 ms. Add only one inline table-driven arithmetic case. Assert delimiter-qualified signing credentials (`Credential=access-key/` and `Credential=AKIA_TEST/`) plus exact session tokens in existing S3 replay cases, preserving the replay header exclusions and all existing fixture, logging, scanner/sync, S3, and GCS coverage.

## Outcomes & Retrospective


The source, fixture, wrapper, CI, and operative documentation changes are prepared. File-focused rustfmt parsing/formatting, shell syntax checks, and `git diff --check` passed during the source implementation step. Its static preservation check across changed Rust files found 2,606 assertion macro invocations and 1,339 test attributes both before and after that source-only migration; this is an editing safeguard, not a replacement for the runtime inventory comparison.

The test implementation strengthens the six existing retry cases and two existing S3 replay cases and adds the inline `http::retry::tests::retry_delay_from_nanos_respects_jitter_boundaries` case. Expected equivalent-Linux inventory is 1,989 library cases (`354 + 1,640 - 6 + 1`), six `http_retry` cases, and one case in each logging target: 1,997 total. File-scoped `rustfmt --check --edition 2021 --config skip_children=true` passed for the four edited Rust files, and `git diff --check` passed. Static inspection confirmed all six retry names remain and the `.covgate` diff is empty. These are expected counts; runtime inventory, coverage, refinement, and full CI validation remain pending. No local compilation/full tests, threshold changes, generated-library edits, commits, or pushes were performed during the test implementation step. This plan remains active until the final acceptance conditions are met.

## Context and Orientation


`agent/Cargo.toml` declares the empty Cargo feature `test`. Cargo features change how a library is compiled even when it is used by another crate. Rust's ordinary `#[cfg(test)]` instead includes code only in that crate's unit-test build. The broad suite in `agent/tests/mod.rs` is currently an automatically discovered integration-test crate: it uses the separately compiled library, where `cfg(test)` is false. Replacing every feature gate with `cfg(test)` without changing this organization therefore breaks the suite.

Research at the recorded base found 49 feature-test conditionals: 38 in production-source files and 11 setup guards in `agent/tests/sync/syncer.rs`. The suite spans 154 files under `agent/tests/` plus source-inline tests, with additional cases generated by macros. These source counts are orientation only; acceptance uses actual test names emitted by CI. Existing inline tests, including generated status-enum tests in `agent/src/models/status.rs`, also belong in that inventory.

`agent/src/http/retry.rs::with_retry` performs one initial attempt and at most two retries for connection errors. Production delays each retry by `500 + (SystemTime subsecond nanoseconds % 500)` milliseconds; the feature-enabled implementation returns zero instead. `agent/src/s3/mod.rs::Store::from_http_client` independently rebuilds the client and discards the supplied credentials in favor of dummy credentials. `agent/src/gcs/mod.rs::Store::build` already shares construction between ordinary and injected clients. `agent/src/data_uploads/upload/transfer.rs::SdkTransfer` conditionally compiles both injection fields and selection branches.

`agent/src/filesys/dirs.rs` and `agent/src/filesys/files.rs` contain test temporary-directory/file wrappers and seed helpers. Their owned `tempfile` guards delete the resource on drop; preserving that ownership matters. Production `dirs::create_temp` also uses `tempfile`, so the dependency must remain available to production. Scanner inspection commands and the sync state setter are additive test access, unlike the alternate retry implementation.

`agent/tests/logs_init_smoke.rs` and `agent/tests/logs_init_locked.rs` deliberately run as separate integration executables because the global tracing subscriber can be installed only once per process. The latter currently assumes the wrapper supplied `RUST_LOG=off`. `.github/workflows/ci.yml` runs Linux lint, coverage gates, independent linter-tool checks, and a native Windows library/binary compile check. `scripts/lib/lint.sh` already checks all Cargo targets; the wrapper already scans both `agent/src` and `agent/tests` for imports and `agent/tests` for assertion style.

## Plan of Work


Milestone 1 establishes evidence and normal test visibility. Obtain runtime test names from a successful CI run at the selected base commit before changing the suite. Prefer existing logs. If they lack a complete inventory, use a one-time CI run of an isolated base checkout with its then-required feature and `-- --list`; do not retain that historic build in the final workflow. Record the base SHA, run URL, target names, and complete lists in implementation evidence rather than checking in a large baseline or building an inventory framework.

In `agent/src/lib.rs`, mount the existing suite with `#[cfg(test)] #[path = "../tests/mod.rs"] mod tests;`. A `#[cfg(test)] extern crate self as miru_agent;` alias can retain existing imports of production APIs. In `agent/Cargo.toml`, set package `autotests = false` and declare explicit `[[test]]` targets `logs_init_smoke`, `logs_init_locked`, and `http_retry`, using their existing paths, with the retry path `tests/http/retry.rs`. Remove `pub mod retry;` from `agent/tests/http/mod.rs` so those retry cases run exactly once. Do not add a target for `agent/tests/mod.rs`.

Keep production API imports distinct from test-harness imports. Rewrite references that previously meant the integration crate's root to `crate::tests::...`: `mocks`, `test_utils`, `errors::harnesses`, `models::harnesses`, `sync::{helpers,syncer}`, and queue/cache harness paths. Update `$crate::...` references inside `agent/tests/cache/{single_thread,concurrent}.rs` and paths expanded by model harness macros. In `agent/tests/data_uploads/queue.rs`, change the harness references in `queue_suite_emit!` and `queue_suite!` to `$crate::tests::data_uploads::queue`. Exported macro names still live at crate root. Do not solve module collisions with test re-exports inside production modules.

Move `TempDir`, `TempFile`, `temp`, and `seed` convenience code into a small support subtree rooted at `agent/tests/test_utils/filesys.rs`, exposed by `agent/tests/test_utils/mod.rs`; nested `dirs` and `files` support modules are acceptable. Update all callers, including source inline tests, to use the support paths. Preserve guard lifetime, conversions to production `Dir`/`File`, cleanup-on-drop tests, and real filesystem operations. Move fixture-specific inline tests with their helpers or import support explicitly; ordinary source tests remain `#[cfg(test)]`. The two logging executables should use local `tempfile` guards and public logging APIs, without importing the library's test support. Explicitly set `RUST_LOG=off` in the locked test and clear it in the smoke test before subscriber initialization and before work that may observe the environment.

Convert additive inspection/state setup in `agent/src/data_uploads/scan/{state,rule,scanner}.rs` and `agent/src/sync/syncer.rs` to ordinary `#[cfg(test)]` with private or `pub(crate)` visibility as needed. Keep the same actor request handling, state transitions, and cooldown logic. Remove all 11 feature guards from the setup operations in `agent/tests/sync/syncer.rs`, so their state initialization always executes. Do not weaken cooldown, reset, or shutdown assertions. This milestone may temporarily retain the Cargo feature for runtime alternatives until milestone 2.

Milestone 2 removes alternate production behavior. Delete the zero-delay retry implementation and compile the existing production delay in every build. A private pure helper receiving nanoseconds may expose the existing arithmetic to inline tests, while the ordinary caller still reads `SystemTime`. Do not make the algorithm or sleep duration depend on `cfg(test)`. In the separate `http_retry` integration target, use paused Tokio time and a local error implementing the public `miru_agent::errors::Error` trait if needed. These tests must invoke public `with_retry` from the externally compiled library, where `cfg(test)` is false. Retain the six existing retry cases and add assertions for the timing contract specified below.

Consolidate S3 client construction into one ordinary internal builder that always uses `Config.creds` and region and accepts optional HTTP transport/path-style settings. `Store::new` supplies production defaults; an injection convenience constructor may remain `#[cfg(test)] pub(crate)` but calls that same builder. Preserve default endpoint resolution, TLS, SDK retry behavior, and virtual-host/path-style defaults. Keep GCS's existing shared `build` logic, including bearer-token validation, sensitive headers, retry limits, and timeouts; expose only the crate-private ordinary construction entry point needed by `SdkTransfer`. Test-only wrappers such as `from_stub` can use `#[cfg(test)] pub(crate)`. Replace feature-gated `Default` implementations for dummy S3/GCS credentials with explicitly named fixture functions in test support or the relevant suite.

In `SdkTransfer`, make dependency settings or a small internal factory ordinary private data compiled in every build. Both default and injected instances must call the same store-construction path; only dependency values differ. Keep `Default` equivalent to today's production settings. Test-only convenience constructors may populate this ordinary data, but neither transfer selection nor SDK construction may branch on compilation mode. Retain full offline S3/GCS request tests in `agent/tests/data_uploads/upload/transfer.rs`, `agent/tests/s3/`, and `agent/tests/gcs/`. Extend captured S3 requests to assert that signing uses the supplied access key and session token; the current constructor that substitutes dummy credentials must fail this assertion. Do not make a blanket public API out of test helpers. Remove `[features] test = []` after all agent feature uses are gone.

Milestone 3 updates the entry points. Clear `CARGO_FEATURES` in `scripts/{test,coverage,covgate,update-covgates}.sh` so inherited environment values cannot reactivate the removed feature. Generic optional-feature plumbing in `scripts/lib/{test,coverage,covgate,update-covgates}.sh` can remain, but change the examples that recommend the agent's old feature. Leave the threshold-updating wrapper usable without running it or changing any `.covgate` file.

In `.github/workflows/ci.yml`, retain the existing Windows production check and add a Linux production check using `cargo check --package miru-agent --locked`. Exercise the exact ordinary workspace and package test commands, the test wrapper, coverage wrapper, and coverage gate on Linux with `RUST_LOG` initially absent. Reuse cached builds and group these commands sensibly; the HTML coverage wrapper and gate both require execution evidence, so allow a justified timeout increase if necessary. Keep existing Linux lint and linter-tool validation. Record target-specific `-- --list` output for migration verification. No final workflow command may enable the removed agent feature.

Update `README.md`, `AGENTS.md`, and `ARCHITECTURE.md` to describe ordinary Cargo tests, the library-mounted private suite, separate public integration targets, and existing `#[serial]` coordination for fixed resources. Remove claims that all tests need one thread. Update only operative future instructions in `plans/active/20260812-revendor-backend-spec-v05-beta2.md`, `plans/active/20260911-platform-paths.md`, `plans/active/20260911-portable-path-fixtures.md`, and `plans/active/20260812-retention-queue-structural-durability.md`; preserve completed historical records. General-purpose `cfg(feature = "test")` parser/classifier fixtures under `tools/lint` are unrelated and stay unchanged.

Milestone 4 closes verification and delivery. Compare actual base and final CI inventories by target and test name using the rules below. Review the entire diff for compile-mode substitutions, skipped assertions, public helper exposure, and changes copied from PR #237. Run the refinement and preflight workflows, fix all actionable findings and CI failures, and keep the PR draft until preflight returns exactly `CLEAN` with successful CI on the pushed final branch HEAD. Static review alone cannot satisfy this task. Commit milestone documentation before the final push; any later commit requires fresh CI evidence for its new SHA.

## Concrete Steps


All local commands below run from `/home/ben/miru/workbench4/repos/agent`. Heavy compilation, tests, lint, and coverage run only in GitHub Actions. Local inspection, editing, diff checks, and file-focused formatting are allowed.

For milestone 1, inspect branch/base and existing CI evidence before editing:

    git status --short
    git fetch origin main
    git branch --show-current
    git rev-parse origin/main
    git log --oneline origin/main..HEAD
    BASE_SHA=$(git rev-parse origin/main)
    gh run list --commit "$BASE_SHA" --workflow CI --status success --json databaseId,headSha,url

Expect the cleanup branch with only its plan commits above the chosen main base and no unrelated working-tree changes. If main advanced, incorporate only that main state and record the new base SHA before collecting inventory. Inspect a returned run with `gh run view RUN_ID --log`, substituting its actual numeric ID. If no complete runtime names are available, an isolated base CI checkout must run `RUST_LOG=off cargo test --package miru-agent --features test -- --list` once and provide the resulting lists. That command is exclusively historical baseline evidence, not a final workflow step. Promote the plan and perform milestone 1 edits, then commit:

    git mv plans/backlog/20260913-remove-test-feature.md plans/active/20260913-remove-test-feature.md
    git diff --check
    git add agent/Cargo.toml agent/src agent/tests plans/active/20260913-remove-test-feature.md
    git commit -m "refactor(test): use normal unit-test visibility for private fixtures"

For milestone 2, implement the shared runtime paths and regression tests. Inspect all remaining conditionals before the commit:

    rg -n 'feature\s*=\s*"test"' agent/src agent/tests agent/Cargo.toml
    git diff --check
    git add agent/Cargo.toml agent/src agent/tests plans/active/20260913-remove-test-feature.md
    git commit -m "refactor(test): share production retry and storage construction"

The search must return no agent feature-test conditions; `rg` exit status 1 means no matches. Review remaining ordinary `cfg(test)` blocks to ensure they add tests, fixtures, or access rather than replace production algorithms.

For milestone 3, edit wrappers, documentation, and CI, then commit:

    git diff --check
    git diff --name-only -- ':(glob)**/.covgate'
    git add .github/workflows/ci.yml scripts README.md AGENTS.md ARCHITECTURE.md plans/active
    git commit -m "ci(test): validate ordinary Cargo tests without custom features"

The `.covgate` diff must be empty. Stage only task-owned edits if other work appears. The Linux CI checkout root must execute the following commands, with the existing toolchain, `cargo-llvm-cov`, and `jq` installed. The listed environment reset deliberately tests direct invocation without wrapper-provided logging configuration:

    cargo check --package miru-agent --locked
    env -u RUST_LOG cargo test
    env -u RUST_LOG cargo test --package miru-agent
    env -u RUST_LOG ./scripts/test.sh
    env -u RUST_LOG ./scripts/coverage.sh
    env -u RUST_LOG ./scripts/covgate.sh
    env -u RUST_LOG cargo test --package miru-agent --lib -- --list
    env -u RUST_LOG cargo test --package miru-agent --test http_retry -- --list
    env -u RUST_LOG cargo test --package miru-agent --test logs_init_smoke -- --list
    env -u RUST_LOG cargo test --package miru-agent --test logs_init_locked -- --list

Each execution must exit zero, every test run must report zero failures, the coverage wrapper must generate `target/coverage/html/index.html`, and all coverage gates must pass their unchanged floors. Existing lint jobs continue executing `LINT_FIX=0 ./scripts/lint.sh`, `LINT_FIX=0 ./tools/lint/scripts/lint.sh`, and `./tools/lint/scripts/covgate.sh` in CI. The native Windows job continues executing `cargo check --target x86_64-pc-windows-msvc --package miru-agent --locked`.

For milestone 4, invoke preflight with `ci_trigger: draft-pr` and `base: main`. Let it own refinement, publishing, draft-PR creation or reuse, and CI watching for every correction batch. Prepare the PR description first, covering the resulting behavior, retained external-test boundary, and validation evidence. After preflight returns `CLEAN`, complete the inventory comparison and retrospective, then provisionally move the plan and prepare the final documentation commit:

    git mv plans/active/20260913-remove-test-feature.md plans/completed/20260913-remove-test-feature.md
    git add plans/completed/20260913-remove-test-feature.md
    git commit -m "docs(plan): record test-feature cleanup verification"

Run preflight again for this documentation commit. Capture `FINAL_SHA=$(git rev-parse HEAD)` immediately after its publish step rebases and pushes; recapture it whenever another round publishes a new head. After preflight returns `CLEAN`, verify:

    gh pr view --json headRefOid,url,isDraft
    gh run list --commit "$FINAL_SHA" --workflow CI --json databaseId,headSha,status,conclusion,url

The PR head must equal `FINAL_SHA`, and all relevant CI jobs for that pushed head must be successful. Use `gh pr ready` only after those conditions hold. Otherwise retain draft status and report the unresolved failure; do not mark the task complete or substitute a static-clean report.

## Validation and Acceptance


Test preservation is a comparison of actual cases, not a static test-attribute count. Keep original and final target-specific inventories and execution results as CI artifacts/log references. Existing source-inline cases retain their library target and names. Map the old `mod` integration target's names to final library names with the one added `tests::` prefix removed. Map final `http_retry` names back to `http::retry::<name>` in the old `mod` target. Keep both logging targets distinct. Explicitly record any fixture-test name/target moves and every additional regression case. Compare sorted multisets, not just sets or totals, to catch duplicates as well as missing cases; generated macro cases, ignored status, and any platform-conditioned cases must be accounted for. Compare equivalent Linux environments. No unexplained missing test, new ignore, duplicate suite, or removed substantive assertion is acceptable.

The retry integration executable must prove that a connection failure does not trigger another attempt before 500 ms and does trigger it by 1,000 ms of controlled Tokio time. Poll the retry future once to register its sleep, advance to 499 ms and confirm the attempt count remains one, then advance through 1,000 ms and observe attempt two. Ensure the test harness does not accidentally permit Tokio's idle auto-advance before each assertion. Success and application errors must return after one attempt without virtual time advancing; persistent network errors stop after three total attempts, and a network error followed by an application error stops after two. Private arithmetic tests cover `0 -> 500`, `499 -> 999`, and `500 -> 500` ms. The new nonzero-delay assertion fails against the old feature-enabled zero-delay variant. Do not pause suites that depend on real socket I/O indiscriminately.

Filesystem guard tests must show that resources exist while owned and are removed on drop. Logging tests must pass both direct Cargo commands with `RUST_LOG` absent; the locked test proves its own environment setup. Existing scanner and sync cases must still exercise inspection, state setup, cooldown, reset, and shutdown. SDK transfer cases must retain request body, physical bucket, object key, metadata, authentication, error classification, and missing-file behavior. S3 captured signing headers must contain the supplied access key and session token; GCS must retain its supplied bearer-token assertions. Default store construction must retain its offline missing-file coverage and production defaults.

Acceptance also requires no agent Cargo `test` feature or references enabling it in final operational commands, no blanket public test helper exports, unchanged `.covgate` files, passing Linux and Windows production checks, passing tool lint/tests, and preflight `CLEAN` backed by green CI on the exact pushed final SHA. A passing unit suite by itself does not prove the public externally compiled retry/logging boundary, so those explicit targets must execute successfully too.

## Idempotence and Recovery


Inspection, inventory comparison, and CI reruns are repeatable. Do not run local heavyweight validation, regenerate API clients, update unrelated dependencies, or run the threshold updater to hide failures. If moved-suite imports fail, repair the test namespace or narrow visibility; do not restore the feature, expose all helpers publicly, delete cases, or skip tests. If paused time interacts badly with external I/O, constrain virtual time to isolated retry tests and keep the real-I/O tests exercising ordinary behavior.

Check the working tree before every stage/commit operation and preserve unrelated changes. A milestone can be reversed with a normal targeted `git revert` of its own commit after considering dependent milestones; never reset the workspace or rewrite unrelated history. If the final branch must incorporate newer main changes, record the new base, reconcile test inventory additions, and rerun CI for the new head. If final acceptance fails or is interrupted after the provisional move, move `plans/completed/20260913-remove-test-feature.md` back to `plans/active/20260913-remove-test-feature.md`, record the unresolved condition, and commit that recovery with the next correction batch before preflight publishes again. Repeat the provisional completion sequence after corrections pass; keep the PR draft until all acceptance conditions are satisfied.
