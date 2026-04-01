# Import Linter: Redesign, Lint, and Test

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | `tools/lint/` linter source code |

This plan lives in `agent/.ai/exec-plans/` because all code changes are within the agent repository.

## Purpose / Big Picture

The import linter at `tools/lint/` was built in a single session and works end-to-end — it correctly fixes all 103 agent source files. However, a code review identified correctness bugs in the checker and fixer, zero test coverage for 3 of 5 modules (checker, fixer, config), and the linter's own code has never been linted.

After this work:
- All known correctness bugs are fixed (checker header detection, fixer blank-line handling).
- The linter passes its own lint pipeline (`cargo fmt`, `cargo clippy`).
- Every module has unit tests covering happy paths, error paths, and edge cases.
- A developer can run `cargo test` from `tools/lint/` and see comprehensive test coverage.

## Progress

- [x] (2026-03-04) Milestone 1: Redesign — fix correctness bugs found in review
- [x] (2026-03-04) Milestone 2: Lint — run fmt + clippy on the linter and fix all issues (6 clippy errors fixed)
- [x] (2026-03-04) Milestone 3: Test — add comprehensive tests for checker, fixer, and config (22 new tests, 35 total)

## Surprises & Discoveries

- Observation: clippy found 6 issues in linter code — dead_code on enum fields, derivable Default impl, drain().collect() → mem::take(), manual strip_prefix.
  Evidence: `cargo clippy --all-targets -- -D warnings` produced 6 errors on first run.

## Decision Log

- Decision: Used `#[allow(dead_code)]` on `Comment.line` and `BlankLine.line` fields rather than removing them.
  Rationale: These fields are structurally important for the data model (they track where items appear in the file) and are used in tests. They will likely be needed for future diagnostic improvements. Removing them would lose information the parser already computes.
  Date/Author: 2026-03-04

- Decision: Config tests use `tempfile` crate for filesystem isolation.
  Rationale: Config's `find_from` and `from_file` methods interact with the real filesystem. Using tempfile ensures tests are isolated and don't depend on the working directory or leave artifacts.
  Date/Author: 2026-03-04

## Outcomes & Retrospective

**Achieved:**
- Fixed 3 correctness bugs: checker header detection skipping blank lines, fixer double blank line, parser inconsistent line numbering
- Linter passes `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` cleanly
- 35 tests total (13 existing + 22 new): checker (7), fixer (9), config (6), parser (8+1 existing = 9), classifier (4)
- Zero violations on agent source, fix mode is idempotent (0 files changed)

**Test coverage by module:**
| Module | Tests | Coverage |
|--------|-------|----------|
| parser | 9 | Happy paths, edge cases, boundary conditions |
| classifier | 4 | All classification categories |
| checker | 7 | Correct input, wrong order, missing/wrong headers, blank-line-separated headers, single group, no imports |
| fixer | 9 | Idempotent, adds headers, regroups, preserves rest, trailing newline, no double blank, attributes, empty file, single group |
| config | 6 | Defaults, parse TOML, missing file, malformed file, walk-up discovery, no config fallback |

**Lesson:** Running the linter's own lint pipeline early would have caught the 6 clippy issues during initial development. For new tools, always include self-linting as part of the first implementation pass.

## Context and Orientation

### The linter tool

The import linter lives at `agent/tools/lint/` as a standalone Rust workspace (separate `[workspace]` in its `Cargo.toml` to avoid being absorbed by the agent workspace). It has 5 modules:

- `parser.rs` — Parses the import block at the top of a `.rs` file into structured `ImportBlock` containing `ImportBlockItem`s (comments, use statements, blank lines). Has 8 unit tests.
- `classifier.rs` — Classifies each `UseStatement` into `Standard`, `Internal`, or `External` by checking `root_crate` against known lists and config. Has 4 unit tests.
- `checker.rs` — Produces diagnostics for wrong group order and missing/wrong headers. Has **zero tests**.
- `fixer.rs` — Rewrites the import block with correct grouping and headers (no sorting — that's delegated to `cargo fmt`). Has **zero tests**.
- `config.rs` — Reads `.lint-imports.toml`, walks up directory tree to find it. Has **zero tests**.
- `main.rs` — CLI entry point using clap. Walks directory, parses each `.rs` file, checks or fixes.

### Known bugs from code review

**Bug 1 — checker.rs:100-124: Header detection skips blank lines.**
The checker looks at `block.items[idx - 1]` for the header comment before a group's first use statement. If a blank line separates the header from the use statement, the checker misses it and reports a false "missing-header" diagnostic. The fix is to walk backward, skipping `BlankLine` items.

**Bug 2 — checker.rs:99-124: Header detection skips attribute lines.**
Similarly, if one or more `#[attr]` lines sit between the header comment and the use statement (the attrs are absorbed into `UseStatement.attrs`), `idx - 1` points to the item before the attrs — which may be a blank line or the previous group's use statement, not the header. The checker must account for the fact that attrs are stored inside the `UseStatement`, not as separate `ImportBlockItem`s.

Actually, looking more carefully: the parser stores attrs inside the `UseStatement.attrs` field and they are NOT separate `ImportBlockItem`s. So the item at `idx - 1` in `block.items` should be the comment or blank line directly before the use statement (with attrs already consumed). The blank-line skip is the real issue.

**Bug 3 — fixer.rs:84-86: Potential double blank line.**
The fixer unconditionally adds `\n` before `rest_lines` (creating a blank separator line). If `rest_lines` starts with an empty line, the output has two consecutive blank lines. The fix is to skip leading blank lines from `rest_lines` before adding the separator, or check if the first rest line is blank.

**Bug 4 — parser.rs:157-161: Trailing attrs get 0-based line numbers.**
When flushing pending attrs after the main loop, they get `line: i` (0-based) instead of `line: i + 1` (1-based). Low practical impact (this code path is rarely hit) but inconsistent.

### Import block structure reference

The parser recognizes this structure at the top of a file:
- Group header comments: `// standard crates`, `// internal crates`, `// external crates`
- `use` / `pub use` statements (single-line or multi-line with brace tracking)
- `#[attr]` lines preceding a use statement (stored in `UseStatement.attrs`)
- Blank lines separating groups

The block ends at the first line that doesn't match any of the above (e.g. `fn`, `struct`, `#[derive]` not followed by `use`).

## Plan of Work

### Milestone 1: Redesign — fix correctness bugs

#### Fix 1: Checker header detection (checker.rs)

In `check_headers()`, replace the single `idx - 1` lookup with a backward walk that skips `BlankLine` items:

In `checker.rs`, the current code at lines 100-124 does:
```rust
if idx > 0 {
    if let ImportBlockItem::Comment { text, .. } = &block.items[idx - 1] {
```

Replace with a helper that walks backward from `idx`, skipping blank lines:
```rust
// Walk backward from idx, skipping blank lines, to find the nearest comment
let mut check_idx = idx;
while check_idx > 0 {
    check_idx -= 1;
    match &block.items[check_idx] {
        ImportBlockItem::BlankLine { .. } => continue,
        ImportBlockItem::Comment { text, .. } => {
            let trimmed = text.trim();
            if trimmed == expected_label {
                found_header = true;
            } else if is_group_header(trimmed) {
                diagnostics.push(Diagnostic { ... });
                found_header = true;
            }
            break;
        }
        _ => break, // hit a use statement from another group
    }
}
```

#### Fix 2: Fixer blank line handling (fixer.rs)

In `fix_file()`, before the unconditional `output.push('\n')` at line 84, check if `rest_lines` starts with a blank line. If it does, skip leading blank lines from rest to avoid doubling:

```rust
// Skip leading blank lines from rest_lines to avoid double blank line
let rest_start = rest_lines
    .iter()
    .position(|l| !l.trim().is_empty())
    .unwrap_or(rest_lines.len());
let rest_lines = &rest_lines[rest_start..];

if !rest_lines.is_empty() {
    output.push('\n');
}
```

#### Fix 3: Parser trailing attrs line numbers (parser.rs)

At line 160, change `line: i` to `line: i + 1` for consistency with all other line numbers in the parser.

#### Verify

After all fixes:
1. `cargo build` from `tools/lint/` compiles.
2. `cargo run -- --path ../../agent/src --config ../../.lint-imports.toml` — zero violations.
3. `cargo run -- --path ../../agent/src --fix --config ../../.lint-imports.toml` — zero files changed (idempotent).
4. From `agent/`: `./scripts/test.sh` — all agent tests pass (no regressions from linter changes).

### Milestone 2: Lint the linter

Run `cargo fmt` and `cargo clippy` on the linter's own code and fix all issues.

1. From `tools/lint/`: `cargo fmt`
2. From `tools/lint/`: `cargo clippy --all-targets -- -D warnings`
3. Fix any issues found.
4. Repeat until clean.

### Milestone 3: Test the linter

Add comprehensive unit tests to the three untested modules. Tests should be inline `#[cfg(test)]` modules following the pattern already used in `parser.rs` and `classifier.rs`.

#### checker.rs tests

Test `check()` with constructed `ImportBlock` + `Classifier` inputs:

1. **correct_ordering_no_diagnostics** — All three groups in order with correct headers. Assert empty diagnostics.
2. **wrong_group_order** — External use before internal use. Assert `wrong-group-order` diagnostic.
3. **missing_header** — Imports present but no comment header before a group. Assert `missing-header` diagnostic.
4. **wrong_header** — `// standard crates` label before external imports. Assert `wrong-header` diagnostic.
5. **header_with_blank_line_between** — Header comment, blank line, then use statement. Assert the header IS found (tests the bug fix from Milestone 1).
6. **single_group_needs_header** — File with only internal imports and no header. Assert `missing-header`.
7. **no_imports_no_diagnostics** — Empty import block. Assert empty diagnostics.

#### fixer.rs tests

Test `fix_file()` with constructed content + `ImportBlock` + `Classifier` inputs:

1. **already_correct_unchanged** — Content with correct grouping and headers. Assert output == input.
2. **adds_missing_headers** — Imports without headers. Assert headers are added.
3. **regroups_misplaced_imports** — External import mixed into internal group. Assert it's moved to external group.
4. **preserves_rest_of_file** — Import block followed by `fn main() {}`. Assert everything after imports is preserved byte-for-byte.
5. **preserves_trailing_newline** — File ending with `\n`. Assert output ends with `\n`.
6. **no_double_blank_line** — File where content after imports starts with a blank line. Assert no double blank line in output.
7. **handles_attributes_on_use** — Use statement with `#[allow(unused_imports)]`. Assert attribute is preserved in output.
8. **empty_file_unchanged** — Empty content. Assert output == input.
9. **single_group_file** — Only internal imports. Assert single header, no extra blank lines.

#### config.rs tests

1. **default_config_values** — `Config::default()` has empty `internal_crates` and standard labels.
2. **from_file_parses_toml** — Write a temp `.lint-imports.toml`, call `from_file`, assert parsed values.
3. **from_file_missing_returns_default** — Call `from_file` with nonexistent path, assert default.
4. **from_file_malformed_returns_default** — Write invalid TOML, call `from_file`, assert default.
5. **find_from_walks_up** — Create a temp directory tree, place config in parent, call `find_from` on child. Assert config is found.
6. **find_from_no_config_returns_default** — Call `find_from` on a temp dir with no config file. Assert default.

#### Verify

From `tools/lint/`: `cargo test` — all tests pass (existing 12 + new ~22 = ~34 total).

## Concrete Steps

All commands are run from `agent/` (submodule root) unless otherwise noted.

### Milestone 1

    # Edit tools/lint/src/checker.rs — fix header detection to skip blank lines
    # Edit tools/lint/src/fixer.rs — fix blank line handling
    # Edit tools/lint/src/parser.rs — fix trailing attr line numbers

    # Verify
    From agent/tools/lint/: cargo build
    From agent/tools/lint/: cargo run -- --path ../../agent/src --config ../../.lint-imports.toml
    From agent/tools/lint/: cargo run -- --path ../../agent/src --fix --config ../../.lint-imports.toml
    From agent/: ./scripts/test.sh

Expected: zero violations, zero files changed on fix, all agent tests pass.

### Milestone 2

    From agent/tools/lint/: cargo fmt
    From agent/tools/lint/: cargo clippy --all-targets -- -D warnings
    # Fix any issues, repeat until clean

Expected: fmt and clippy both pass cleanly.

### Milestone 3

    # Add #[cfg(test)] mod tests to checker.rs, fixer.rs, config.rs
    # Verify
    From agent/tools/lint/: cargo test

Expected: all ~34 tests pass.

## Validation and Acceptance

1. From `tools/lint/`: `cargo test` — all tests pass (parser, classifier, checker, fixer, config).
2. From `tools/lint/`: `cargo fmt -- --check` — clean.
3. From `tools/lint/`: `cargo clippy --all-targets -- -D warnings` — clean.
4. From `tools/lint/`: `cargo run -- --path ../../agent/src --config ../../.lint-imports.toml` — exit 0, zero violations.
5. From `agent/`: `./scripts/test.sh` — all agent tests pass.

## Idempotence and Recovery

All changes are within `tools/lint/src/`. If any milestone breaks the linter's behavior on agent source files:

1. Revert with `git checkout -- tools/lint/` from `agent/`.
2. Re-verify the linter still produces zero violations on agent source.
3. Retry the milestone with corrections.

The linter tool is independent from the agent binary — changes to `tools/lint/` cannot break agent compilation or tests.
