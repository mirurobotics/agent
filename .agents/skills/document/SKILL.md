---
name: document
description: Add or improve code and repository documentation so behavior, constraints, and usage are clear and current. Use when asked to write/update docs, docstrings, inline rationale comments, architecture notes, or change-related documentation for code updates.
---

# Documentation Workflow

## Inputs
- `scope` (optional): symbols, files, module, or repo-level docs.
- `doc_level` (optional): `minimal` (default), `standard`, or `comprehensive`.
- `audience` (optional): engineers, operators, API consumers, or mixed.

## References
Use selective loading:
- Always start with `references/philosophy/overview.md`.
- Load topical philosophy files only when applicable:
- `references/philosophy/docstrings.md` when API docs/docstrings are in scope.
- `references/philosophy/comments.md` when inline rationale comments are needed.
- `references/philosophy/architecture.md` when updating architecture/codemap docs.
- `references/philosophy/markdown-quality.md` when editing markdown/MDX-heavy docs.
- Load only active-language checklists:
- `references/checklists/go.md`
- `references/checklists/javascript-typescript.md`
- `references/checklists/python.md`
- `references/checklists/rust.md`
- In polyglot changes, load only the checklists for touched languages.

## Procedure
1. Identify documentation targets from changed behavior and user request.
2. Determine doc type needed.
3. Draft updates that explain what changed, why, usage, and constraints.
4. Validate consistency, links, and examples.
5. Summarize updates and remaining documentation gaps.

## Language-Specific Documentation Standards
- `go`:
- Add/maintain `doc.go` for package-level docs when appropriate.
- Exported items should have Go-style doc comments beginning with item name.
- Keep inline comments focused on non-obvious rationale.
- `javascript`/`typescript`:
- Use JSDoc (`/** ... */`) for public functions/classes/components when behavior is non-obvious.
- Use `@param`, `@returns`, and `@throws` where useful; avoid duplicating type annotations.
- Keep inline comments to explain "why", not "what".
- `python`:
- Use module docstrings for public modules.
- Use consistent docstring style (Google/NumPy/Sphinx) within a repo.
- Document args/returns/raises when behavior is non-trivial.
- `rust`:
- Use `///` for public item docs and `//!` for module docs.
- Add `# Examples`, `# Errors`, `# Panics`, and `# Safety` sections when applicable.
- Keep docs runnable where practical via doc tests.

## Rules
- Prefer clarity and brevity over exhaustive prose.
- Avoid repeating obvious code in comments.
- Keep docs synchronized with current behavior.
- Add inline comments only for non-obvious "why" context.

## Output Contract
1. Documentation plan (targets and doc types).
2. Files/sections updated.
3. Validation notes and residual doc gaps.
