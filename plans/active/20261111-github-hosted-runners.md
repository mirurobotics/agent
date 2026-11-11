# Run agent's Linux CI on GitHub-hosted `ubuntu-latest` runners

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | Change the 11 Linux Blacksmith runner labels in `ai-review.yml`, `ci.yml`, `codeql-analysis.yml` and `release.yml` under `.github/workflows/` to `ubuntu-latest`. No other source file changes. |
| `docs/`, `gotools/`, `setup-cli/` | read-only | Exemplar mirurobotics/docs#320 and siblings mirurobotics/gotools#69 and mirurobotics/setup-cli#164 made the same change and settled the label, the inline comment and the label cleanup. |
| `cli/` | read-only | Precedent: already passes `ubuntu-latest` to the same pinned reusable AI review workflow. |
| `infra/` | read-only | Trust policies for the OIDC roles the `test` job assumes (see Context and Orientation). |

This plan lives in `agent/plans/` because the only edits are in the agent repo. Work on the checked-out branch `claude/youthful-lamport-yn0sf2` (base: `main` at 4f86f04).

Out of scope: the Windows jobs and `builder.yml`, which stay on Blacksmith (see Left out). This is the last repo in the pattern.

## Purpose / Big Picture

mirurobotics/agent is a public repository. GitHub-hosted standard runners are free for public repositories (Linux: 4 vCPU, 16 GB) and do not draw from the GitHub Enterprise Cloud included compute allowance, while every Blacksmith minute spent here is billed. After this change every Linux job in the agent workflows except the Docker builder-image job runs on GitHub's `ubuntu-latest` runner, so those minutes cost nothing. The jobs keep their names and steps, so branch protection's required checks are unchanged. A reader sees it working when the Linux job logs on the PR show "Runner name: GitHub Actions …" and an image of `ubuntu-24.04` instead of a Blacksmith runner, and all checks are green.

This PR follows the exemplar mirurobotics/docs#320 and the siblings mirurobotics/gotools#69 and mirurobotics/setup-cli#164, and cites them in its body.

## Decisions

### Precedent

| New file | Mirrors |
|----------|---------|
| `plans/active/20261111-github-hosted-runners.md` | `docs/plans/active/20261111-github-hosted-runners.md` (the exemplar's plan), the gotools and setup-cli plans of the same name, and `plans/completed/20261110-app-run-tests-local-mqtt-broker.md` |

No source file is added. Each edited line mirrors docs#320, gotools#69 and setup-cli#164, written as `runs-on: ubuntu-latest # standard GitHub-hosted runners are free in public repos`. The `with: runs-on:` input to the AI review workflow mirrors `cli/.github/workflows/ai-review.yml`, which passes `ubuntu-latest` to `mirurobotics/.github/.github/workflows/ai-review.yml` at the same SHA agent pins (`bf09161`).

### Left out

- The Windows jobs `windows-check` (`ci.yml:51`), `windows-agent-build` (`ci.yml:132`) and `windows-release-build` (`release.yml:61`), on `blacksmith-4vcpu-windows-2025`: the Rust compile there is several times faster than on `windows-latest`, as the comment above `windows-agent-build` (`ci.yml:126`) records, and `build/windows/README.md` relies on that. Moving them is a cost-versus-speed decision of its own.
- `builder.yml:20` (`build-and-push`): it uses `useblacksmith/setup-docker-builder` and `useblacksmith/build-push-action`, which need Blacksmith's builder. Proposed follow-up: replace them with `docker/setup-buildx-action` and `docker/build-push-action`, then move the label. It runs only when `build/Dockerfile.builder` or `builder.yml` change on `main` or `release/*`, so its cost is small.
- Lines already on GitHub-hosted runners stay as they are: `windows-package` (`ci.yml:164`, `windows-latest`) and `windows-sign` (`release.yml:119`, `windows-2025`).
- Runner-label configuration cleanup: agent has no `.github/actionlint.yml`, no root `actionlint.yml`, and no yamllint, Prettier or super-linter config, and no workflow or `scripts/lint.sh` step lints workflow YAML. So nothing registers the Blacksmith labels and nothing checks line length. The new lines are 84 and 86 characters; the workflows already hold 70 lines over 80 characters.
- Timeout changes: no moved job's recent duration is near half its timeout (see Context and Orientation). Raise one only if CI on this PR actually times out.
- Larger or ARM runners, concurrency changes, a different cache (such as sccache), and bumping pinned SHAs: not needed to move off Blacksmith.

### Open questions

- Raise `timeout-minutes` on `test` (20) or `tools` (10) ahead of time, or keep a job on Blacksmith, because GitHub's CPUs are slower per core and the first runs start with a cold cache? Default: no. Keep the label swap on every listed job and change nothing else. If a job on this PR times out, record its duration in Surprises and raise that job's timeout in this PR; keeping it on Blacksmith is the fallback only if a raised timeout still leaves it slower than a reviewer accepts.

## Progress

- [ ] Move this plan to `plans/active/`.
- [ ] Replace the 11 Linux Blacksmith labels with `ubuntu-latest` plus the inline comment.
- [ ] Commit the milestone (message in Concrete Steps).
- [ ] Push and open the draft PR; CI and a dispatched CodeQL run pass on the branch head.
- [ ] Preflight reports `CLEAN`.
- [ ] After merge: AI Review on the next PR, the nightly `ci.yml` and CodeQL runs on `main` pass on `ubuntu-latest`.
- [ ] On the next release tag: `release.yml` passes.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- 2026-11-11: Label, inline comment and core-count reasoning are taken unchanged from the exemplar docs#320, whose review settled them, as gotools#69 and setup-cli#164 did.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The agent is a Rust workspace (`Cargo.toml`, crate `agent/`, generated clients under `libs/`). A runner is the virtual machine a GitHub Actions job runs on, chosen by the job's `runs-on` key. `blacksmith-2vcpu-ubuntu-2404` and `blacksmith-4vcpu-ubuntu-2404` are labels served by Blacksmith, a paid third-party runner provider; `ubuntu-latest` is GitHub's own hosted Ubuntu runner, which resolves to `ubuntu-24.04` today, the same release. In public repos `ubuntu-latest` has 4 vCPU and 16 GB, so the 4-vCPU jobs keep their core count and the 2-vCPU jobs gain cores.

The 11 lines to change (line numbers and job names as of `main` at 4f86f04):

- `ai-review.yml:30`, job `ai-review`, `blacksmith-2vcpu-ubuntu-2404`. Not a `runs-on` key but a `with: runs-on:` input to the reusable workflow `mirurobotics/.github/.github/workflows/ai-review.yml`, which uses it as its job's runner.
- `ci.yml`, all `blacksmith-4vcpu-ubuntu-2404`: `:28` `lint` (no timeout; `scripts/lint.sh`: import linter, `cargo fmt`, machete, diet, audit, clippy), `:108` `windows_package_scope`, shown as `windows-package-scope` (no timeout; a `dorny/paths-filter` step), `:195` `test` (`timeout-minutes: 20`; `scripts/covgate.sh`, the full test suite under `cargo-llvm-cov` with coverage gates), `:245` `tools` (`timeout-minutes: 10`; lint and coverage gate of the import linter in `tools/lint/`), `:276` `debian-package` (`timeout-minutes: 10`; ShellCheck and `build/debian/tests/postinst-test.sh`, which runs a `debian:bookworm-slim` container with `docker run`).
- `codeql-analysis.yml:20` `changes` ("Check for new commits", `timeout-minutes: 5`, `blacksmith-2vcpu-ubuntu-2404`) and `:36` `codeql` (no timeout, `blacksmith-4vcpu-ubuntu-2404`; CodeQL for `rust, actions` with autobuild).
- `release.yml:18` `check-main` ("Verify tag is on main") and `:225` `release` (both `blacksmith-4vcpu-ubuntu-2404`, no timeout), and `:259` `notify` ("Notify #deploys", `blacksmith-2vcpu-ubuntu-2404`). `release` runs `build/release.sh`, which runs `docker build -f build/Dockerfile` on the public `ghcr.io/mirurobotics/agent-builder` image; GoReleaser inside it compiles the x86_64 and aarch64 Linux binaries with no build cache.

`builder.yml:20` and the three Windows labels also contain `blacksmith-` and must stay (see Left out).

Recent durations on Blacksmith, from the GitHub API (the five nightly `schedule` runs of `ci.yml` on `main` from 2026-11-07 to 2026-11-11, twelve successful `pull_request` runs from 2026-11-10 to 2026-11-11, the last six CodeQL runs and the last three releases):

| Job | Timeout | Nightly on `main` (releases for `release.yml`) | PR runs | Share of timeout |
|-----|---------|-------------------|---------|------------------|
| `ci.yml` `lint` | none (360) | 0:47–1:20 | 0:49–1:27 | under 1% |
| `ci.yml` `windows-package-scope` | none (360) | 0:04–0:05 | 0:04–0:17 | under 1% |
| `ci.yml` `test` | 20 | 1:21–1:44 | 1:35–2:34 | 13% at most |
| `ci.yml` `tools` | 10 | 0:41–0:45 | 0:45–1:19 | 13% at most |
| `ci.yml` `debian-package` | 10 | 0:08–0:10 | 0:08–0:17 | 3% at most |
| CodeQL `changes` | 5 | 0:05–0:06 | — | 2% |
| CodeQL `codeql` | none (360) | 4:31–6:26 | — | 2% |
| `release.yml` `check-main` | none (360) | 0:07–0:08 | — | under 1% |
| `release.yml` `release` | none (360) | 5:23–9:03 | — | 3% |
| `release.yml` `notify` | none (360) | 0:05–0:06 | — | under 1% |

No moved job is above a third of its timeout, let alone half, so none is flagged. Those times are with warm caches. Every Rust job caches with `Swatinem/rust-cache`, which stores through `actions/cache`. On Blacksmith the job log says "Enabling Blacksmith transparent cache": Blacksmith redirects `actions/cache` to its own colocated store, so the GitHub cache likely holds nothing usable and the first runs on `ubuntu-latest` start cold. (The cache API is not reachable from this session, so this was not checked.) In the 2026-11-11 PR run, `test` restored about 540 MB and still compiled the three workspace crates in 53 s; cold, it compiles all about 460 packages in `Cargo.lock` with coverage instrumentation, which is the run to watch against the 20-minute timeout. `lint` restores about 316 MB and `tools` about 117 MB (73 packages in `tools/lint/Cargo.lock`). GitHub's cache limit is 10 GB per repository, evicting least recently used entries; the three Linux caches total about 1 GB per scope, and the Windows jobs keep caching on Blacksmith. `ci.yml` does not run on push to `main`; its nightly `schedule` run on `main` saves the caches that PR runs fall back to, so PRs warm up after the first night. Blacksmith also caches Docker images, so on GitHub `debian-package` and `release` pull their base images each run (the builder image pull took 15 s on Blacksmith).

Runner-specific setup: only `test` uses credentials. It authenticates with GitHub OIDC tokens to Google Cloud (`google-github-actions/auth`, workload identity) and AWS (`aws-actions/configure-aws-credentials`) for the real-GCS and real-S3 upload tests. GitHub issues the OIDC token, not the runner, and the trust rules in `infra/deploy/terraform/gcp-ci-storage-test.tf` (`attribute_condition` on `assertion.repository`) and `infra/cicd/deploy/agent-ci-role.tf` (`sub` like `repo:mirurobotics/agent:*`) check only the repository, not `runner_environment` or source IP; no IP condition exists in either. Those steps already skip on Dependabot and fork PRs, which is unchanged. No moved job uses `useblacksmith/*` actions or sticky disks. Both runners run jobs as the `runner` user with passwordless `sudo` and Docker available.

When each workflow runs, which decides what the PR itself can prove:

- `ci.yml`: on `pull_request` to `main` and `release/*`, push to `release/*`, nightly (`cron: '17 6 * * *'`), `workflow_dispatch`, and `workflow_call` from `release.yml`. All five moved jobs run on the PR.
- `codeql-analysis.yml`: push to `release/*`, nightly (`cron: '31 7 * * *'`) and `workflow_dispatch`; nothing calls it. It does not run on PRs, so it is dispatched against the branch. The `codeql-gate` action skips the scan only on scheduled runs where `main` has not moved, so a dispatched run always scans.
- `ai-review.yml`: on `pull_request_target`, which runs the file from the base branch. The PR's own AI review still runs on Blacksmith; the new runner takes effect on the first PR after merge.
- `release.yml`: on push of any tag only. It calls `ci.yml`, builds and signs the Windows binaries, and publishes the GitHub release with GoReleaser, so it must never be triggered to test this change; it is verified on the next real release.

## Plan of Work

In `ai-review.yml`, `ci.yml`, `codeql-analysis.yml` and `release.yml` under `.github/workflows/`, replace each `blacksmith-2vcpu-ubuntu-2404` and `blacksmith-4vcpu-ubuntu-2404` with `ubuntu-latest # standard GitHub-hosted runners are free in public repos`, leaving indentation and every other line untouched. Do not touch `builder.yml` or the `blacksmith-4vcpu-windows-2025` lines. One commit, one milestone.

## Concrete Steps

All commands run from the agent repo root, `/home/user/agent`.

1. Confirm the branch and the starting count, and move this plan:

       git branch --show-current          # expect: claude/youthful-lamport-yn0sf2
       grep -c 'blacksmith-[24]vcpu-ubuntu-2404' .github/workflows/*.yml
       # expect: ai-review.yml:1 builder.yml:1 ci.yml:5 codeql-analysis.yml:2 release.yml:3
       mkdir -p plans/active
       git mv plans/backlog/20261111-github-hosted-runners.md plans/active/

   If the counts read `builder.yml:1` and 0 for every other file, the change is already applied; skip to step 4. If the plan is already in `plans/active/`, skip the `git mv`.

2. Replace the labels in the four files only (not `builder.yml`):

       cd .github/workflows
       sed -i 's/runs-on: blacksmith-[24]vcpu-ubuntu-2404$/runs-on: ubuntu-latest # standard GitHub-hosted runners are free in public repos/' ai-review.yml ci.yml codeql-analysis.yml release.yml
       cd ../..

3. Check the result (read-only inspection, not validation tooling):

       grep -rn 'blacksmith' .github/workflows/
       # expect only: builder.yml:20, :30, :49; ci.yml:51, :132; release.yml:61
       grep -c 'runs-on: ubuntu-latest # standard GitHub-hosted runners are free in public repos' .github/workflows/*.yml
       # expect: ai-review.yml:1 builder.yml:0 ci.yml:5 codeql-analysis.yml:2 release.yml:3
       git diff --stat -- .github           # expect: 4 files changed, 11 insertions(+), 11 deletions(-)

4. Commit the milestone:

       git add .github/workflows plans/
       git commit -m "ci: run agent Linux workflows on GitHub-hosted ubuntu-latest runners"

5. Push and open the draft PR (via `$pr`). In the PR body, link docs#320, gotools#69 and setup-cli#164, list the Windows jobs and `builder.yml` as staying on Blacksmith with the `builder.yml` follow-up, and state which workflows can only be verified after merge or on the next release.

6. Dispatch CodeQL against the branch, which runs on GitHub, not locally:

       gh workflow run codeql-analysis.yml --ref claude/youthful-lamport-yn0sf2
       gh run list --workflow codeql-analysis.yml --branch claude/youthful-lamport-yn0sf2 --limit 1

7. Run `$preflight` until it reports `CLEAN`. Fix any failure from `gh run view --log-failed`. Record the `test`, `tools`, `lint` and `codeql` durations of the first run on `ubuntu-latest` in Surprises & Discoveries.

No local `cargo`, `scripts/*.sh`, linters, builds or tests are run. Never dispatch or tag `release.yml`.

## Validation and Acceptance

This change adds no test; it only changes CI configuration. The test is that every touched workflow still runs and passes on GitHub's runner:

- `ci.yml` on the PR: `lint`, `windows-package-scope`, `test`, `tools` and `debian-package` pass, and each one's "Set up job" log section shows a GitHub-hosted runner with image `ubuntu-24.04`, not Blacksmith. In `test`, the "Authenticate to Google Cloud" and "Authenticate to AWS" steps succeed and the covgate run includes the real-GCS and real-S3 upload tests rather than skipping them. The Windows jobs still run on Blacksmith and `windows-package` on `windows-latest`, as before. If `test` or `tools` reaches its timeout, record the duration in Surprises and raise that timeout in the same PR (see Open questions).
- `codeql-analysis.yml`: the dispatched run from step 6 passes `changes` and runs `codeql` on `ubuntu-latest`.
- Gate: `$preflight` must report `CLEAN`, meaning CI is green on the pushed branch head, before the PR leaves draft.

Verified only after merge, and recorded in Outcomes:

- `ai-review.yml`: the AI review on the next PR after merge runs on `ubuntu-latest` and posts its `ai-review` status. The cli repo already passes `runs-on: ubuntu-latest` to the same reusable workflow at the same SHA.
- The next nightly `ci.yml` and CodeQL runs on `main` pass on `ubuntu-latest`; the `ci.yml` run seeds the `main` cache.
- `release.yml`: the next tag runs `check-main`, the called `ci` jobs, `release` and `notify` on `ubuntu-latest`, publishes the GitHub release and posts to #deploys. Expect `release` to take longer than its 5–9 minutes on Blacksmith; it has no timeout.

## Idempotence and Recovery

Step 2 is safe to rerun; once the labels are gone the `sed` matches nothing, and it never touches `builder.yml` or the Windows labels. Before committing, `git checkout -- .github/workflows` restores the files. After merge, a failure that only `ubuntu-latest` causes (a timeout, or `release` running out of disk) is fixed forward in a new PR, for example by raising a timeout, or that job's label is reverted with `git revert <sha>` in a PR. Never force-push `main`.
