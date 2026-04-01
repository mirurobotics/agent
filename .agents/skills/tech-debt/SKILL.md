---
name: tech-debt
description: Discover, document, resolve, and maintain tech debt items in per-repo TECH_DEBT.md files. Use when asked to find tech debt, document debt, fix a tech debt item, or audit a submodule for debt.
---

# Tech Debt Workflow

Tech debt is tracked in a `TECH_DEBT.md` at the root of each repo (submodule). Each repo owns its own file — never put one repo's debt in another repo's file.

## Inputs
- `scope` (optional): files, module, package, or repo. Defaults to repo inferred from working context.
- `mode` (optional): `discover` (default), `audit`, or `resolve`.
- `items` (optional): specific TD-xxx ID(s). Required for `resolve` mode.

## References

Use these skills as analytical lenses during discovery:
- `$design` for convention deviations, structural issues, complexity.
- `$reliability` for error handling and lifecycle gaps.
- `$security` for security hygiene debt.
- `$performance` for performance-related structural debt.
- `$test` for test coverage gaps.

Per-repo conventions (load for target repo):
- `ARCHITECTURE.md` — intended structure and conventions.
- `AGENTS.md` — repo-specific coding conventions.
- Existing `TECH_DEBT.md` — avoid duplicates, determine next ID.

## Procedure

### 1. Resolve scope and mode
- Default scope: infer repo from user's working context.
- Default mode: `discover`.
- If mode is `resolve`, require `items` input.
- Map scope to correct `TECH_DEBT.md` using the repo mapping table below.

### 2. Load repo conventions
- Read target repo's `ARCHITECTURE.md` (intended structure and conventions).
- Read target repo's `AGENTS.md` (repo-specific coding conventions).
- Read existing `TECH_DEBT.md` (avoid duplicates, determine next ID).
- Identify primary language(s) in scope.

### 3. Discover
Systematic scan through five lenses:

1. **Structure scan** → `structure`, `inconsistency`: Compare directory layout and file organization against `ARCHITECTURE.md`. Look for files in wrong locations, missing expected directories, patterns that deviate from documented architecture.

2. **Dead code scan** → `dead-code`: Unused exports, unreferenced files, stale imports, unreachable branches, commented-out code blocks, orphaned config entries.

3. **Consistency scan** → `inconsistency`: Patterns followed in most of the codebase but violated in specific places. Naming conventions, API style, error handling patterns, file structure conventions.

4. **Complexity scan** → `complexity`: Duplicated logic that should be shared, deeply nested control flow, functions/modules doing too many things, unclear boundaries.

5. **Test coverage scan** → `missing-tests`: Features with mutations or complex logic that have no or weak test coverage.

**Actionability filter** — apply to every finding before documenting:
- Must be worth the churn — don't document trivial deviations.
- Must match real codebase conventions, not theoretical ideals.
- Must be self-contained — another agent can fix it without asking questions.

After scanning, formulate each finding as a well-formed item (see Item Format below), assign next sequential ID, write to `TECH_DEBT.md` (create with standard header if needed), and update the summary table.

### 4. Audit
1. Read every item in target `TECH_DEBT.md`.
2. Check referenced files for each item to verify the issue still exists.
3. If resolved: remove item and table row, note as stale.
4. If still valid but description outdated: update description.
5. If scope changed: update scope tag.

### 5. Resolve
1. Read specified item(s) from repo's `TECH_DEBT.md`.
2. Verify each item is still valid — check the files referenced in Location.
3. If already resolved: remove the item and its table row, note as stale.
4. If still valid: implement the fix, then remove item and table row from `TECH_DEBT.md`.
5. Commit with ID reference — e.g., `refactor(config-types): colocate hooks (TD-050)`.
6. If scope changed during implementation: update scope or split into multiple items.

## Output Contract

| Mode | Output |
|------|--------|
| discover | Items written (ID, title, category, scope for each). Repos where `TECH_DEBT.md` was created or modified. |
| audit | Items confirmed valid. Items removed as stale (with reason). Items updated. |
| resolve | Items resolved (ID + fix summary). Commit references. Items split or re-scoped. |

## Repo Mapping


When a `TECH_DEBT.md` doesn't exist yet for a repo, create it with this header:

```markdown
# Tech Debt — {Repo Name}

Items are ordered by ID. Gaps in IDs are expected — never renumber.

| ID | Title | Category | Scope |
|----|-------|----------|-------|
```

## What Qualifies as Tech Debt

- Deviations from conventions in `ARCHITECTURE.md` or repo-level `AGENTS.md`
- Dead code — unused files, exports, aliases, unreachable branches
- Missing test coverage for features with mutations or complex logic
- Structural inconsistencies — files/dirs that don't match project conventions
- Unnecessary complexity — code harder to maintain than it needs to be

Tech debt is **not**: feature requests, bugs, product decisions, or performance optimizations without a correctness concern.

## Item Format

Every item must be self-contained — another agent should be able to fix it without asking questions.

### Template

```markdown
### TD-xxx: Short descriptive title `category` `scope`

**Location:** `path/to/affected/files/` (paths relative to repo root)

**Current state:** Concrete description of what exists now. Include file paths, patterns observed, counts where relevant.

**Desired state:** What it should look like after the fix. Reference ARCHITECTURE.md or AGENTS.md conventions where applicable.

**Notes:** Optional — gotchas, dependencies on other items, why this matters.
```

### Rules

1. **ID**: Scan the repo's `TECH_DEBT.md` for the highest existing `TD-xxx` number and increment by 1. IDs are never reused. Each repo has its own independent ID sequence.
2. **Category**: Exactly one of: `inconsistency`, `dead-code`, `missing-tests`, `structure`, `complexity`.
3. **Scope**: Exactly one of:
   - `XS` — under 30 minutes, 1–2 files
   - `S` — under 1 hour, 3–5 files
   - `M` — 1–3 hours, 5–15 files
   - `L` — half day or more, 15+ files or architectural changes
4. **Location**: Use concrete file paths relative to the repo root. Glob patterns are fine for groups. Never write "various files" or "multiple places."
5. **Current state**: Describe what actually exists. Not "this is bad" — describe the shape of the problem so someone seeing it for the first time understands.
6. **Desired state**: Describe the target. Reference the specific convention if one applies.
7. **Summary table**: When adding an item, append a row to the table at the top. When removing an item, delete its row.

### Example

Bad:
```markdown
### TD-050: Fix the hooks `inconsistency` `M`
**Location:** various files
**Current state:** Some hooks are in the wrong place.
**Desired state:** Move them.
```

Good:
```markdown
### TD-050: Config-type hooks not colocated with components `inconsistency` `S`
**Location:** `src/features/config-types/hooks/`
**Current state:** Contains `useCreateConfigTypeForm.ts`, `useConfigTypeForm.ts`, `useDeleteConfigTypeAlert.ts`, `useEditConfigTypeForm.ts`. These hooks are used only by specific components but live at the feature level.
**Desired state:** Each hook moves into the component directory that uses it. The `hooks/` directory is removed. Follows the component-level colocation convention in ARCHITECTURE.md.
```

## Cross-Repo Debt

If a debt item spans multiple repos (e.g., an API contract mismatch between `backend` and `openapi`), create an item in the **primary** repo (the one that needs the most change) and add a note referencing the other repo. Do not duplicate the item across repos.

## Housekeeping

- If you notice during normal work that a documented item has already been resolved, remove it.
- If you're fixing something and discover related debt, finish your current task first, then add new items.
- Don't renumber existing items. Gaps in IDs are fine and expected.
- Items are ordered by ID (insertion order). Don't reorder.
- Keep the summary table in sync with the items below it.
