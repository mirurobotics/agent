# Build a Custom Rust Import Linter

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | New `tools/lint/` workspace, integration into `scripts/lint.sh`, fix existing violations |

This plan lives in `agent/.ai/exec-plans/` because all code changes are within the agent repository.

## Purpose / Big Picture

After this work, running `cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/src` from the agent repo root will check every `.rs` file and report violations of the import convention: wrong group order, missing comment headers, unsorted imports within a group, or misclassified imports. Running with `--fix` will auto-fix all violations in place. The linter will be integrated into `scripts/lint.sh` so it runs alongside `cargo fmt` and `clippy` as part of the standard lint workflow.

A developer can verify it works by intentionally misordering imports in any file and running the linter — it reports the file, line, and violation. Running with `--fix` corrects the file. Running again reports zero violations.

## Progress

- [x] Milestone 1: Scaffold `tools/lint/` Rust workspace with binary crate
- [x] Milestone 2: Implement line-based import block parser
- [x] Milestone 3: Implement classification, ordering checks, and diagnostics
- [x] Milestone 4: Implement `--fix` mode (auto-rewrite)
- [x] Milestone 5: Add tests (unit + fixture-based) — 13 unit tests passing
- [x] Milestone 6: Integrate into `scripts/lint.sh` and fix existing violations — 71 files fixed, 0 violations remaining
- [x] Final: All 1004 agent tests pass, clippy clean, zero lint violations

## Surprises & Discoveries

- Observation: The initial parser accepted ALL comment lines as part of the import block, which caused it to consume section separator comments (e.g. `// ======= SECTION =======`) and subsequent `#[derive(...)]` attributes. The fixer then dropped these, stripping derive macros from structs/enums.
  Evidence: First fix attempt produced 324 compile errors — `DplTarget` and other enums lost their `Clone`, `Copy`, `Debug`, `Default`, `Serialize` derives.
  Fix: Parser now only accepts recognized group header comments (`// standard`, `// internal`, `// external`) and only accepts `#[attr]` lines if a lookahead confirms they precede a `use` statement.

- Observation: Pre-existing issue — `agent/src/cli.rs` and `agent/src/cli/mod.rs` both existed (from TD-002 restructure not being fully committed). This caused E0761 (duplicate module file). Resolved by `git rm agent/src/cli.rs`.

## Decision Log

- Decision: Place the tool at `agent/tools/lint/` as a standalone workspace separate from the agent Cargo workspace.
  Rationale: Keeping it outside the agent Cargo workspace avoids polluting agent's dependency tree and compile graph. It has its own `Cargo.toml` workspace root and compiles independently.
  Date/Author: 2026-03-04

- Decision: Use line-based parsing, not `syn`.
  Rationale: Import lines are syntactically predictable. `syn` is heavy (~10s compile time) and overkill for matching `use` statements. Line-based parsing handles the actual patterns in this codebase (including multi-line `use` with braces) with minimal code.
  Date/Author: 2026-03-04

- Decision: Classify `backend_api` and `device_api` as internal crates.
  Rationale: They are workspace members defined in `agent/Cargo.toml`. Most files already place them under `// internal crates`. This was decided in the earlier TD-001 work.
  Date/Author: 2026-03-04

- Decision: Use a config file (`.lint-imports.toml`) for internal crate names rather than hardcoding.
  Rationale: Different workspaces have different internal crates. A config file at the workspace root makes the tool reusable. The config also specifies comment labels, so the convention can evolve without changing the tool.
  Date/Author: 2026-03-04

- Decision: Comment headers are required only for groups that have imports. A file with only internal imports gets just `// internal crates`, not all three headers.
  Rationale: Matches existing codebase style — small files with one group don't have empty-group headers. Forcing empty headers would add noise to files like `storage/git_commits.rs` (2 lines of imports).
  Date/Author: 2026-03-04

## Outcomes & Retrospective

**Achieved:**
- Standalone Rust linter at `tools/lint/` (compiles in ~6s from clean, runs in <1s against 103 files)
- Detects: wrong group order, missing headers, wrong headers, unsorted imports, misplaced imports
- `--fix` mode auto-corrects all violations idempotently
- 13 unit tests covering parser, classifier, and edge cases
- Integrated into `scripts/lint.sh` after `cargo fmt` step
- Fixed 71 source files — zero violations remaining
- All 1004 agent tests pass, clippy clean

**Lesson:** Line-based parsers for import blocks must be conservative about what constitutes "part of the import block." Separator comments and non-use attributes should terminate the block. The initial permissive approach caused data loss (stripped derive macros). Adding lookahead for attributes and restricting comment acceptance to known group headers fixed this completely.

## Context and Orientation

### The import convention

The agent codebase (`agent/src/`) follows a three-group import convention documented in `AGENTS.md`:

    // standard crates
    use std::sync::Arc;

    // internal crates
    use crate::app::state::AppState;

    // external crates
    use tokio::sync::broadcast;

**Groups must appear in this order:** standard → internal → external. Each present group is preceded by a comment header and separated from the next group by a blank line.

**Classification rules:**
- **Standard:** `std`, `core`, `alloc`
- **Internal:** `crate`, `super`, `self`, plus workspace-sibling crates (`backend_api`, `device_api`, `miru_agent`)
- **External:** everything else (e.g. `tokio`, `serde`, `axum`, `tracing`)

Within each group, `use` statements are sorted alphabetically by their full path text.

### Current state of the codebase

After the TD-001 cleanup, ~88 of ~103 source files have correct comment headers. Known remaining issues:
- `server/handlers.rs` — `use std::sync::Arc;` on line 1 has no `// standard crates` header; `use std::future::Future;` on line 23 is misplaced in the external group.
- `sync/deployments.rs` — internal imports (`super::`, `crate::`, `backend_api::`) have no `// internal crates` header.
- ~15 small files have imports without any headers (single-group files).

### Import block structure

The "import block" is the contiguous region at the top of a file containing only:
- Comment lines (`//` comments — specifically group headers and blank `//` lines)
- `use` statements (single-line or multi-line with `{...}`)
- Blank lines (separating groups)
- Attribute lines (`#[...]`) directly preceding a `use` statement (e.g. `#[allow(unused_imports)]`)

The import block ends at the first line that is none of the above (e.g. a `fn`, `struct`, `pub mod`, `const`, or `#[cfg(test)]`). `use` statements inside `mod tests {}` blocks are not part of the import block and must be ignored.

### Multi-line use statements

53 files contain multi-line `use` statements like:

    use crate::mqtt::{
        client::{poll, ClientI},
        device::{Ping, SyncDevice},
        errors::*,
    };

The parser must track brace depth to correctly identify where a multi-line `use` statement ends. The sort key for a multi-line `use` is the path prefix on the first line (e.g. `crate::mqtt`).

### Project layout

All paths below are relative to the agent repo root (`agent/`).

    tools/
      lint/
        Cargo.toml          # [package] name = "lint-imports"
        src/
          main.rs           # CLI entry point (clap)
          parser.rs         # Import block parser
          classifier.rs     # Crate classification (std/internal/external)
          checker.rs        # Ordering and header checks, diagnostics
          fixer.rs          # Auto-fix: rewrite import block
          config.rs         # Read .lint-imports.toml
        tests/
          fixtures/         # .rs fixture files for testing
            correct.rs
            wrong_order.rs
            missing_headers.rs
            mixed_groups.rs
            multiline.rs
            single_group.rs
            no_imports.rs
          integration.rs    # Fixture-based tests

### Config file format

A `.lint-imports.toml` file at the scanned directory root (or the closest ancestor that has one):

    # Crate names that belong in the "internal" group
    # (in addition to crate/super/self which are always internal)
    internal_crates = ["backend_api", "device_api", "miru_agent"]

    # Comment labels for each group
    [labels]
    standard = "// standard crates"
    internal = "// internal crates"
    external = "// external crates"

If no config file is found, the tool uses sensible defaults: no extra internal crates, and the labels shown above.

## Plan of Work

### Milestone 1: Scaffold the workspace

Create `tools/lint/` as a standalone Rust workspace with a binary crate called `lint-imports`.

Files to create:
- `tools/lint/Cargo.toml` — workspace and package definition. Dependencies: `clap` (CLI arg parsing), `toml` (config parsing), `walkdir` (recursive file discovery).
- `tools/lint/src/main.rs` — CLI skeleton using `clap` derive API. Accepts `--path <dir>` (default `.`), `--fix` flag, `--config <path>` optional override.
- Empty module files: `parser.rs`, `classifier.rs`, `checker.rs`, `fixer.rs`, `config.rs`.

Verify: `cargo build` from `tools/lint/` compiles and `cargo run -- --help` prints usage.

### Milestone 2: Import block parser

Implement `parser.rs` — given the text content of a `.rs` file, extract the import block as a structured representation.

Data model:

    pub enum ImportGroup {
        Standard,
        Internal,
        External,
    }

    pub struct UseStatement {
        /// The full text of the use statement (may be multiple lines)
        pub text: String,
        /// Line number (1-based) where this use statement starts
        pub line: usize,
        /// The crate/path prefix used for classification (e.g. "std", "crate", "tokio")
        pub root_crate: String,
        /// The full path used for sorting (e.g. "crate::app::state::AppState")
        pub sort_key: String,
    }

    pub struct ImportBlock {
        /// Ordered list of items in the import block (comments, uses, blanks)
        pub items: Vec<ImportBlockItem>,
        /// Line number where the import block ends
        pub end_line: usize,
    }

    pub enum ImportBlockItem {
        Comment { text: String, line: usize },
        Use(UseStatement),
        BlankLine { line: usize },
    }

Parsing logic:
1. Iterate lines from the top of the file.
2. Track brace depth for multi-line `use` statements.
3. For each `use` statement, extract the `root_crate` (first path segment after `use`) and `sort_key` (the full path up to `::` or `{`).
4. Stop at the first line that is not a comment, `use`, blank, or attribute.
5. Handle `pub use` and `use` identically for parsing; both are import statements.

### Milestone 3: Classification, ordering checks, and diagnostics

Implement `classifier.rs` — classify each `UseStatement` into `Standard`, `Internal`, or `External` based on its `root_crate` field and the config.

Implement `checker.rs` — given a parsed `ImportBlock` and classification, produce a list of diagnostics:

    pub struct Diagnostic {
        pub file: PathBuf,
        pub line: usize,
        pub kind: DiagnosticKind,
        pub message: String,
    }

    pub enum DiagnosticKind {
        MissingHeader,       // group present but no comment header
        WrongHeader,         // comment text doesn't match expected label
        WrongGroupOrder,     // e.g. external before internal
        MisplacedImport,     // import is in the wrong group
        UnsortedImport,      // import is not alphabetically ordered within group
        MissingBlankLine,    // no blank line between groups
        ExtraBlankLine,      // blank line within a group
    }

Checks to perform:
1. Classify every `UseStatement`.
2. Verify group ordering: all standard before all internal before all external.
3. For each group, verify the comment header is present and correct.
4. Within each group, verify alphabetical ordering by `sort_key`.
5. Verify blank lines separate groups (exactly one blank line between groups, no blank lines within a group).

Wire into `main.rs`: walk the `--path` directory for `*.rs` files, parse each, check each, print diagnostics. Exit code 0 if no violations, 1 if any.

### Milestone 4: `--fix` mode

Implement `fixer.rs` — given a parsed `ImportBlock` with classifications, produce the corrected import block text.

Fix logic:
1. Collect all `UseStatement`s, classify each.
2. Group them into three buckets (standard, internal, external).
3. Sort each bucket alphabetically by `sort_key`.
4. Emit the fixed import block: for each non-empty bucket, emit the comment header, then the sorted `use` statements, then a blank line.
5. Replace the original import block (lines 1 through `end_line`) with the fixed text, preserving everything after the import block unchanged.

Write the fixed content back to the file only when `--fix` is passed. Report which files were fixed.

### Milestone 5: Tests

**Unit tests** (inline `#[cfg(test)]` modules):
- `parser.rs` — parse single-line use, multi-line use, mixed comments/blanks, empty file, file with no imports.
- `classifier.rs` — classify std, crate, super, self, external, configured internal crates.
- `checker.rs` — detect each diagnostic kind.

**Fixture-based integration tests** (`tests/integration.rs`):
- Create `.rs` fixture files under `tests/fixtures/` representing each violation pattern and the expected correct output.
- For each fixture: parse → check → assert expected diagnostics. Then: fix → re-check → assert zero diagnostics.
- Fixtures needed:
  - `correct.rs` — already correct, zero diagnostics.
  - `wrong_order.rs` — external before internal.
  - `missing_headers.rs` — imports present but no comment headers.
  - `mixed_groups.rs` — std import in external group (like `handlers.rs`).
  - `multiline.rs` — multi-line use statements in wrong order.
  - `single_group.rs` — file with only internal imports, verify header is added.
  - `no_imports.rs` — file with no use statements, verify no crash and zero diagnostics.

### Milestone 6: Integration

1. Create a `.lint-imports.toml` in the agent repo root:

       internal_crates = ["backend_api", "device_api", "miru_agent"]

2. Run the linter against `agent/src/` in check mode to see all current violations.

3. Run with `--fix` to auto-fix all violations.

4. Add a step to `scripts/lint.sh` that runs the linter in check mode. Insert it after the `cargo fmt` step:

       echo "Checking import formatting"
       echo "-------------------------"
       cargo run --manifest-path "$git_repo_root_dir/tools/lint/Cargo.toml" -- --path agent/src --config "$git_repo_root_dir/.lint-imports.toml"
       echo ""

5. Run `scripts/test.sh` to verify no behavioral regressions from the auto-fix.

6. Run `scripts/lint.sh` end-to-end to verify the linter passes as part of the full lint pipeline.

## Concrete Steps

All commands are run from the agent repo root (`agent/`) unless otherwise noted.

### Milestone 1

    mkdir -p tools/lint/src tools/lint/tests/fixtures

    # Create tools/lint/Cargo.toml — see Plan of Work for contents
    # Create tools/lint/src/main.rs — clap CLI skeleton
    # Create empty module files: parser.rs, classifier.rs, checker.rs, fixer.rs, config.rs

    # Verify
    From agent/tools/lint/: cargo build
    From agent/tools/lint/: cargo run -- --help

Expected: prints usage with `--path`, `--fix`, `--config` options.

### Milestone 2

    # Implement tools/lint/src/parser.rs
    # Wire parser into main.rs for smoke testing

    # Verify
    From agent/tools/lint/: cargo run -- --path ../../agent/src

Expected: parses all files without panicking (may print debug info, no diagnostics yet).

### Milestone 3

    # Implement tools/lint/src/classifier.rs
    # Implement tools/lint/src/checker.rs
    # Implement tools/lint/src/config.rs
    # Wire checker output into main.rs

    # Verify
    From agent/tools/lint/: cargo run -- --path ../../agent/src --config ../../.lint-imports.toml

Expected: reports violations in `server/handlers.rs` (misplaced std import, missing header), `sync/deployments.rs` (missing internal header), and other files with missing or incorrect headers. Exit code 1.

### Milestone 4

    # Implement tools/lint/src/fixer.rs
    # Wire --fix flag into main.rs

    # Verify
    # First, copy a known-bad file to a temp location and test fix:
    cp agent/src/server/handlers.rs /tmp/test_handlers.rs
    From agent/tools/lint/: cargo run -- --path /tmp --fix --config ../../.lint-imports.toml
    cat /tmp/test_handlers.rs  # verify corrected import block

Expected: `std::sync::Arc` and `std::future::Future` are both in the standard group with `// standard crates` header. Groups are ordered and sorted.

### Milestone 5

    # Create fixture files in tools/lint/tests/fixtures/
    # Create tools/lint/tests/integration.rs

    # Verify
    From agent/tools/lint/: cargo test

Expected: all unit and integration tests pass.

### Milestone 6

    # Create .lint-imports.toml at agent repo root
    # Run linter in fix mode against agent source
    From agent/tools/lint/: cargo run -- --path ../../agent/src --fix --config ../../.lint-imports.toml

    # Verify fixes are correct
    From agent/: ./scripts/test.sh
    From agent/: cargo fmt -p miru-agent -- --check

    # Add linter step to scripts/lint.sh
    # Run full lint pipeline
    From agent/: ./scripts/lint.sh

Expected: all tests pass, fmt clean, full lint pipeline passes including the new import check step.

## Validation and Acceptance

1. From `tools/lint/`: `cargo test` — all linter tests pass.
2. From `tools/lint/`: `cargo run -- --path ../../agent/src --config ../../.lint-imports.toml` — exit code 0, zero violations.
3. From `agent/`: `./scripts/test.sh` — all agent tests pass (no behavioral regressions from auto-fix).
4. From `agent/`: `./scripts/lint.sh` — full lint pipeline passes, including the new import lint step.
5. Intentionally misorder imports in any agent source file → linter reports violations → `--fix` corrects them → linter reports zero violations.

## Idempotence and Recovery

- **Milestones 1-5** (tool development) are fully idempotent — creating and editing files in `tools/lint/` has no effect on the agent codebase.
- **Milestone 6** (auto-fix) modifies agent source files. If the fix produces incorrect output:
  1. Revert with `git checkout -- agent/src/` from the agent repo root.
  2. Investigate the fixture that doesn't match the real file's pattern.
  3. Fix the parser/fixer and re-run.
- The `--fix` mode is idempotent: running it twice produces the same output. Running the checker after a fix should always report zero violations.
- The linter never modifies code outside the import block (everything after the last `use` statement is preserved byte-for-byte).
