# Refine the Windows service lifecycle PR


This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective current as work proceeds.

## Scope


The sole repository to change is `/home/ben/miru/workbench2/repos/agent` (`mirurobotics/agent`). Refine the existing branch `feat/windows-service-lifecycle` and deliver the existing draft PR https://github.com/mirurobotics/agent/pull/242 against `main`. Read the workbench instructions and skills at `/home/ben/miru/workbench2/.agents/skills/` and the review skill at `/home/ben/.codex/skills/.system/review-agent/SKILL.md` as read-only references. Do not change the workbench, shared skill files, generated API libraries, or unrelated code.

The completed plan's intended path is `plans/completed/20260917-refine-windows-service-lifecycle.md` in the agent repository; relocation from `plans/active/` is pending delivery bookkeeping. The explicit task workflow and current repository convention use `plans/backlog`, `plans/active`, and `plans/completed`; this placement supersedes the repository planning policy's older `.agents/exec-plans` paths.

## Purpose / Big Picture


Establish that PR #242 correctly runs the Windows agent under the Service Control Manager (SCM), the Windows component that starts, stops, and observes background services. Find and fix only demonstrated regressions introduced by this PR, while preserving its intended foreground `--console` option, Windows persistence, and Unix behavior. Deliver the same PR ready for review only after review findings are resolved and CI is green for the final pushed commit.

## Progress


- [x] (2026-09-17) Read applicable repository/workbench instructions, architecture, repository plan skill and its complete policy, and the explicitly selected review/refine/preflight/task workflows.
- [x] (2026-09-17) Confirm PR identity, branch, merge base, existing tests, and CI configuration; author this conditional plan.
- [x] (2026-09-17) Promote this plan to `plans/active/` and commit through the task orchestrator (`a33c9631`).
- [x] (2026-09-17) Review the entire PR diff with a fresh read-only review agent; a separate critique confirmed its single P2 startup-stop finding (one accepted, zero skipped).
- [x] (2026-09-17) Apply the confirmed startup-stop fix and four portable regression cases (`5ee82d33`), then complete fresh full-diff review iteration 2 with no findings (two of three iterations used).
- [x] (2026-09-17) Run CI-driven preflight: `CLEAN` on pushed head `50a1f3932460d2481a49197cb12513135ae5e4d4`, CI run [35277371800](https://github.com/mirurobotics/agent/actions/runs/35277371800), one of three CI rounds used.
- [x] (2026-09-17) Complete implementation and this plan's outcome record; ready for delivery.
- [ ] Relocate the completed plan, commit and push final bookkeeping, verify CI on that final head, update PR #242's description, and mark that PR ready.

## Surprises & Discoveries


The initial tree was clean. Both local HEAD and `origin/feat/windows-service-lifecycle` were `86986fe3c6bcdba1427a65a46b0600594f17b71f`; `origin/main` and the merge base were `fa80a317af1acc2a2f9fbc0255c53fab607ae0d1`. PR #242 was draft. CI run `35275351251` had successful lint, test, tools, windows-check, and windows-package-scope jobs. The packaging job was intentionally skipped by its path classifier. These observations are a baseline, not validation of later commits.

The PR body still cites older commit evidence, although its current head has newer green CI. Refresh its description at delivery from the final behavior and actual validation. Full-diff review iteration 1 reported one P2 finding: SCM STOP is never observed during startup upgrade reconciliation, so an activated device with an outdated or absent version marker and an unavailable backend can remain in StopPending indefinitely. A fresh critique confirmed the call path. Although reconciliation's retry loop predates the PR and individual HTTP requests time out, the PR newly accepts SCM STOP during this unbounded loop; the latched signal and later application watchdog cannot release it.

Cancellation must occur between complete reconciliation attempts or during retry backoff. `disk::setup::reset` performs multiple writes/deletions before writing the version marker last, so racing STOP against the whole reconciliation future would risk interrupting persistence. The accepted fix intentionally waits for an active attempt to finish.

Fresh full-diff review iteration 2 inspected all 26 changed paths at `5ee82d33900fd05b802f96ee04aaaa9e8d9a58f0` against merge base `fa80a317af1acc2a2f9fbc0255c53fab607ae0d1`, plus relevant callers and tests, and returned no findings. A fresh fetch confirmed `origin/main` was unchanged. The reviewer confirmed that the startup-stop defect is resolved without cancelling an active disk reset.

CI run [35277371800](https://github.com/mirurobotics/agent/actions/runs/35277371800) passed on exact pushed head `50a1f3932460d2481a49197cb12513135ae5e4d4`. Lint, Linux tests/coverage, tools, Windows tests, and Windows packaging scope all succeeded; packaging itself was skipped by its path classifier as expected. The four new regression names appear with `ok` in both Linux and Windows logs. All coverage gates passed (app 94.06%, portable Windows module 100%). No CI repair was needed.

## Decision Log


2026-09-17, planner: Reuse PR #242 and its branch instead of the generic task workflow's new-branch/new-PR setup, because the requested outcome explicitly targets this existing draft.

2026-09-17, planner: Use `/home/ben/.codex/skills/.system/review-agent/SKILL.md` for every review stage within the workbench `$refine` workflow. The reviewer must inspect the complete change, remain read-only, and neither edit nor delegate. Preserve that skill's findings-first output plus brief assessment and test gaps.

2026-09-17, planner: Make all production/test fixes conditional on confirmed findings. A critique must explain both the evidence for a defect and why it could be a false positive; uncertainty alone does not authorize a change. Investigate until the defect is demonstrated or explicitly skip it. This honors the task's confirmed-findings constraint over refine's generic uncertainty default.

2026-09-17, planner: Use GitHub CI for all full suites, whole-repository lint, and coverage. Do not run local aggregate validation scripts or refresh dependencies merely to satisfy an older local-lint suggestion. Local verification is limited to reading changes, checking diffs, and trivially cheap file-scoped formatting/static checks.

2026-09-17, implementation: Accept the sole review finding after a separate critique. Add a shutdown future to the existing production reconciliation path, returning an explicit stopped result only at safe boundaries. Service startup supplies the latched SCM stop; foreground startup supplies a never-resolving future to preserve its behavior. Test the actual reconciliation function with a failing backend and controlled retry wait, an already-triggered stop, and a stop during a successful attempt that must finish persistence. Apply this conditional source-and-regression batch through the refine fix stage, then review the complete diff again before CI publication.

2026-09-17, implementation: Implementation is complete. Leave this outcome record uncommitted for completed-plan relocation in one final delivery commit, then verify CI on that pushed head. Preserve draft status and the existing PR description until that check is green. Report final delivery evidence in the PR and task after the bookkeeping commit, avoiding a recursive plan-only commit.

## Outcomes & Retrospective


Review iteration 1 produced one confirmed P2 finding and zero skips. Its fix is implemented in `agent/src/app/upgrade.rs` and `agent/src/main.rs`, with four portable regression cases in `agent/tests/app/upgrade.rs`. Reconciliation observes startup stop before and after complete attempts and during backoff; foreground startup passes a never-resolving future. An active attempt is deliberately allowed to finish persistence before the service body exits cleanly.

The regression tests cover offline backoff cancellation without state changes, a pre-triggered stop with zero backend requests, stop during a successful attempt with reset and backend update completed, and the existing typed missing-key validation failure. Five existing successful/retry cases retain their assertions with the new explicit completed outcome. File-scoped formatting and the repository's import/function-length/assertion checker passed on the three changed files; diff whitespace checks passed. No local suites, compilation, whole-repository lint, or coverage were run.

Implementation preflight is `CLEAN`: CI run [35277371800](https://github.com/mirurobotics/agent/actions/runs/35277371800) passed on exact pushed head `50a1f3932460d2481a49197cb12513135ae5e4d4`. The four new regression tests and existing SCM lifecycle tests passed in CI; new regression names were independently confirmed in both Linux and Windows logs. Global counts are two of three review iterations and one of three CI rounds, with one confirmed finding fixed, zero skipped, zero unresolved, and no CI fixes. Commits since the original PR head are `a33c9631` (plan), `5ee82d33` (fix and regression coverage), and `50a1f393` (second-review record). Fetch/rebase was a no-op; the branch was pushed once and local, remote, and PR head SHAs matched with a clean tree before this final plan-only update.

Implementation is complete and ready for delivery; PR #242 remains draft. The task orchestrator must relocate this completed plan, commit and push delivery bookkeeping, validate that final head, update the PR description, and mark the same PR ready. Final delivery evidence belongs in the PR and task after that commit. A live registered Windows SCM start/stop smoke test remains outside the available evidence; shutdown deliberately waits for any active reconciliation attempt to complete so disk reset is not interrupted.

## Context and Orientation


This repository builds a Rust agent that synchronizes robot configuration with Miru. Provisioning and runtime are separate startup paths. `agent/src/main.rs` selects those paths, builds Tokio's asynchronous runtime, and chooses Windows SCM dispatch versus foreground operation in `run_runtime_mode`. `service_body` gives the SCM thread its own runtime and file logging; `run_agent` performs activation, upgrade reconciliation, settings loading, and application execution using a supplied shutdown-future factory.

`agent/src/windows/mod.rs` defines the portable `StopSignal` relay and `RunOutcome`. A stop must reach waiters created before or after it is triggered. `agent/src/windows/scm.rs`, compiled only on Windows, implements `dispatch`, `service_main`, `handle_control`, `status`, `exit_code`, and `run_lifecycle`. Its status reports use `StartPending`, `Running`, `StopPending`, and `Stopped`; `RunOutcome` maps successful completion or failure to the SCM exit status. `agent/src/windows/errors.rs` provides SCM errors, including the foreground `--console` hint.

`agent/src/cli/mod.rs` parses `--console`. `agent/src/platform/mod.rs` exposes `supports_idle_exit`; `LifecycleOptions::resolve_persistence` in `agent/src/app/options.rs` preserves Unix settings while forcing persistent Windows operation. Application shutdown must retain its existing dependency order. Inspect the callers and surrounding application startup/shutdown code when reviewing these changes, including interaction with code merged since the feature began.

Tests live in `agent/tests/windows/{scm,stop_signal,errors}.rs`, `agent/tests/cli/mod.rs`, `agent/tests/platform/mod.rs`, and `agent/tests/app/options.rs`. The Windows-only SCM tests use a recording `StatusSink` instead of a registered service; they run in the Windows CI job. The portable stop relay also runs in Linux CI. Public behavior tests follow the existing integration layout; private-access unit tests belong inline. Do not create test-only production algorithms, suppress coverage, or skip tests to obtain green CI.

Other PR paths include workspace/package manifests, `Cargo.lock`, `agent/src/lib.rs`, test registration, the Windows coverage threshold, `ARCHITECTURE.md`, and the existing Windows roadmap and completed implementation plan. Review all these paths too. MSI service registration, Event Log support, and other roadmap items are not automatically part of this refinement. Existing tests do not prove behavior on a live registered Windows service; disclose that limitation unless actual host evidence is obtained.

## Plan of Work


First promote and commit the plan, confirm the existing branch is still selected, and capture the current upstream base and PR head. A merge base is the latest commit shared by two branches; reviewing against it isolates what the PR would merge. Resolve it against fetched `origin/main`, then inspect the full diff, including uncommitted refinements, rather than only the most recent commit or modified source files.

Run the workbench refine procedure with a maximum of three review/fix iterations. Each iteration begins with a fresh reviewer that receives the repo, base/ref SHAs, complete diff and changed-file list, and the requested review-agent skill. It must inspect every changed path and enough tests/callers to demonstrate actionable regressions. Findings identify severity, affected scenario, and a small changed-line range. No findings is a valid outcome.

For findings, a separate fresh critique/planning agent evaluates both sides and records `fix` or `skip` with evidence. For every confirmed fix, specify the precise file, function, smallest behavior change, and regression scenario before editing. A fresh fixing agent applies only that accepted plan. Add tests that distinguish the defective behavior from the corrected behavior; do not add tests merely to mirror code. Re-read edits and use only cheap file-scoped checks locally. Refresh the entire diff and repeat. Record any remaining findings if the limit is reached; do not silently declare them resolved.

Invoke workbench preflight with base `main`, `ci_trigger: draft-pr`, and at most three CI rounds, explicitly preserving PR #242 and the review-agent override. Batch local changes, commit through `$commit`, fetch/rebase onto `origin/main`, and push once per round. If rebasing changes relevant code, review the resulting full diff before publishing. Watch the run belonging to the exact pushed SHA. Diagnose failures from all failed CI job logs, make a single minimal fix batch, refine again, and use the next round. Do not reproduce full suites locally.

Only a preflight report of `CLEAN` with green CI on the pushed branch head permits delivery. After completing/moving the plan and committing any final bookkeeping, push and watch CI on that new head too. If this delivery re-check fails, leave the PR draft and report the failing jobs. Once green, mark PR #242 ready and invoke `$pr` in update mode to reflect final behavior, findings resolved, validation, and known test limitations. Do not merge the PR.

## Concrete Steps


All repository commands below run from `/home/ben/miru/workbench2/repos/agent`. The task orchestrator handles commits, branch updates, publishing, and delivery; the planner runs none of those mutations.

Confirm identity and CI availability, then obtain review input:

    git status --short
    git branch --show-current
    gh auth status
    test -f .github/workflows/ci.yml
    gh pr view 242 --repo mirurobotics/agent --json number,url,isDraft,headRefName,baseRefName,headRefOid
    git fetch origin main feat/windows-service-lifecycle
    git rev-parse HEAD origin/main origin/feat/windows-service-lifecycle
    git merge-base HEAD origin/main

Expect `feat/windows-service-lifecycle`, base `main`, and the same PR URL. Record the returned merge-base SHA as `REVIEW_BASE_SHA` for that review, then run the following with that literal SHA substituted. Include new untracked task files explicitly, because ordinary `git diff` does not show them.

    git diff --name-status REVIEW_BASE_SHA
    git diff --no-ext-diff REVIEW_BASE_SHA
    git ls-files --others --exclude-standard
    git diff --check

After accepted changes are committed and reviewed, preflight rebases and publishes from this same repository:

    git fetch origin main
    git rebase origin/main
    git push -u origin feat/windows-service-lifecycle

If the rebase rewrites already pushed commits, use `git push --force-with-lease -u origin feat/windows-service-lifecycle` instead of the ordinary push, never an unconditional force push. Reuse the existing draft PR; a synchronized push triggers its CI.

Locate and inspect the run, replacing `RUN_ID` with the matching run's database ID:

    git rev-parse HEAD
    gh pr view 242 --repo mirurobotics/agent --json headRefOid,isDraft
    gh run list --repo mirurobotics/agent --workflow ci.yml --branch feat/windows-service-lifecycle --limit 10 --json databaseId,headSha,status,conclusion,url
    gh run view RUN_ID --repo mirurobotics/agent --json headSha,status,conclusion,jobs,url
    gh pr checks 242 --repo mirurobotics/agent

Poll using bounded waits until the run for the pushed SHA completes. For failed jobs only, obtain `gh run view RUN_ID --repo mirurobotics/agent --log-failed`. After the final green delivery re-check, use `gh pr ready 242 --repo mirurobotics/agent`, then update that PR's description through the PR skill.

## Validation and Acceptance


Full tests, lint, and coverage execute only through `.github/workflows/ci.yml`: Linux `test` runs `./scripts/covgate.sh`; Windows `windows-check` runs `RUST_LOG=off cargo test --package miru-agent --locked`; `lint` runs `LINT_FIX=0 ./scripts/lint.sh`; `tools` runs its lint and coverage scripts. These are CI commands, not instructions for local execution. Require their successful conclusions. `windows-package-scope` must succeed; a classifier-driven `windows-package` skip is expected when no packaging/workflow path changed, otherwise that job must succeed too.

For each confirmed code defect, record a concrete input/event sequence and incorrect result, then identify the new/updated regression test and the corrected expected result. The reviewer must establish why that test distinguishes the pre-fix code; record actual execution only when CI has run it. Relevant existing cases verify stop delivery before/after subscription, repeated stops, status ordering and exit codes, `--console` parsing, and the four persistence input combinations. Confirm these remain covered by the actual CI jobs; do not infer a live SCM smoke test from fake-sink unit tests.

Acceptance requires a completed full-diff review and documented disposition of every finding, minimal fixes only for confirmed defects, a clean working tree after task commits, matching local/remote/PR head SHAs, and an explicit preflight `CLEAN` report meaning **CI green on the pushed branch head**. Static-only validation, an old green run, pending jobs, or `CAPPED` does not satisfy this gate. The PR must not leave draft and the task must not be reported complete until that condition holds, including after final plan commits. Deliver the same PR #242 with an accurate description and remaining test limitations.

## Idempotence and Recovery


Read-only review, status queries, and CI polling may be repeated safely. Recompute the merge base and refresh the full diff after every rebase or fix batch. Preserve unrelated work if the tree becomes dirty; do not reset or overwrite it. Use existing PR #242 rather than creating duplicates. On a rebase conflict, resolve only understood changes and review the resolution; `git rebase --abort` restores the pre-rebase state if necessary. A rejected lease requires fetching and inspecting the remote change before retrying.

If authentication or CI is unavailable, report that validation is incomplete and keep the PR draft. At the refinement or CI limit, report remaining findings/failing jobs and retain restartable progress. Never make green CI by deleting/skipping tests, lowering coverage, or suppressing errors. Final plan relocation is safe only once implementation and preflight have succeeded; if the resulting delivery re-check fails, record that failure and keep delivery incomplete.

Revision (2026-09-17): Record completed implementation and the intended completed-plan path while keeping final commit CI and PR delivery pending.
