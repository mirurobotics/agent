---
name: lint
description: Run repository linting and formatting checks, fix lint failures, and verify a clean result. Use when asked to lint code, resolve style/tooling violations, or make CI lint checks pass across changed files or whole repositories.
---

# Lint Workflow

## Inputs
- `scope` (optional): target repo, package, directory, or file set.
- `fix_mode` (optional): `auto` (default) or `report-only`.
- `strict` (optional): fail fast on first blocking lint error when `true`.

## References
Use selective loading:
- Always start with `references/philosophy/overview.md`.
- Load `references/philosophy/linting.md` when lint strategy/tooling decisions are needed.
- Load only active-language checklists:
- `references/checklists/go.md`
- `references/checklists/javascript-typescript.md`
- `references/checklists/python.md`
- `references/checklists/rust.md`
- In polyglot changes, load only the checklists for touched languages.

## Procedure
1. Resolve lint command(s).
2. Run lint for the target scope and capture failures.
3. Apply safe fixes.
4. Re-run lint until clean or blocked.
5. Report what changed, what remains, and any manual follow-ups.

## Language-Specific Command Defaults
- `go`:
- Format/imports: `goimports -w` or `gofmt -w`.
- Lint/static analysis: repo script, `golangci-lint run`, or `go vet ./...`.
- `javascript`/`typescript`:
- Format: `prettier --write` (or repo equivalent).
- Lint/typecheck: `eslint`, plus `tsc --noEmit` for TypeScript.
- `python`:
- Format/imports: `black`, `isort`.
- Lint/static analysis: `ruff` (or `flake8`/`pylint` if repo-standard).
- `rust`:
- Format: `cargo fmt`.
- Lint/static analysis: `cargo clippy --all-targets --all-features`.

## Rules
- Use existing project lint configuration; do not invent new lint policy unless requested.
- Keep fixes minimal and scoped to lint issues.
- Do not suppress lint errors without explicit approval.
- Preserve repository conventions and file ownership boundaries.

## Output Contract
1. Lint command(s) executed and scope.
2. Summary of fixes applied.
3. Final lint status (clean or blocked) with remaining issues.
