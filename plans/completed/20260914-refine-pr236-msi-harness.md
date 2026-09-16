# Review, refine, commit, and preflight PR #236 (Windows MSI harness split)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (`/home/ben/miru/workbench5/repos/agent`) | read-write | Commit the uncommitted harness split and Manufacturer rename, apply review fixes, push to `feat/windows-msi-packaging`. |
| `workbench5/` (`/home/ben/miru/workbench5`) | read-only | Skill definitions only (`.agents/skills/{review,refine,commit,preflight}`). |

This plan lives in `agent/plans/` because every edit and commit happens in the agent repo.

## Purpose / Big Picture

PR #236 (`mirurobotics/agent`, draft, base `main`, branch `feat/windows-msi-packaging`) adds WiX MSI packaging for the Windows agent plus a PowerShell test harness that runs in CI. The working tree currently holds an uncommitted refactor: the harness is split into a shared module and a dot-sourced library, the installer Manufacturer is renamed from "Miru Robotics" to "Miru", and the wixproj error for a missing fixture payload gains code `MIRUMSI1009`. After this plan, those changes are reviewed, fixed, landed as scoped signed commits, and CI on the pushed head is green on all five checks (`lint`, `test`, `tools`, `windows-scope`, `windows-check`). Observable result: `gh pr checks 236` shows every check `pass` for the SHA reported by `git rev-parse HEAD`, and the PR is still a draft.

## Progress

- [x] M1: Baseline captured, full review produced, findings accepted/rejected.
- [x] M2: Commit A (`7a5b0811`) and Commit B (`568155e0`) landed locally, each self-consistent.
- [x] M3: Refine loop fixes committed (`e42c4dcd`, `23636442`, `60d2a57f`); no accepted findings remain.
- [x] M4: Pushed; CI CLEAN on pushed head (run 34879810311 on `f0578ec5`); delivery recheck done.

## Surprises & Discoveries

- The signing precondition was already resolved before M1 (`commit.gpgsign` only from the global config); the plan file had been committed as `34409fa9` under `plans/active/`, not `plans/backlog/`.
- The pre-split harness defined per-case mocks unscoped inside the case body and relied on dynamic scoping; the split changed them to `function script:X`, which is what introduced the leak (finding 5). Reverting to body-local mocks removes the leak with no restore machinery.
- `dotnet restore`/`build` of the wixproj also writes `build/windows/obj` and `build/windows/bin`, so finding 7 needed three ignore entries, not one.

## Decision Log

- Finding 1: accepted; fixed by ordering (Commit A patches the inline assertion, Commit B ships `MsiTest.psm1` with `"Miru"`).
- Finding 2: accepted (major). Only reachable on the manual production-smoke path, but deterministic on a real VM. Fixed in `e42c4dcd`.
- Finding 3: accepted as a nit; `Property.Property` is the table primary key so the check is equivalent, only the label was stale. Relabelled in `23636442`.
- Finding 4: rejected. All callers pass a non-empty ProductCode and the wixproj already conditions the define on non-empty; error strings are not asserted anywhere.
- Finding 5: accepted (minor). A first attempt used a function-table snapshot/restore in `Invoke-Case`, but `Remove-Item function:script:X` is not reliably scope-aware in 5.1; replaced with body-local mocks (the pre-split design) in `23636442`.
- Finding 6: rejected; the harness now dot-sources the same lib the entry script uses, so the AST uniqueness guard has nothing to protect.
- Finding 7: accepted; `.gitignore` gains `build/windows/{artifacts,bin,obj}/` in `60d2a57f`.
- Finding 8 (`CLAUDE.md` dangling symlink): out of scope, report only.
- Review nits not acted on: `Get-Acl` mock now uses a psobject with a `Translate` ScriptMethod instead of `[SecurityIdentifier]::new` (equivalent, unexplained churn); `harness-tests.ps1` no longer parses `integration-tests.ps1` itself (still executed by the CI integration step).

## Outcomes & Retrospective

- Landed as scoped signed commits on `feat/windows-msi-packaging` (PR #236 still draft): `7a5b0811` (Manufacturer rename + MIRUMSI1009), `568155e0` (harness split), `e42c4dcd` (ARP DisplayName StrictMode guard), `23636442` (body-local per-case mocks + label), `60d2a57f` (gitignore), `5d0b6060` (plan progress), `f0578ec5` (rustls bump).
- CI round 1 (run 34877995025 on `5d0b6060`): `windows-check`, `windows-scope`, `test`, `tools` green; `lint` failed on RUSTSEC-2026-0285 (rustls 0.23.43, advisory published 2026-09-14, unrelated to this PR and also affecting `main`). Fixed with a lockfile-only bump to rustls 0.23.45.
- CI round 2 (run 34879810311 on `f0578ec5`): all five checks green. Preflight CLEAN.
- Retrospective: keeping mocks body-local (the pre-split design) was simpler and safer than function-table snapshot/restore, whose `Remove-Item function:script:X` scope handling is uncertain in 5.1. Out of scope, for follow-up: `CLAUDE.md` is a dangling symlink to `AGENTS.MD` (case mismatch).

## Context and Orientation

All paths below are relative to `/home/ben/miru/workbench5/repos/agent` unless stated. Run every git command from that directory.

Branch state: `feat/windows-msi-packaging` at `1fdf844c` (signed), equal to `origin/feat/windows-msi-packaging`; merge base with `origin/main` is `3f20eb06` and the branch is up to date with main. PR #236 is a draft; run 34802250357 on `1fdf844c` passed all five checks.

Uncommitted working tree (`git status --short`): modified `build/windows/miru-agent.wixproj` (adds `Code="MIRUMSI1009"` to the existing `<Error>` for `TestWixSource` without `FixturePayloadPath`), `build/windows/miru-agent.wxs` (line 4: `Manufacturer = "Miru"`), `build/windows/tests/harness-tests.ps1`, `build/windows/tests/integration-tests.ps1` (670 to 60 lines), `build/windows/tests/package-tests.ps1`; new `build/windows/tests/MsiTest.psm1`, `build/windows/tests/integration-lib.ps1`, `build/windows/tests/non-admin-probe.ps1`. These changes must never be discarded, reset, stashed, or checked out over.

Key files:

- `build/windows/tests/MsiTest.psm1`: PowerShell module (`Import-Module -Force`), StrictMode Latest and `$ErrorActionPreference = "Stop"` at module scope (module scope only; each entry script sets its own `Stop`: `integration-tests.ps1:11`, `harness-tests.ps1:5`, `package-tests.ps1:9`). Exports functions `Assert-True`, `Assert-Equal`, `New-MsiSessionLogDirectory`, `Open-MsiDatabase`, `Close-MsiDatabase`, `Get-MsiRows`, `Get-MsiPropertyValue`, `Test-MsiTable`, `Get-MsiContract`, `Get-MsiIdentity`, `Invoke-DotNetBuild`, `Assert-FailingFixtureContract`, and variables `$MsiProductName`, `$MsiManufacturer` (line 7, currently the stale value `"Miru Robotics"`), `$MsiUpgradeCode`, `$MsiExpectedSddl`, `$MsiExpectedDirectories`, `$MsiFixtureProductCodes`. Header comment says keep in sync with `miru-agent.wxs`.
- `build/windows/tests/integration-lib.ps1`: dot-sourced (not a module) so functions read caller-scope variables and so `harness-tests.ps1` can shadow them with `function script:X` mocks. Holds the lifecycle functions formerly inline in `integration-tests.ps1` (`Invoke-Msi`, `Build-IntegrationPackage`, `Get-MiruArpProducts`, `Invoke-NonAdminProbe`, `Invoke-IntegrationRun`, etc.). Sets `$script:WindowsTestRoot = $PSScriptRoot`.
- `build/windows/tests/non-admin-probe.ps1`: the old inline here-string probe as a file; copied by `Invoke-NonAdminProbe`.
- `build/windows/tests/package-tests.ps1`: imports the module; line 85 asserts `$MsiManufacturer` against the built MSI; line 178 expects `MIRUMSI1009` from a `missing-fixture-payload` build. At HEAD (`git show HEAD:build/windows/tests/package-tests.ps1`) line 209 has the inline assertion `Assert-Equal "Miru Robotics" $metadata.Manufacturer "Manufacturer"`.
- `build/windows/tests/harness-tests.ps1`: mocks the lib with `function script:X`; `Register-HarnessMocks` re-registers base mocks at each `Invoke-Case` (line 120), but per-case mocks defined inside case bodies are also `script:`-scoped and persist into later cases.
- `.github/workflows/ci.yml`: `windows_scope` classifier job, then `windows-check` on `windows-latest` (runs `harness-tests.ps1`, builds, runs `package-tests.ps1` and `integration-tests.ps1 -ConfirmDisposableTestMachine`). Triggers are `push` and `pull_request` on `main`/`release/*` only; there is no `workflow_dispatch`, so pushing the branch is what triggers the PR `synchronize` run. Native lane takes about 14 minutes.
- `build/windows/tests/workflow-tests.test.mjs`: run by the `tools` job via `node --test`; the only Windows-related test runnable on Linux.

Installer identity: Manufacturer appears only in `miru-agent.wxs:4`, `MsiTest.psm1:7`, `package-tests.ps1:85`, and the `Get-MsiContract` field. Install dir (`ProgramFiles64Folder\Miru\Agent`) and data dir (`CommonAppDataFolder\Miru`) do not derive from it. UpgradeCode `{B5ED0336-5F14-4308-A667-3CE8CDEF7D48}` is unchanged and `MajorUpgrade` keys on it, so upgrades from "Miru Robotics" builds are unaffected; the only visible effect is the ARP Publisher column. `Get-MiruArpProducts` filters on DisplayName `"Miru Agent"`, unchanged.

Constraints carried from the prior plan (`plans/completed/20260913-review-pr236-msi-foundation.md`): never run the PowerShell tests or native installer locally (CI is the execution environment); preserve UpgradeCode, component GUIDs, fixture ProductCode allowlist, retained customer state, and workflow triggers; no `workflow_dispatch`, no path-filter actions; keep the PR draft; do not change installer identities to simplify tests; `gh pr edit` is broken here (use `gh api -X PATCH repos/mirurobotics/agent/pulls/236` only if a body update is needed, which it is not). Preflight is capped at 3 rounds.

Commit signing: global config has `commit.gpgsign=true`, `gpg.format=ssh`, key `/home/ben/.ssh/github.pub`. The repo-local `.git/config` currently holds a malformed `commit.gpgsign = gpg.format`, which makes every `git commit` fail with `fatal: bad boolean config value 'gpg.format' for 'commit.gpgsign'`; neither `-S` nor `-c commit.gpgsign=true` bypasses a config parse error. This is a one-time environment repair (`git config --local --unset commit.gpgsign`) that requires the user's permission in this environment; the implementer must ask before changing local git config (M1 precondition step). Verify signatures with `git cat-file commit <sha> | grep -c gpgsig` (expect `1`); do not re-sign preemptively. Every commit body ends with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

Candidate findings to confirm or reject in M1 (numbers are reused in M3):

1. Blocking: `MsiTest.psm1:7` says `"Miru Robotics"` but wxs says `"Miru"`; `package-tests.ps1:85` would fail `windows-check`. Fixed by Commit B carrying `"Miru"`.
2. StrictMode Latest is now set in every script; `Get-MiruArpProducts` (`integration-lib.ps1:103`) reads `$_.DisplayName` on every Uninstall key, and keys without that value throw under StrictMode. Reached only on the manual production-smoke path (mocked in harness, not run in CI). Fix: `Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -eq "Miru Agent" }`.
3. `Get-MsiIdentity` asserts `$null -ne $value` where the old code asserted `1 -eq $rows.Count` with the same label; confirm the label still describes the check.
4. `Build-IntegrationPackage` error text changed and `-p:ProductCode` is passed only when non-empty; verify equivalence.
5. Harness per-case `script:` mocks leak into later cases (base mocks are re-registered, per-case ones are not). Fix: in `Invoke-Case`, snapshot `Get-ChildItem function:script:*` names before `& $Body` and remove any additions in a `finally`, or re-dot-source `integration-lib.ps1` before `Register-HarnessMocks`.
6. Old AST uniqueness guard removed; accepted as no longer needed.
7. `build/windows/artifacts/` not gitignored (minor; fix only if trivial).
8. `CLAUDE.md` is a dangling symlink to `AGENTS.MD` (case mismatch); out of scope, mention in the report only.

## Plan of Work

M1 (no edits): capture baseline SHAs and status, then review `origin/main...HEAD` plus the working tree per `$review` semantics: correctness, regressions, test coverage, installer identity implications of the rename, and behavioral equivalence of the harness refactor (old inline functions vs `integration-lib.ps1`, old AST extraction vs module import in `harness-tests.ps1`). Run the Linux-side consistency checks in Concrete Steps. Produce a findings list with accept/reject decisions; record them in the Decision Log.

M2, Commit A (`build(windows): rename Manufacturer to Miru and code fixture payload error`): stage `miru-agent.wxs` and `miru-agent.wixproj` from the working tree, and stage a version of `package-tests.ps1` derived from HEAD with only the line-209 string changed to `"Miru"`, using `git hash-object` + `git update-index --cacheinfo` so the working-tree copy (the harness-split version) is untouched. A lands before B so no commit has the module and wxs disagreeing on Manufacturer: A patches the HEAD inline assertion, and B introduces `MsiTest.psm1` already carrying `"Miru"`. Commit B (`test(windows): split MSI test harness into shared module and lib`): first edit `MsiTest.psm1:7` to `$MsiManufacturer = "Miru"`, then stage the three modified test scripts and three new files and commit. Verify each commit's tree with greps.

M3: apply accepted findings as edits to `integration-lib.ps1`, `harness-tests.ps1`, `MsiTest.psm1` (and `.gitignore` if finding 7 is accepted). Commit as `fix(windows): ...` for behavior fixes (finding 2) and `test(windows): ...` for harness fixes (findings 3, 5). Re-run the `$refine` loop (review, critique, plan, fix) on `origin/main...HEAD` until no accepted findings remain.

M4: push with plain `git push origin feat/windows-msi-packaging`, watch the triggered run to completion, fix from `--log-failed` with a new commit if needed (max 3 rounds), then run the delivery recheck.

## Concrete Steps

All commands run from `/home/ben/miru/workbench5/repos/agent`.

M1: baseline and review.

Precondition (one-time environment repair, before any commit in this plan):

    git config --show-origin --get-all commit.gpgsign
    # expect exactly one line: file:/home/ben/.gitconfig  true
    # if a second line 'file:.git/config  gpg.format' is present, every commit will fail.
    # Ask the user for permission, then run once:
    git config --local --unset commit.gpgsign
    git config --show-origin --get-all commit.gpgsign   # re-check: only the global 'true' line remains
    git commit --dry-run -m x >/dev/null && echo commit-ok  # expect commit-ok, not a fatal config error

Baseline:

    git status --short                       # expect the 5 M + 3 ?? files listed above, plus 'A ' or 'AM' for plans/backlog/20260914-refine-pr236-msi-harness.md
    git rev-parse HEAD origin/feat/windows-msi-packaging   # both 1fdf844c...
    git merge-base HEAD origin/main          # 3f20eb06...
    gh pr view 236 --json isDraft,headRefOid,baseRefName
    git diff --stat; git diff origin/main...HEAD --stat
    node --test build/windows/tests/workflow-tests.test.mjs   # expect all tests pass, 0 fail
    git diff --check; git diff origin/main...HEAD --check     # expect no output
    grep -rn 'Miru Robotics' build/windows                    # expect only MsiTest.psm1:7 (finding 1)
    grep -n 'MIRUMSI10' build/windows/miru-agent.wixproj build/windows/tests/package-tests.ps1
    command -v pwsh || echo no-pwsh                           # pwsh is absent here; skip parse checks

Function-usage consistency check: for each `Verb-Noun` called in the three test scripts, confirm a definition exists in `MsiTest.psm1` Export-ModuleMember or `integration-lib.ps1`.

    defs=$(grep -hoE '^function (script:)?[A-Z][A-Za-z]+-[A-Za-z]+' build/windows/tests/*.ps1 build/windows/tests/*.psm1 | sed -E 's/function (script:)?//' | sort -u)
    grep -hoE '\b(Assert|Get|New|Open|Close|Test|Invoke|Build|Add|Complete|Register)-[A-Z][A-Za-z]+' build/windows/tests/*.ps1 | sort -u | while read f; do echo "$defs" | grep -qx "$f" || echo "UNDEFINED: $f"; done
    # expect only built-in cmdlets (e.g. Get-ChildItem, Get-ItemProperty, Test-Path, New-Item) in the UNDEFINED list

Then write the findings list (accepted/rejected with file:line evidence).

The plan file is in the index (`A ` or `AM` in `git status --short`; the staged copy may be older than the working tree because the plan was revised after staging). It must leave the index before anything is staged for Commit A, or Commit A will include it. Re-stage the current working-tree copy and commit it now (requires the signing precondition above):

    git add plans/backlog/20260914-refine-pr236-msi-harness.md
    git diff --cached --stat        # expect exactly 1 file: the plan
    git diff --stat -- plans/backlog/20260914-refine-pr236-msi-harness.md   # expect no output (index == working tree)
    git commit -S -m 'docs(plan): add PR #236 harness refine plan' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
    git diff --cached --stat        # expect no output

If the user does not want the plan committed on this branch, use `git restore --staged plans/backlog/20260914-refine-pr236-msi-harness.md` instead (the file then shows as `??`); either way the index must be empty before M2.

M2: Commit A, without touching the working-tree `package-tests.ps1`.

    git add build/windows/miru-agent.wxs build/windows/miru-agent.wixproj
    blob=$(git show HEAD:build/windows/tests/package-tests.ps1 | sed 's/Assert-Equal "Miru Robotics"/Assert-Equal "Miru"/' | git hash-object -w --stdin)
    git update-index --cacheinfo 100644,$blob,build/windows/tests/package-tests.ps1
    git diff --cached --stat        # expect exactly 3 files, tiny diff
    git commit -S -m 'build(windows): rename Manufacturer to Miru and code fixture payload error' -m 'Manufacturer is now "Miru" (ARP Publisher only; UpgradeCode and directories unchanged). The wixproj error for TestWixSource without FixturePayloadPath now carries Code MIRUMSI1009 so package-tests can match it.' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
    git grep -n 'Miru Robotics' HEAD -- build/windows   # expect no output
    git status --short                                  # package-tests.ps1 still M (working tree differs from new HEAD)

Commit B.

    sed -i 's/^\$MsiManufacturer = "Miru Robotics"/$MsiManufacturer = "Miru"/' build/windows/tests/MsiTest.psm1
    git add build/windows/tests/
    git diff --cached --stat        # expect 6 files: 3 modified, 3 new
    git commit -S -m 'test(windows): split MSI test harness into shared module and lib' -m 'Move shared MSI helpers into MsiTest.psm1, integration lifecycle into dot-sourced integration-lib.ps1, and the non-admin probe into its own file. package-tests now also builds the fixture MSI and expects MIRUMSI1009 for a missing payload.' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
    git status --short              # expect clean (or only '?? plans/backlog/...' if the plan was unstaged rather than committed)
    for c in HEAD~1 HEAD; do echo $c; git cat-file commit $(git rev-parse $c) | grep -c gpgsig; git grep -c 'Miru Robotics' $c -- build/windows || true; done   # expect 1 and no matches for each

M3: refine loop. Edit per accepted findings, then:

    node --test build/windows/tests/workflow-tests.test.mjs
    git diff --check
    git add build/windows/tests/integration-lib.ps1 && git commit -S -m 'fix(windows): guard ARP DisplayName reads under StrictMode' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
    git add build/windows/tests/harness-tests.ps1 build/windows/tests/MsiTest.psm1 && git commit -S -m 'test(windows): isolate per-case harness mocks' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'

Adjust subjects to what was actually fixed; one commit per concern. Do not use interactive staging (`git add -p`); if one file needs changes for two concerns, make and commit them as two sequential edits. Repeat review until no accepted findings remain.

M4: push and watch.

    git log --oneline origin/main..HEAD
    git push origin feat/windows-msi-packaging
    gh run list --branch feat/windows-msi-packaging --event pull_request --limit 5 --json databaseId,headSha,status --jq '.[] | "\(.databaseId) \(.headSha) \(.status)"'
    # expect lines like '34802250357 <40-hex sha> in_progress'; pick the run whose sha == git rev-parse HEAD (push and pull_request both trigger; use the pull_request run so gh pr checks 236 matches)
    gh run watch <run-id> --exit-status
    gh run view <run-id> --json headSha,status,conclusion,jobs
    gh pr checks 236

On failure: `gh run view <run-id> --log-failed`, fix, commit (`fix(windows): ...`), push again. Stop after 3 rounds and report.

## Validation and Acceptance

Preflight must report CLEAN — CI green on the pushed branch head — before this task is reported complete. Concretely, all of the following must hold:

- `node --test build/windows/tests/workflow-tests.test.mjs` passes on the final HEAD; `git diff origin/main...HEAD --check` prints nothing.
- Per-commit self-consistency: `git grep -n 'Miru Robotics' <sha> -- build/windows` is empty for every new commit; at Commit A the tree contains no `MsiTest.psm1` and `package-tests.ps1` asserts `"Miru"`; from Commit B onward `MsiTest.psm1` exports `$MsiManufacturer = "Miru"` and every function called in the three test scripts is defined in the module or lib.
- Every new commit is signed and attributed (baseline is `1fdf844c`, the pre-plan branch head): `for s in $(git rev-list 1fdf844c..HEAD); do git cat-file commit $s | grep -c gpgsig; done` prints `1` per line, and `git log --format='%h %(trailers:key=Co-Authored-By)' 1fdf844c..HEAD` shows the trailer on each new commit.
- `git status --short` is clean and the harness split files exist in `git ls-tree -r HEAD --name-only build/windows/tests` (`MsiTest.psm1`, `integration-lib.ps1`, `non-admin-probe.ps1`).
- `git rev-parse HEAD` equals `gh pr view 236 --json headRefOid -q .headRefOid` and equals `gh run view <run-id> --json headSha -q .headSha`.
- `gh run view <run-id> --json jobs -q '.jobs[] | "\(.name) \(.conclusion)"'` shows `success` for `lint`, `test`, `tools`, `windows-scope`, and `windows-check`; `gh pr checks 236` shows all pass.
- `gh pr view 236 --json isDraft -q .isDraft` prints `true`.

Test steps: `windows-check` on the pushed head runs `harness-tests.ps1`, then `package-tests.ps1` (which fails before Commit B's Manufacturer fix with `Manufacturer` mismatch and passes after; the new `missing-fixture-payload` case expects `MIRUMSI1009`), then `integration-tests.ps1 -ConfirmDisposableTestMachine`. `tools` runs `workflow-tests.test.mjs`. The harness mock-isolation fix (finding 5) is validated by `harness-tests.ps1` still passing with cases run in the existing order; if a case is added to exercise isolation, it must fail before the fix and pass after.

## Idempotence and Recovery

- M1 is read-only and repeatable. All greps and `node --test` are safe to rerun.
- Never run `git reset --hard`, `git stash`, or `git checkout -- <path>` while the harness split is uncommitted. Commit A stages `package-tests.ps1` via `git update-index --cacheinfo`, which leaves the working tree untouched; if the index gets confused, `git restore --staged .` only unstages and is safe.
- A wrong intermediate commit that has not been pushed can be fixed with `git commit --amend` (re-signed automatically) or `git reset --soft HEAD~1` (keeps the working tree). Once pushed, add a new commit instead.
- If `git push` is rejected, `git fetch origin` and inspect `git log HEAD..origin/feat/windows-msi-packaging`; never force-push except `--force-with-lease=feat/windows-msi-packaging:<inspected-sha>`, and plain push is expected to suffice here.
- CI failure: fix from `gh run view <id> --log-failed`, new commit, push; superseded runs are cancelled by workflow concurrency. Cap at 3 rounds, then report status and the failing job.
- If a re-review finds nothing but CI still fails on a native-only issue, the fix commit goes in M4, not M3, and the Decision Log records why.

Revision 2026-09-14: added signing-config precondition and plan-file commit step, removed interactive staging, pinned attribution range to 1fdf844c, moved ordering rationale to Plan of Work, dropped the resolved `$ErrorActionPreference` candidate (old 7; the artifacts-gitignore item renumbered 8 to 7).
