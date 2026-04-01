---
name: rename
description: Safely rename symbols and identifiers with scope control, ambiguity checks, and post-rename verification. Use when renaming variables, functions, types, files, or API fields across one or many files, especially when consistency and low false-positive risk matter.
---

# Rename Workflow

## Inputs
- `target`: exact symbol or identifier to rename.
- `replacement`: new name.
- `scope` (optional): file, directory, package, module, or repo boundaries.

## References
For complex tasks, invoke the `$design` and `$lint` skills for naming and intent consistency standards before applying large renames.

## Method Selection
Use programmatic rename when all are true:
- At least 3 occurrences or multiple files.
- Pattern is unambiguous.
- Scope can be bounded safely.

Use manual rename when any are true:
- Only 1-2 occurrences.
- Pattern is ambiguous.
- Context-dependent edits are required.

## Procedure
1. Confirm target, replacement, and scope exactly.
2. Discover occurrences and classify by confidence (safe vs ambiguous).
3. Check for uncommitted changes and warn when rename will mix with unrelated edits.
4. Apply rename with the chosen method.
5. Verify no stale references remain.
6. Run relevant tests/lint checks when available and proportionate.
7. Report affected files and any residual risk.

## Language-Specific Conventions
- `go`: exported identifiers are `PascalCase`; unexported are `camelCase`; acronyms remain uppercase (`ID`, `URL`, `HTTP`).
- `javascript`/`typescript`: variables/functions `camelCase`, classes/components `PascalCase`, constants `SCREAMING_SNAKE_CASE`.
- `python`: functions/variables/modules `snake_case`, classes `PascalCase`, constants `SCREAMING_SNAKE_CASE`.
- `rust`: functions/modules `snake_case`, types/traits `PascalCase`, constants `SCREAMING_SNAKE_CASE`.

## Safety Rules
- Avoid partial rename completion.
- Avoid replacing unrelated identifiers.
- Prefer symbol-aware tooling when available.
- Validate casing and naming conventions for the language.

## Output Contract
1. Rename plan (method, scope, risk notes).
2. Rename execution summary (files, replacement count).
3. Verification summary (search/tests/lint outcomes).
