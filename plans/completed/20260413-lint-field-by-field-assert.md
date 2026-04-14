# Lint Rule: Field-by-Field Assert Detection

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | New `tools/lint-asserts/` crate, integration into `scripts/lib/lint.sh` and CI |

## Purpose / Big Picture

After this work, running `cargo run --manifest-path tools/lint-asserts/Cargo.toml -- --path agent/tests` from the agent repo root will scan every `.rs` file containing test functions and flag any test that has 4 or more `assert_eq!` calls on fields of the same variable. The diagnostic message recommends constructing an expected struct and comparing the whole thing. This improves test readability and maintainability by replacing fragmented field-by-field assertions with single structural comparisons.

A developer can verify it works by writing a test with 4+ `assert_eq!` calls on fields of one variable, running the linter, and seeing the diagnostic. Adding `// lint:allow(field-by-field-assert)` anywhere in the test body suppresses the warning.

### Why a separate tool

The existing `tools/lint/` tool is `lint-imports` -- all 6 rules are import-related, the CLI is named `lint-imports`, and the code is tightly coupled to import block parsing. Adding an unrelated AST analysis rule would conflate concerns. A new `tools/lint-asserts/` tool is cleaner because:
- It is a separate concern from import linting.
- The lint pipeline in `scripts/lib/lint.sh` already runs multiple separate tools.
- It can be added as another step in the pipeline.
- It shares the `syn` dependency but nothing else.

## Progress

- [ ] Milestone 1: Scaffold `tools/lint-asserts/` crate
- [ ] Milestone 2: Implement AST walking and field-assert detection
- [ ] Milestone 3: Implement CLI, output formatting, and escape hatch
- [ ] Milestone 4: Add tests
- [ ] Milestone 5: Integrate into lint pipeline and CI

## Surprises & Discoveries

(none yet)

## Decision Log

- Decision: Create a standalone `tools/lint-asserts/` crate rather than extending `tools/lint/`.
  Rationale: The import linter's parser, classifier, checker, and fixer modules are all import-specific. Bolt-on rules would bloat the tool and confuse its CLI (`--fix` has no meaning for this new rule). A separate binary keeps each tool focused and independently testable.
  Date/Author: 2026-04-13

- Decision: Use `syn::visit::Visit` trait for AST walking rather than manual recursion.
  Rationale: The `Visit` trait provides a clean, exhaustive walk of the AST. We only need to override `visit_expr_macro` to collect `assert_eq!` calls and `visit_item_fn` to scope collection to test functions. This avoids hand-rolling recursive descent over every `Expr` variant.
  Date/Author: 2026-04-13

- Decision: Threshold is configurable via `--threshold` but defaults to 4.
  Rationale: 4 is the sweet spot -- 3 field asserts is common and often reasonable (e.g. id + type + status), while 4+ strongly suggests an expected struct would be clearer. The flag allows repos to tune this.
  Date/Author: 2026-04-13

## Outcomes & Retrospective

(to be filled on completion)

## Context and Orientation

### The pattern being detected

In test functions, it is common to assert individual fields of a response/result struct:

```rust
#[test]
fn test_get_deployment() {
    let req = client.last_request();
    assert_eq!(req.call, Call::GetDeployment);
    assert_eq!(req.method, reqwest::Method::GET);
    assert_eq!(req.path, "/deployments/dpl_1");
    assert_eq!(req.url, "http://mock/deployments/dpl_1");
}
```

This is harder to read and maintain than constructing an expected struct:

```rust
#[test]
fn test_get_deployment() {
    let req = client.last_request();
    let expected = Request {
        call: Call::GetDeployment,
        method: reqwest::Method::GET,
        path: "/deployments/dpl_1".into(),
        url: "http://mock/deployments/dpl_1".into(),
    };
    assert_eq!(req, expected);
}
```

The lint rule flags the first pattern when the number of field-asserts on a single receiver variable meets or exceeds the threshold.

### Existing instances in the codebase

A search of `agent/tests/` shows this pattern in several files:
- `server/handlers.rs` -- multiple tests assert 3-5 fields on `actual`
- `events/model.rs` -- tests assert fields on `deserialized`, `actual`, `event`
- `cache/single_thread.rs` and `cache/concurrent.rs` -- tests assert fields on `read_entry`, `entry`, `found`
- `server/sse.rs` -- tests assert fields on `event`

### Root receiver extraction algorithm

Given an expression that is the first argument to `assert_eq!`, extract the root variable being asserted on:

1. `req.call` -- `Expr::Field { base: Expr::Path("req"), member: "call" }` -- root = `req`
2. `req.token.as_deref()` -- `Expr::MethodCall { receiver: Expr::Field { base: Expr::Path("req"), member: "token" } }` -- root = `req` (walk through method calls to find the innermost field access)
3. `req.data[0].name` -- `Expr::Field { base: Expr::Index { expr: Expr::Field { ... } } }` -- root = `req` (walk through indexing)
4. `result` -- `Expr::Path("result")` -- no field access, skip (not a field-by-field assert)
5. `foo()` -- `Expr::Call` -- no field access, skip

The algorithm walks the expression tree inward (through `Field`, `MethodCall`, `Index`, `Paren`, `Reference`, `Try`, `Unary`) until it reaches either:
- An `Expr::Path` with a single segment (the root variable name), or
- Something that is not a variable path (function call, literal, etc.), in which case skip it.

### Project layout

```
tools/
  lint-asserts/
    Cargo.toml          # [package] name = "lint-asserts"
    src/
      main.rs           # CLI entry point, file walking, output
      detect.rs         # AST analysis: find test fns, collect assert_eq! calls,
                        #   extract root receivers, group and threshold
      extract.rs        # Root receiver extraction from syn::Expr
```

### Dependencies

```toml
[package]
name = "lint-asserts"
version = "0.1.0"
edition = "2021"

[dependencies]
clap = { version = "4", features = ["derive"] }
syn = { version = "2", features = ["full", "visit"] }
walkdir = "2"
proc-macro2 = "1"

[dev-dependencies]
tempfile = "3"
```

The `visit` feature gives the `syn::visit::Visit` trait for clean AST traversal. `proc-macro2` is needed for working with token streams from macro invocations.

## Plan of Work

### Milestone 1: Scaffold `tools/lint-asserts/` crate

Create the crate structure with a minimal CLI that compiles and prints help.

Files to create:
- `tools/lint-asserts/Cargo.toml` -- package definition with dependencies listed above.
- `tools/lint-asserts/src/main.rs` -- CLI skeleton using `clap` derive API. Accepts `--path <PATH>...` (required, one or more directories), `--threshold <N>` (default 4). Placeholder call to detection logic.
- `tools/lint-asserts/src/detect.rs` -- empty module with public function signature.
- `tools/lint-asserts/src/extract.rs` -- empty module with public function signature.

Verify:
```bash
cd tools/lint-asserts && cargo build
cd tools/lint-asserts && cargo run -- --help
```

Expected: prints usage with `--path` and `--threshold` options.

### Milestone 2: Implement AST walking and field-assert detection

Implement the core analysis in `detect.rs` and `extract.rs`.

**`extract.rs`:**
- `pub fn root_receiver(expr: &syn::Expr) -> Option<String>` -- walks the expression tree inward through `Field`, `MethodCall`, `Index`, `Paren`, `Reference`, `Try`, `Unary` to find the root `Expr::Path` single-segment identifier. Returns `None` if the expression does not involve a field access (i.e., if the outermost relevant node is not `Expr::Field`).

**`detect.rs`:**

Data types:
```rust
pub struct Violation {
    pub file: PathBuf,
    pub line: usize,        // line of the first assert_eq! in the group
    pub test_fn: String,    // name of the test function
    pub receiver: String,   // the root variable name
    pub count: usize,       // number of field-asserts on this receiver
}
```

Core logic:
1. `pub fn check_file(path: &Path, threshold: usize) -> Vec<Violation>` -- reads and parses the file with `syn::parse_file`. On parse failure (e.g. generated code with macros), silently skip the file and return empty.
2. For each `syn::ItemFn` in the file whose attributes include `#[test]` or `#[tokio::test]`:
   a. Check the function body for the escape hatch comment `// lint:allow(field-by-field-assert)`. To detect this, search the token stream or convert the block to a string and check. Alternatively, iterate the statements looking for an expression-level comment. Since `syn` does not preserve comments in the AST, read the original source lines spanned by the function and search for the escape hatch string in those raw lines.
   b. Walk all statements in the function body. For each `Expr::Macro` where the macro path ends in `assert_eq`:
      - Parse the first argument from the macro's token stream (everything before the first top-level comma).
      - Call `root_receiver()` on the parsed expression.
      - If it returns `Some(name)`, record `(name, line_number)`.
   c. Group records by receiver name. For any group with `count >= threshold`, emit a `Violation`.

Wire into `main.rs`: walk all `--path` directories for `*.rs` files, call `check_file` on each, collect violations, print them, and exit with code 1 if any violations found.

Verify:
```bash
cd tools/lint-asserts && cargo run -- --path ../../agent/tests
```

Expected: reports violations in test files that have 4+ field-asserts on the same variable. Output format matches:
```
agent/tests/server/handlers.rs:345: 3 assert_eq! calls on fields of `actual` — consider constructing an expected struct [field-by-field-assert]
```

### Milestone 3: Implement CLI, output formatting, and escape hatch

Refine the output and ensure the escape hatch works.

**Output format:**
```
<filepath>:<line>: <N> assert_eq! calls on fields of `<var>` — consider constructing an expected struct [field-by-field-assert]
```

Where `<filepath>` is relative to the current working directory (use `pathdiff` or strip the common prefix), `<line>` is the line of the first `assert_eq!` in the flagged group, and `<N>` is the count.

**Escape hatch:**
- Read the raw source bytes of each file.
- For each test function, extract the byte range from the function's opening brace to closing brace using `syn::spanned::Spanned` (requires `proc-macro2` span info).
- Search that range for the literal string `lint:allow(field-by-field-assert)`.
- If found, skip the function entirely.

**Edge cases:**
- Files that fail to parse (macro-heavy generated code): skip silently.
- Test functions inside `mod tests { }` blocks: the `ItemMod` contains `ItemFn`s. Walk into modules recursively to find all test functions.
- `#[tokio::test]` attribute: the path segments are `["tokio", "test"]`. Check both `#[test]` and `#[tokio::test]` (and any path ending in `::test`).
- Async test functions (`async fn`): handled identically, the attribute is what matters.

Verify:
```bash
# Create a temp file with the escape hatch and confirm it is skipped
cd tools/lint-asserts && cargo test
```

### Milestone 4: Add tests

Add an embedded `#[cfg(test)]` module in each source file plus integration-style tests.

**Unit tests in `extract.rs`:**
- `test_simple_field_access` -- `req.call` returns `Some("req")`
- `test_nested_field_access` -- `req.inner.call` returns `Some("req")`
- `test_method_on_field` -- `req.token.as_deref()` returns `Some("req")`
- `test_no_field_access` -- `result` returns `None`
- `test_function_call` -- `foo()` returns `None`
- `test_indexed_field` -- `req.items[0]` returns `Some("req")`
- `test_reference` -- `&req.call` returns `Some("req")`
- `test_try_operator` -- `req.call?` returns `Some("req")` (unlikely in tests but handle gracefully)

**Unit tests in `detect.rs`:**
- `test_flags_four_field_asserts` -- a test function with 4 `assert_eq!` on fields of `req` produces one violation.
- `test_below_threshold_no_violation` -- 3 `assert_eq!` on fields of `req` with threshold 4 produces no violation.
- `test_different_receivers_not_grouped` -- 2 asserts on `req` and 2 on `resp` produces no violation at threshold 4.
- `test_escape_hatch_suppresses` -- a test function with the `// lint:allow(field-by-field-assert)` comment and 5 field asserts produces no violation.
- `test_non_test_function_ignored` -- a regular function (no `#[test]` attribute) with 5 field asserts produces no violation.
- `test_tokio_test_detected` -- a function with `#[tokio::test]` and 4 field asserts produces one violation.
- `test_nested_mod_tests` -- test functions inside `mod tests { }` are detected.
- `test_unparseable_file_skipped` -- a file with invalid syntax produces no violations and no panic.
- `test_custom_threshold` -- threshold of 2 flags groups with 2+ asserts.

**Integration test in `main.rs` or a separate test file:**
- Create a temporary directory with a `.rs` file containing known patterns.
- Run `check_file` and assert the expected violations.

Verify:
```bash
cd tools/lint-asserts && cargo test
```

Expected: all tests pass.

### Milestone 5: Integrate into lint pipeline and CI

**`scripts/lib/lint.sh`:**

Add a new section after the "Custom Linter" (import linter) block and before "Cargo fmt". The new section header is "Assert Linter". It runs:

```bash
echo "Assert Linter"
echo "-------------"
cargo run --manifest-path "$REPO_ROOT/tools/lint-asserts/Cargo.toml" -- --path "$ASSERT_LINT_PATHS"
echo ""
```

The `ASSERT_LINT_PATHS` environment variable is set by the caller script (`scripts/lint.sh`). This follows the same pattern as `IMPORT_LINT_PATHS` -- the shared library script reads env vars set by the per-crate wrapper.

**`scripts/lint.sh` (the agent-level wrapper):**

Add:
```bash
export ASSERT_LINT_PATHS="$REPO_ROOT/agent/tests"
```

**`tools/lint/scripts/lint.sh` (the lint-tool-level wrapper):**

The lint tool's own lint script (`tools/lint/scripts/lint.sh`) does not need assert linting -- it has no test files with this pattern. Set `ASSERT_LINT_PATHS` to empty or do not export it, and have `scripts/lib/lint.sh` skip the assert linter when the variable is unset.

**CI (`ci.yml`):**

No changes needed. The `lint` job already runs `LINT_FIX=0 ./scripts/lint.sh`, which calls `scripts/lib/lint.sh`. The new assert linter step will execute automatically. The `tools` job runs `tools/lint/scripts/lint.sh` which will skip the assert linter (no paths configured).

**AGENTS.md:**

Update the "Linting" section to mention the assert linter:
```
In CI, the Lint workflow runs:
- `cargo run --manifest-path tools/lint-asserts/Cargo.toml -- --path agent/tests`
```

Verify:
```bash
# Full lint pipeline
cd /home/ben/miru/workbench3/agent && ./scripts/lint.sh

# CI mode
cd /home/ben/miru/workbench3/agent && LINT_FIX=0 ./scripts/lint.sh
```

Expected: lint pipeline completes. The assert linter step either reports violations (which must be addressed by adding escape hatches or refactoring tests) or passes clean.

**Important:** If the linter flags existing test files, there are two options:
1. Refactor the tests to use expected structs (preferred, but out of scope for this plan).
2. Add `// lint:allow(field-by-field-assert)` to existing tests that intentionally use field-by-field asserts (pragmatic, keeps this plan focused on tooling).

The initial integration should add escape hatches to any existing tests that are flagged, so that `scripts/lint.sh` passes clean. A follow-up plan can address refactoring those tests.

## Concrete Steps

All commands are run from the agent repo root (`/home/ben/miru/workbench3/agent`) unless otherwise noted.

### Milestone 1

```bash
mkdir -p tools/lint-asserts/src
```

Create the following files:
- `tools/lint-asserts/Cargo.toml`
- `tools/lint-asserts/src/main.rs` (CLI skeleton with clap derive, module declarations)
- `tools/lint-asserts/src/detect.rs` (empty module, public function stubs)
- `tools/lint-asserts/src/extract.rs` (empty module, public function stubs)

Verify:
```bash
cd tools/lint-asserts && cargo build
cd tools/lint-asserts && cargo run -- --help
```

### Milestone 2

Implement `extract.rs`:
- `root_receiver(expr: &syn::Expr) -> Option<String>` -- recursive descent through Field, MethodCall, Index, etc.

Implement `detect.rs`:
- `Violation` struct
- `check_file(path: &Path, source: &str, threshold: usize) -> Vec<Violation>`
- Helper: `is_test_fn(item_fn: &syn::ItemFn) -> bool`
- Helper: `has_escape_hatch(source: &str, fn_span_start: usize, fn_span_end: usize) -> bool`
- Helper: `parse_assert_eq_first_arg(mac: &syn::ExprMacro) -> Option<syn::Expr>`

Wire into `main.rs`: directory walking, file reading, violation collection, output.

Verify:
```bash
cd tools/lint-asserts && cargo run -- --path ../../agent/tests --threshold 4
```

### Milestone 3

Polish output format, handle edge cases (parse failures, nested modules, tokio::test).

Verify manually with known test files.

### Milestone 4

Add `#[cfg(test)]` modules to `extract.rs`, `detect.rs`, and optionally `main.rs`.

```bash
cd tools/lint-asserts && cargo test
```

Expected: all tests pass.

### Milestone 5

Edit `scripts/lib/lint.sh` to add the assert linter section.
Edit `scripts/lint.sh` to export `ASSERT_LINT_PATHS`.
Update `AGENTS.md` linting section.
Add escape hatches to any existing tests that are flagged.

```bash
# Run full lint pipeline
./scripts/lint.sh
```

Expected: passes clean.

## Test Steps

These are the specific test commands that must pass before this work is considered complete.

1. **Tool unit tests:**
   ```bash
   cd tools/lint-asserts && cargo test
   ```
   All tests in `extract.rs`, `detect.rs`, and any integration tests must pass.

2. **Tool builds cleanly:**
   ```bash
   cd tools/lint-asserts && cargo build 2>&1 | grep -c "warning" | xargs test 0 -eq
   ```
   Zero warnings from the lint-asserts crate itself.

3. **Tool runs against agent tests without panic:**
   ```bash
   cargo run --manifest-path tools/lint-asserts/Cargo.toml -- --path agent/tests --threshold 4
   ```
   Exits with 0 (no violations after escape hatches are added) or 1 (violations listed cleanly, no panics).

4. **Agent test suite unaffected:**
   ```bash
   ./scripts/test.sh
   ```
   All agent tests pass. The lint tool does not modify any source files.

5. **Full lint pipeline:**
   ```bash
   ./scripts/lint.sh
   ```
   All steps pass, including the new assert linter step.

6. **CI mode:**
   ```bash
   LINT_FIX=0 ./scripts/lint.sh
   ```
   Passes clean (the assert linter has no `--fix` mode, so `LINT_FIX` does not affect it, but the overall pipeline must still work).

## Validation

**Preflight must report clean before changes are published.** Specifically:

1. `cd tools/lint-asserts && cargo test` -- all linter tests pass.
2. `cd tools/lint-asserts && cargo clippy -- -D warnings` -- no clippy warnings in the tool.
3. `cargo run --manifest-path tools/lint-asserts/Cargo.toml -- --path agent/tests` -- exit code 0 (all violations either refactored or suppressed with escape hatches).
4. `./scripts/test.sh` -- all agent tests pass (no behavioral regressions).
5. `./scripts/lint.sh` -- full lint pipeline passes, including the new assert linter step.
6. `LINT_FIX=0 ./scripts/lint.sh` -- CI-mode lint pipeline passes.

All six checks must be green. If any fail, diagnose and fix before opening a PR.
