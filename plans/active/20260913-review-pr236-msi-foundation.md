# Review and refine the Windows MSI foundation in PR 236

This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective current throughout implementation.

## Scope


| Repository | Access | Purpose |
| --- | --- | --- |
| `/home/ben/miru/workbench5/repos/agent` | Read-write | Review the entire PR, correct accepted defects, add regression coverage, and push the existing branch. |
| `/home/ben/miru/workbench5` | Read-only | Shared agent instructions and review skills; never edit its `.agents/` subtree. |

This plan belongs in the agent repository because that repository owns every deliverable. Delivery is `task mode:push` on existing branch `feat/windows-msi-packaging` and existing draft PR [236](https://github.com/mirurobotics/agent/pull/236). Preserve its draft status. Do not create another PR or branch.

## Purpose / Big Picture


Make the proposed Windows installer foundation reliable enough to review and merge: relevant changes must receive native Windows validation, installer failures must remain diagnosable, and existing installation, upgrade, rollback, permissions, and retained-data behavior must remain intact. Completion requires accepted review findings corrected with meaningful regression coverage and green CI associated with the final pushed branch head.

## Progress


- [ ] Record the full-PR review and critique decisions.
- [ ] Correct accepted defects, add regression coverage, and refine the full PR.
- [ ] Reach preflight CLEAN with green CI on the pushed source head.
- [ ] Complete the plan, push delivery changes, and verify CI on the final head.

## Surprises & Discoveries


Add observations and supporting evidence as work proceeds.

## Decision Log


Add non-trivial decisions, their reasons, dates, and authors as work proceeds.

## Outcomes & Retrospective


Record delivered behavior, final commit and CI evidence, remaining limitations, and lessons at completion.

## Context and Orientation


The agent is a Rust executable that synchronizes device configuration. This PR packages its existing x64 Windows console executable with WiX 5.0.2, a tool that produces Windows Installer packages (`.msi`). It does not implement a Windows service. The installer places the executable under `Program Files/Miru/Agent` and protects `ProgramData/Miru` and its logs for administrators and SYSTEM. Populated customer state must survive uninstall and upgrade. Preserve UpgradeCode `B5ED0336-5F14-4308-A667-3CE8CDEF7D48`, the identifier linking product versions, and existing component identities.

Read `AGENTS.md` and `ARCHITECTURE.md` before implementation. The original eight changed files are `.github/workflows/ci.yml`, `build/windows/README.md`, `build/windows/miru-agent.wixproj`, `build/windows/miru-agent.wxs`, `build/windows/tests/integration-test.wxs`, `build/windows/tests/integration-tests.ps1`, `build/windows/tests/package-tests.ps1`, and `plans/active/20260910-windows-support.md`. Review all changes from the merge base, the common ancestor of the PR branch and `origin/main`, including any files subsequently added to the PR.

At research time HEAD and PR head were `942952fc02b97248173a14827875996e90844ee5`. The merge base was `a0e7afb33fd8964539e9e309f0c83943b14c5033`; `origin/main` was ahead at `3f20eb06554159850da08c340c4acc1c70ab8f5f` with an Actions dependency bump. Fetch and resolve these again. Baseline CI run `34789271323` passed, but validates only that baseline.

`.github/workflows/ci.yml` classifies PR paths in `windows_scope` using GitHub-owned `actions/github-script`, then chooses Windows compilation or complete MSI validation. `.github/workflows/release.yml` calls this reusable workflow under its own permission ceiling. GitHub-owned actions are permitted; do not reintroduce `dorny` or add `workflow_dispatch`. Existing PR `synchronize` events, emitted by branch pushes, are the CI trigger.

The WiX project validates executable inputs and three numeric version fields: major/minor at most 255, patch at most 65535. Package tests inspect MSI tables, identities, permissions, isolation, invalid inputs, and upgrade sequencing. Integration tests build fixtures with distinct marker payloads and a rollback-failure action; they exercise real install, repair, upgrade, downgrade rejection, rollback, uninstall, nonadministrator denial, retained state, and offline `--provision --check` returning 3. They require elevated 64-bit Windows PowerShell 5.1 on a disposable Windows host. Native CI typically takes about 14 minutes.

## Plan of Work


Begin source work with a defect-first review of the entire merge-base diff and affected callers. Every review agent must first read `/home/ben/.codex/skills/.system/review-agent/SKILL.md` and `/home/ben/miru/workbench5/.agents/skills/review/SKILL.md`, remain read-only, and never delegate. The review-agent/user rule overrides delegation instructions in the review methodology. Do not limit later reviews to this task's corrections: source refinement and preflight refinement must also cover the full PR.

Refinement uses fresh agents in the order review, critique plus plan, then fix, passing structured text containing finding IDs, locations, failure scenarios, evidence, and proposed validation. The critique assigns only `fix` or `skip` to each finding and explains why. Fix agents implement only `fix` verdicts. Repeat at most three cycles, recording residual findings rather than declaring unresolved work clean. The following candidates are investigation targets, not accepted findings:

1. Check whether `windows_scope` requesting `pull-requests: read` exceeds `.github/workflows/release.yml` caller permissions, even when the classifier's non-PR path avoids the API. If confirmed, minimally correct caller permissions or classifier placement while preserving triggers and release behavior; validate the caller/callee relationship without publishing a release tag.
2. Inspect `Invoke-Msi`, the manual-smoke outer `finally`, and integration catch/cleanup in `build/windows/tests/integration-tests.ps1`. Determine whether temporary-directory deletion loses manual-failure logs or logs created by a cleanup-only MSI failure. If confirmed, preserve logs after all cleanup paths, report their persistent location, and preserve the original failure when cleanup also fails.
3. Check whether classification of only `filename` omits `previous_filename` when a relevant path is renamed outside its matched directory. If confirmed, classify both names in `.github/workflows/ci.yml`. Add small behavioral coverage for renamed package inputs, renamed compile inputs, ordinary relevant changes, irrelevant changes, and non-PR events; avoid duplicating the production classifier in tests.
4. Compare README promises with `Invoke-ManualSmoke`: maintenance and uninstall currently discard `Invoke-Msi` results while install and upgrade print them. If confirmed, report each stage's 3010 reboot-required result and maintain truthful documentation. Validate output with injected results; do not claim an interactive Windows smoke run occurred without evidence.

Correct other substantiated defects found anywhere in the full PR using the same critique process. Keep changes minimal. Add or extend regression tests for accepted behavioral defects, using existing PowerShell harnesses where suitable. If isolated helper tests are needed, place them in `build/windows/tests/harness-tests.ps1` and invoke them from the Windows job. Add no test dependency merely for these checks. Exercise failure, cleanup-only failure, combined failures, and successful cleanup without installing on a developer machine. Update `build/windows/README.md` only where delivered behavior or instructions change.

Preserve protected retained data, fixture ProductCode allowlists, transactional upgrades, and the production/test-package boundary. Windows service support, signing, package publication, and WinGet remain deferred. Do not expand this task to those features or dependency refreshes.

## Concrete Steps


All commands below run from `/home/ben/miru/workbench5/repos/agent`; Windows commands run from that repository's checkout root on the disposable CI runner. Local inspection and lightweight syntax/diff checks are allowed. Do not run full lint, tests, coverage, or native installer tests locally; CI is their execution environment.

The parent activates this plan and invokes `$implement` exactly once with this plan path, repository, `base=main`, and `ci_trigger=draft-pr`. That implementation orchestrator owns milestones 1–3 through its source, refinement, test, and preflight phases; these milestones do not instruct source or test workers to invoke `$implement` again. Milestone 4 belongs to the parent after `$implement` returns `CLEAN`.

Milestone 1 — establish and review the baseline. After the orchestrator activates this plan at `plans/active/20260913-review-pr236-msi-foundation.md`, inspect the branch and PR:

    git status --short --branch
    git fetch origin main feat/windows-msi-packaging
    git branch --show-current
    git merge-base origin/main HEAD
    git diff --stat origin/main...HEAD
    git diff origin/main...HEAD
    git log --oneline HEAD..origin/main
    gh pr view 236 --json url,headRefName,headRefOid,baseRefName,isDraft,statusCheckRollup

Expect branch `feat/windows-msi-packaging`, matching local/PR heads, and `isDraft: true`. Resolve mismatches before changing files; preserve unrelated user changes. Run the full review and critique described above, record accepted/skipped findings and planned regressions in this plan, then end the milestone with:

    git add -- plans/active/20260913-review-pr236-msi-foundation.md
    git commit -m "docs(plan): record full Windows MSI PR review"

Milestone 2 — implement accepted source and test corrections. Apply minimal changes, run the fresh full-PR refinement cycle, and record what each regression proves. Inspect `git diff --check`, `git diff --stat`, and the actual patch. Stage only task-owned changes from the workflow files, `build/windows/`, and the active plan using explicit file paths after inspecting them. End the milestone with:

    git diff --cached --check
    git diff --cached --stat
    git commit -m "fix(windows): address accepted MSI foundation review findings"

Milestone 3 — the implementation orchestrator invokes `$preflight` for full source/CI verification. Use `ci_trigger=draft-pr` despite `task mode:push`; this is an explicit task override. Follow preflight’s normal rebase onto the latest `origin/main`, including the already-existing upstream dependency bump. Before rebasing, fetch and inspect the PR branch and record its remote SHA. Incorporate any newly discovered remote commits without discarding their changes. Use a normal push when possible; when rebasing requires rewriting the PR branch, use `--force-with-lease` constrained to the inspected remote SHA. A rejected lease requires another fetch, inspection, and reconciliation before retrying. Preflight must perform full-PR refinement, push to the existing branch, and report CLEAN only after associated CI is green. Set `max_ci_rounds=3` for source preflight: each round consists of one batch of changes, one push, and one CI run watched to completion. The parent’s single delivery recheck in milestone 4 is separate from this repair budget. The existing workflow executes:

    LINT_FIX=0 ./scripts/lint.sh
    ./scripts/covgate.sh
    LINT_FIX=0 ./tools/lint/scripts/lint.sh
    ./tools/lint/scripts/covgate.sh
    cargo build --target x86_64-pc-windows-msvc --package miru-agent --locked --release
    dotnet restore build/windows/miru-agent.wixproj
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\package-tests.ps1 -ProjectPath build\windows\miru-agent.wixproj -BinDir target\x86_64-pc-windows-msvc\release -ArtifactsDirectory build\windows\artifacts\package-tests
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File build\windows\tests\integration-tests.ps1 -Configuration Release -ConfirmDisposableTestMachine

The compile-only route uses `cargo check --target x86_64-pc-windows-msvc --package miru-agent --locked`. This PR includes packaging changes, so acceptance requires its native package and lifecycle steps to execute. Preflight performs the rebase and single push described above; inspect evidence with:

    gh pr checks 236
    gh run list --branch feat/windows-msi-packaging --limit 5
    gh run view RUN_ID --json headSha,status,conclusion,jobs
    gh run view RUN_ID --log-failed

Replace `RUN_ID` with the observed run ID. Associate the run with the pushed commit, accounting for PR merge-commit SHAs when necessary. Expected terminal result is `status: completed`, `conclusion: success`, with required jobs and native package steps passing. Static checks or baseline CI alone are insufficient. Read failures from CI, correct accepted defects, and make each correction round a separate milestone ending in a scoped commit before its next push. After successful verification, record evidence and end this milestone with:

    git add -- plans/active/20260913-review-pr236-msi-foundation.md
    git commit -m "docs(plan): record Windows MSI preflight evidence"

Milestone 4 — parent delivery completes the plan. Fill Outcomes & Retrospective, mark implementation progress complete, and move the plan only after source preflight is CLEAN. Include the known source-validation run and state that final delivery verification follows the completion commit. End this milestone with:

    git mv plans/active/20260913-review-pr236-msi-foundation.md plans/completed/20260913-review-pr236-msi-foundation.md
    git add -- plans/completed/20260913-review-pr236-msi-foundation.md
    git commit -m "docs(plan): complete Windows MSI PR refinement"
    git push origin HEAD:feat/windows-msi-packaging

Recheck CI using the commands above on this final pushed head; do not report completion before it passes. This is one mandatory delivery recheck, separate from the three source-preflight rounds. If it fails, treat delivery as `CAPPED`, report the failing jobs and evidence, preserve draft status, and stop without starting another repair round. After it passes, preserve draft status when resynchronizing the PR description with the delivered changes and evidence. Report the final SHA and run in the handoff without another bookkeeping commit.

## Validation and Acceptance


Full-PR review findings have explicit critique dispositions, accepted defects have source corrections and meaningful regression coverage, and fresh full-PR review leaves no accepted defect unresolved. For accepted classifier fixes, moving a file from `build/windows/` to an irrelevant destination still requests packaging; moving from `agent/` still requests compilation; irrelevant-only changes request neither. Validate real classifier behavior, not a copied predicate.

For accepted diagnostic fixes, manual failure and cleanup-only failure leave readable MSI logs outside deleted temporary output; a combined failure retains the original error and additional cleanup evidence. Injected 3010 at any promised manual stage produces stage-specific reboot reporting. Existing native checks still prove install/repair/upgrade/downgrade/rollback/uninstall, administrator/SYSTEM protection, denial to a nonadministrator, retained customer state, and the offline exit code of 3. Describe tests that expose the original defect and their observed post-fix results; do not invent counts or native manual-smoke evidence.

All required Linux/tool CI and native Windows package/integration validation must pass, preflight must report CLEAN, the local and remote branch must agree, and the final completion-plan commit must also have green CI. No release tag, service, signing, publication, or WinGet change is required.

## Idempotence and Recovery


Fetching, diff inspection, and CI status reads are repeatable. Keep commits scoped and preserve unrelated user work. Preflight may rebase onto `origin/main` and publish rewritten commits using the inspected-SHA lease described in milestone 3; never use an unconditional force push or discard newly discovered remote changes. Do not reset away user work or change installer identities to simplify tests. Reuse the disposable-host safeguards and allowlisted fixture cleanup; never delete arbitrary installed products or customer data. On failure, preserve evidence before deleting owned temporary files. If three refinement cycles or three source-preflight CI rounds leave unresolved work, or the separate delivery recheck fails, report the precise remaining failure and evidence; do not claim CLEAN or completion.
