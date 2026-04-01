---
name: pull-request
description: Create and manage pull requests with submodule-aware branch handling, auto-generated titles and descriptions from commit history, and safety-first push behavior. Use when asked to open a PR, check PR status, push a branch, or prepare changes for review.
---

# Pull Request Workflow

## Inputs
- `repo` (optional): constrain scope to one repo path (for example `backend`, `frontend`, `core`, or `.` for the root repo).
- `mode` (optional): `create` (default), `status`, or `update`.
- `branch` (optional): branch name to use. Auto-generated from commits if omitted.
- `base` (optional): target branch. Defaults to `main`.
- `draft` (optional): open as draft PR when `true`.

## Procedure

### 1. Resolve repo scope
- If `repo` is given, operate in that submodule directory.
- If omitted, infer from the user's working context or uncommitted changes.
- Verify the repo has a GitHub remote (`gh repo view`).

### 2. Gather context
- `git status` — confirm working tree state (warn if uncommitted changes exist).
- `git branch --show-current` — identify current branch.
- `git log --oneline main..HEAD` — collect commits that will be in the PR.
- `git diff main...HEAD --stat` — summarize file changes.

### 3. Create branch (if on main)
- If currently on `main`, create and checkout a new branch.
- Follow branch naming conventions: `<prefix>/<short-description>` using kebab-case.
- Derive prefix from commit types when possible (`feat` commits -> `feat/`, `fix` commits -> `fix/`).
- If branch name cannot be inferred, ask the user.

### 4. Push branch
- Push with `-u` to set upstream tracking: `git push -u origin <branch>`.
- Never force push. If push is rejected, report the conflict and stop.

### 5. Generate PR content
- **Title**: must use Conventional Commits format: `type(scope): summary`. Single commit -> use its subject line directly. Multiple commits -> determine the dominant type (`feat` if any feats, `fix` if all fixes, `chore`/`refactor`/etc. as appropriate) and summarize the theme. Use the same types as commits: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`, `ci`, `build`, `style`. Scope is optional but encouraged when changes target a specific area. Keep under 72 characters.
- **Body**: structured format:
  ```
  ## Summary
  <bulleted list derived from commit messages>
  ```
- Do not ask for approval — create the PR immediately.

### 6. Create PR
- Use `gh pr create --title "<title>" --base <base> --body "<body>"`.
- Add `--draft` flag when requested.
- Report the PR URL on success.

### 7. Status and update modes
- `status`: run `gh pr status` in the target repo, or `gh pr view <number>` for a specific PR.
- `update`: push latest commits and run `gh pr view` to confirm CI status after push.

## Safety Rules
- Never force push (`--force`, `--force-with-lease`).
- Never push directly to protected branches (`main`, `production`, `uat`, `release/*`).
- Create PRs without waiting for approval — just do it.
- Never mix uncommitted changes into PR scope — warn and stop if working tree is dirty.
- Never delete branches without explicit request.

## Branch Naming Rules
- Format: `<prefix>/<short-description>` using kebab-case.
- Valid prefixes: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`, `ci`, `build`, or a person's name.
- Keep branch names short and descriptive.

## Output Contract
1. Execution summary (PR URL, branch pushed).
3. Warnings for any skipped steps (dirty working tree, existing remote branch, push rejection).
