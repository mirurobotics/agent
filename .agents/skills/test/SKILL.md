---
name: test
description: Design and implement high-value tests for changed or specified behavior, including edge cases, error paths, and dependency interactions, with a plan-first workflow and explicit verification results. Use when asked to add tests, improve coverage, validate fixes, or harden behavior.
---

# Test Workflow

## Inputs
- `target`: symbol, file, module, or behavior to test.
- `test_type` (optional): `unit`, `integration`, or mixed.
- `plan_only` (optional): default `false`.

## References
Use selective loading:
- Always start with `references/philosophy/overview.md`.
- Load only relevant philosophy files:
- `references/philosophy/unit-tests.md` for unit-focused work.
- `references/philosophy/integration-tests.md` for integration scenarios.
- `references/philosophy/e2e-tests.md` only when e2e behavior is in scope.
- `references/philosophy/coverage.md` when coverage quality/thresholds are in question.
- Load only active-language checklists:
- `references/checklists/go.md`
- `references/checklists/javascript-typescript.md`
- `references/checklists/python.md`
- `references/checklists/rust.md`
- In polyglot changes, load only the checklists for touched languages.

## Procedure
1. Identify target behavior and owning files.
2. Locate existing tests and choose placement.
3. Analyze behavior: inputs, outputs, side effects, dependencies, boundary conditions.
4. Produce a concrete test plan before writing code.
5. After approval (or explicit request), implement tests.
6. Run relevant test commands and report exact outcomes.

## Language-Specific Testing
- `go`:
- Prefer table-driven tests.
- Use `Test<Function>` naming and `t.Run` subtests.
- Place unit tests with code (`*_test.go`), integration tests in `tests/` when repository follows that pattern.
- `javascript`/`typescript`:
- Use `vitest` or `jest` conventions (`*.test.ts`, `*.spec.ts[x]`).
- Use `describe`/`it` and `test.each` for parameterized cases.
- For React, test user-visible behavior via Testing Library.
- `python`:
- Use `pytest` with `test_*.py` naming.
- Prefer `pytest.mark.parametrize` for matrixed cases.
- Use fixtures for setup/teardown and `pytest.raises` for failures.
- `rust`:
- Use `#[cfg(test)]` unit modules and `tests/` integration tests.
- Use standard `assert!`/`assert_eq!` and explicit error-case assertions.
- Include doc tests for public API examples when meaningful.

## Required Plan Sections
- Happy path
- Invalid input or error path
- Boundary and edge cases
- Side-effect/dependency behavior

## Test Quality Rules
- Prefer deterministic tests.
- Keep tests independent and order-agnostic.
- Assert on behavior, not implementation trivia.
- Cover changed behavior before broad expansion.

## Output Contract
1. Test plan first.
2. Implementation summary second (when performed).
3. Verification results last (command and pass/fail details).
