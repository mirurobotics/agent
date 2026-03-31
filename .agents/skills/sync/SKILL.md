---
name: sync
description: Sync local repo after a PR merge by switching to main, fetching with prune, pulling, and deleting stale local branches. Use when asked to sync, clean up after merge, or delete merged branches.
---

# Sync Workflow

## Inputs
- `repo` (optional): constrain to a specific submodule path (e.g., `core`, `backend`, or `.` for root).
- `branch` (optional): explicit branch name(s) to delete. If omitted, the skill auto-detects.

## Procedure
1. Resolve repo scope. If `repo` is given, `cd` into that submodule root. Otherwise use the current working directory.
2. Check for uncommitted changes (`git status --porcelain`). If the working tree is dirty, warn the user and stop.
3. Switch to main if not already there: `git checkout main`.
4. Fetch and prune: `git fetch --prune`.
5. Pull: `git pull`.
6. Detect stale branches (see below).
7. Delete stale branch(es): `git branch -d <branch>`.
8. Report what was done.

## Branch detection

When no explicit `branch` is given, always detect what to delete:

1. Run `git branch --merged main` to list local branches fully merged into main (exclude `main` and `master`).
2. If `gh` is available, run `gh pr list --state merged --author @me --limit 10 --json headRefName,title,mergedAt` to add PR context (title, merge date) to each candidate.
3. If exactly one candidate, delete it. If multiple, present the list and ask the user which to delete (or offer to delete all).
4. If none, report "nothing to clean up" and stop.

## Safety Rules
- Never delete `main` or `master`.
- Always use `git branch -d` (lowercase), never `-D`. This lets Git refuse if the branch has unmerged work.
- Never switch branches with a dirty working tree — warn and stop instead.
- Never force-push or reset.

## Output Contract
1. Confirmation of fetched, pulled, and branches deleted.
2. Name(s) of the deleted branch(es).
3. If deletion failed (unmerged work), report the error and suggest next steps.
