---
name: design
description: Design and implement code changes end-to-end, from requirements clarification through implementation, validation, and documentation updates. Use when writing new code, extending features, fixing bugs, or making structured code changes where correctness, maintainability, and clear tradeoffs matter.
---

# Design Workflow

## Inputs
- `goal`: behavior to implement or fix.
- `scope` (optional): files, module, package, or repo.
- `constraints` (optional): compatibility, performance, security, reliability, timeline.

## References
Use selective loading:
- Always start with `references/philosophy/overview.md`.
- Load topical philosophy files only when applicable:
- `references/philosophy/functions.md` for function boundaries/responsibility concerns.
- `references/philosophy/names.md` for naming decisions or rename-heavy work.
- `references/philosophy/never-nest.md` for control-flow simplification.
- `references/philosophy/parse-not-validate.md` for boundary/parsing/invariant work.
- Load only active-language checklists:
- `references/checklists/go.md`
- `references/checklists/javascript-typescript.md`
- `references/checklists/python.md`
- `references/checklists/rust.md`
- In polyglot changes, load only the checklists for touched languages.

## Procedure
1. Clarify target behavior and constraints.
2. Inspect current code paths and affected boundaries.
3. Propose a minimal design that satisfies behavior and constraints.
4. Implement incrementally with clear, cohesive changes.
5. Validate behavior with tests and runtime checks.
6. Run lint/format tooling for touched scope.
7. Update docs/comments when behavior or usage changes.
8. Summarize outcomes, tradeoffs, and residual risk.

## Design Principles
- Prefer simple, explicit control flow over cleverness.
- Keep functions/modules single-purpose and composable.
- Keep invariants at boundaries (parse/normalize early).
- Make side effects explicit and scoped.
- Preserve compatibility unless behavior changes are explicitly requested.

## Language-Specific Implementation Checks
- `go`: clear error wrapping/propagation, context discipline, small interfaces, shallow nesting.
- `javascript`/`typescript`: strict typing over `any`, explicit async error handling, clear module boundaries.
- `python`: explicit exceptions, immutable defaults, typed interfaces where helpful, cohesive modules.
- `rust`: explicit ownership/error flows, avoid unnecessary clones, prefer `Result`/`Option` over panics.

## Output Contract
1. Design summary (approach and constraints).
2. Code changes made (files and rationale).
3. Validation evidence (tests/lint/runtime checks).
4. Remaining risks or follow-ups.
