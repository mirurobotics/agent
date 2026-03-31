---
name: commit
description: Prepare and execute high-quality Conventional Commits from working tree changes with repository-aware commit scoping (root repo and submodules) and atomic commit planning. Use when asked to commit, split commits, draft commit messages, or organize staged/unstaged changes into safe commit units.
---

# Commit Workflow

## Inputs
- `repo` (optional): constrain scope to one repo path (for example `backend`, `frontend`, `core`, or `.` for the root repo).

## Procedure
1. Resolve repo scope.
2. Gather per-repo git context: `git status`, `git diff`, `git diff --cached`, `git log --oneline -10`.
3. Build an atomic commit plan.
4. Execute commits from the correct repo root and report hashes.

## Commit Rules
- Never use broad staging (`git add .`, `git add -A`).
- Never push unless explicitly requested.
- Never mix unrelated concerns in one commit.
- Never discard user changes.

## Conventional Commit Rules
- Format: `type(scope): summary`
- Preferred types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`, `ci`, `build`, `style`
- Use imperative, concise subjects.
- Keep subject under 72 characters when practical.

## Output Contract
1. Execution summary (hashes, repos, remaining changes).
2. Explicit list of skipped files (if any) and why.
