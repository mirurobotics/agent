---
name: review
description: Perform structured code review focused on correctness, test coverage, and regressions, with findings prioritized by severity and grounded in file/line evidence. Use when asked to review, audit, assess risk, or evaluate change quality before merge or commit.
---

# Review Workflow

## Core Principle

Every finding must be **actionable** — it must require a code change or a deliberate decision from the author. If the fix direction is "leave as-is", "no change needed", or "worth confirming", it is not a finding. Drop it.

**Excluded from review findings:**
- Style, formatting, and naming — that's the linter's job.
- "Verify this compiles" — if it compiles, it's fine. Check it yourself.
- Observations about things that are "probably fine" — if you think it's fine, don't mention it.
- Things that look intentional — skip them.

## Inputs
- `scope` (optional): files, package, module, branch, or full repo.
- `mode` (optional): `single-pass` (default) or `iterative`.
- `write_review_file` (optional): default `false`; persist artifacts only if explicitly requested.

## References
Use these skills as authoritative review lenses:
- `$design` for design/correctness quality checks.
- `$reliability` for error handling/lifecycle/failure-mode review.
- `$security` for security-sensitive change review.
- `$performance` for performance hotspot and regression review.
- `$test` for test quality and coverage checks.

## Scope and Order
1. Correctness — logic bugs, broken invariants, state corruption, data loss.
2. Test coverage — every changed behavior path has a test that would fail if the change regressed.
3. Security/reliability/performance where applicable.

## Procedure

### 1. Resolve scope
Default scope is **uncommitted changes** (staged + unstaged working tree diff). Only review previous commits if the user explicitly asks for it (e.g., "review the last 3 commits", "review branch X").

### 2. Correctness analysis
For each changed code path, verify:
- **Logic**: Does the code do what the author intended? Trace inputs through branches, loops, and early returns to confirm the output is correct for all reachable states.
- **Invariants**: Are pre/post-conditions and type contracts preserved? If a function's callers depend on a guarantee (non-null return, sorted output, unique keys), confirm the change still upholds it.
- **State transitions**: For stateful code, confirm every transition is reachable, no state is skipped, and no impossible state is introduced.
- **Boundary values**: Check off-by-one, empty collections, zero/negative/max values, and nil/null/None at every boundary the change touches.
- **Error paths**: Confirm errors are propagated correctly — not swallowed, not double-wrapped, not silently converted to a default value that hides the failure.

### 3. Test coverage analysis
For each changed behavior, locate the corresponding test. If no test exists, that is a finding. Specifically check:
- **Happy path**: Is the primary success case tested with a meaningful assertion on the output or side effect?
- **Error/failure path**: If the change adds or modifies error handling, is there a test that triggers the error and asserts the correct behavior (error type, message, recovery)?
- **Boundary/edge cases**: Are boundary values from step 2 covered by tests? If a function now handles empty input differently, there must be a test for empty input.
- **Regression protection**: If this change fixes a bug, is there a test that reproduces the original bug and would fail if the fix were reverted?
- **Assertion quality**: Tests that exist but assert on implementation details (mock call counts, internal state) instead of observable behavior are weak — flag them if they would not catch a real regression.

### 4. Sensitive path review
For security, reliability, and performance-sensitive changes, apply the corresponding skill lenses (`$security`, `$reliability`, `$performance`).

### 5. Self-resolve open questions
Before raising an open question, search the codebase to answer it yourself. Only raise questions you genuinely cannot resolve from the diff and surrounding code.

### 6. Produce findings
Sort by severity with file/line references and a concrete fix direction.

## Severity Definitions
- **high**: Will cause a bug, data loss, security issue, or regression in production. Also: changed behavior with **no test coverage at all**.
- **medium**: Likely to cause problems under realistic conditions. Also: existing tests that are **too weak to catch a regression** in the changed behavior (e.g., missing error-path test, assertion on implementation detail instead of behavior).

Style, formatting, and naming concerns are never findings at any severity. If something won't break anything and isn't a meaningful design concern, leave it out.

## Findings Format
For each finding include:
- Severity: `high` or `medium`
- Category: `correctness`, `test-coverage`, `security`, `reliability`, or `performance`
- What will go wrong (not what might go wrong)
- Evidence path with line reference
- Concrete fix direction (a specific change, not "consider" or "worth looking into")

If no findings exist, state that explicitly.

## Artifact Rule
- Default to chat output only.
- Do not write `REVIEWS.md` or other files unless explicitly requested.

## Output Contract
1. **Findings** (highest severity first). Fewer findings, more decisive.
2. **Test coverage summary** — for each changed behavior, state whether it is tested, untested, or weakly tested. Keep this concise: a table or short list, not a paragraph per item.
3. **Open questions** only where the reviewer cannot determine correctness from the diff and codebase.

No hedging language. No "worth confirming", "presumably", "likely intentional". Be direct.
