# Add a function-length linter (50-line limit) and fix all violations

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

All paths in this plan are relative to the agent repository root (the directory containing `Cargo.toml`, `agent/`, `tools/`) unless stated otherwise. Every command states its working directory.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `mirurobotics/agent`) | read-write | New `funclen` check in `tools/lint`, refactors of oversized functions in `agent/src` and `tools/lint/src`, `AGENTS.md` doc update. |
| `gotools/` (`mirurobotics/gotools`) | read-only | Reference semantics: `internal/services/lint/linter/funclen/funclen.go` and its test file. Not required to implement this plan — all needed semantics are embedded below. |

This plan lives in `plans/` of the agent repo because all code changes happen here. Generated code in `libs/` (backend-api, device-api) is never linted or edited.

## Purpose / Big Picture

Miru's Go repos enforce a 50-line function limit via the `funclen` linter in gotools (functions longer than 50 non-blank, non-comment body lines fail lint). The agent repo (Rust) has no such check; eight functions in `agent/src` and three in `tools/lint/src` currently exceed the limit.

After this change:

- Running `LINT_FIX=0 ./scripts/lint.sh` (or CI's Lint job) fails with a `[funclen]` diagnostic whenever a function or closure in `agent/src` exceeds 50 non-blank, non-comment body lines, and passes on the current tree because every violation has been refactored (or, in one sanctioned case, suppressed).
- The lint tool dogfoods the rule on its own sources via the existing `tools/lint/scripts/lint.sh` invocation.
- Developers can suppress a finding with `// lint:allow(funclen)` on the `fn` line or the line immediately above it, matching the repo's existing `lint:allow(field-by-field-assert)` convention.

## Progress

- [ ] Milestone 1: implement `funclen` check in `tools/lint` with unit tests; wire into CLI.
- [ ] Milestone 2: refactor the 3 oversized functions in `tools/lint/src`; tools lint passes.
- [ ] Milestone 3: refactor the 8 violations in `agent/src` (7 refactors + 1 suppression); repo lint passes.
- [ ] Milestone 4: update `AGENTS.md`; preflight reports clean.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: extend `tools/lint` with a syn-based `funclen` check instead of enabling clippy's `too_many_lines`.
  Rationale: clippy cannot reproduce the gotools semantics — exact non-blank/non-comment body-line counting, skipping test code, closure coverage, and comment-based suppression (`lint:allow(...)`, the repo's existing convention). `too_many_lines` is attribute-suppressed, has different counting rules, and would apply to test targets. The repo already owns a syn v2 linter (`tools/lint`, package `lint-imports`) with `span-locations` enabled, wired into scripts and CI; adding a check there mirrors both the gotools architecture and the existing `field-by-field-assert` check.
  Date/Author: 2026-07-04 / plan author.
- Decision: the funclen check runs automatically whenever `--path` is given (same trigger as import linting); a new `--funclen-threshold` flag (default 50, `0` disables) mirrors `--assert-threshold`.
  Rationale: all existing invocations (`scripts/lib/lint.sh` loop over `IMPORT_LINT_PATHS`, CI, `tools/lint/scripts/lint.sh`) pick up the check with zero flag or workflow changes. Test-path skipping (below) makes this safe even though `IMPORT_LINT_PATHS` includes `agent/tests`.
  Date/Author: 2026-07-04 / plan author.
- Decision: `tools/lint/src` is itself checked (dogfooding), because `tools/lint/scripts/lint.sh` already passes `--path tools/lint/src`. Its 3 oversized functions are refactored in Milestone 2.
  Date/Author: 2026-07-04 / plan author.
- Decision: test code is exempt, defined as (a) any file whose path contains a `tests` directory component (covers `agent/tests/`), (b) items inside `#[cfg(test)]` modules, (c) `#[test]` / `#[tokio::test]` functions, (d) items gated `#[cfg(feature = "test")]` (the repo's convention for test-only mocks/setters, per `AGENTS.md`). Mirrors gotools skipping `_test.go` files and test helpers.
  Date/Author: 2026-07-04 / plan author.
- Decision: suppression is `// lint:allow(funclen)` appearing on the `fn` keyword line or the line immediately before it (for closures: the closure's first line or the line before). Mirrors gotools' "on the line or the line immediately before" rule while using the repo's `lint:allow(...)` syntax rather than Go's `//nolint:`.
  Date/Author: 2026-07-04 / plan author.
- Decision: `Worker::run` in `agent/src/cache/concurrent.rs` (187 lines) is the single planned suppression, not a refactor. It is a flat actor dispatch table — one match arm per `Command` variant, each arm a single `dispatch!` invocation; splitting it into sub-matches would need catch-all `unreachable!()` arms. gotools applies the same exemption to its own rule-dispatch table (`//nolint:funclen // dispatch table grows with each new rule`). Final call remains with the implementer; everything else must be refactored.
  Date/Author: 2026-07-04 / plan author.
- Decision: milestone order is linter-first (implement, then fix violations). Intermediate commits fail repo-wide lint until Milestone 3 completes; this is accepted within the PR because the linter is the natural verification tool for the refactor milestones, and the final state is green.
  Date/Author: 2026-07-04 / plan author.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The agent repo is a Rust workspace. `agent/` is the production binary crate (`miru-agent`, sources in `agent/src`, integration tests in `agent/tests`). `libs/backend-api` and `libs/device-api` are generated OpenAPI clients — never lint or edit them. `tools/lint` is a standalone crate (package name `lint-imports`, its own `[workspace]`, own `Cargo.lock`) implementing the repo's custom linter using `syn` v2 (features `full`, `visit`) and `proc-macro2` (feature `span-locations`, which makes `span.start().line` return 1-based source lines). No new dependencies are needed.

Key existing files:

- `tools/lint/src/main.rs` — declares modules (`app`, `asserts`, `checker`, `classifier`, `config`, `fixer`, `normalizer`, `parser`) and calls `app::run`.
- `tools/lint/src/app/mod.rs` — `Cli` struct (clap derive: `--path`, `--fix`, `--config`, `--assert-paths`, `--assert-threshold`), `run_from_dir` orchestration, `rust_files()` directory walker, `run_assert_check()` (the model for the new funclen pass), exit-code logic, and `#[cfg(test)] mod tests` with `Cli`-constructing helpers.
- `tools/lint/src/asserts/detect.rs` — the `field-by-field-assert` check: `check_file(path, source, threshold) -> Vec<Violation>`, syn-based, skips unparseable files, comment escape hatch scanned over raw source lines. The funclen module copies this shape.
- `scripts/lint.sh` — repo lint entry point; exports `IMPORT_LINT_PATHS="$REPO_ROOT/agent/src $REPO_ROOT/agent/tests"`, `ASSERT_LINT_PATHS`, `IMPORT_LINT_CONFIG` and executes `scripts/lib/lint.sh`, which loops the paths and runs `cargo run --manifest-path "$REPO_ROOT/tools/lint/Cargo.toml" -- --path <path> [--fix] --config <config> [--assert-paths ...]`, then fmt/machete/diet/audit/clippy. `LINT_FIX=0` (CI mode) is check-only.
- `tools/lint/scripts/lint.sh` — same shared lib with `IMPORT_LINT_PATHS="$LINT_DIR/src"`, so the lint tool lints itself.
- `.github/workflows/ci.yml` — `lint` job runs `LINT_FIX=0 ./scripts/lint.sh`; `tools` job runs `LINT_FIX=0 ./tools/lint/scripts/lint.sh` plus `./tools/lint/scripts/covgate.sh`. **No workflow changes are needed** — both pick up the new check through `--path`.
- `scripts/preflight.sh` — runs repo lint, repo covgate, tools lint, tools covgate in parallel; prints `Preflight clean` on success.
- `tools/lint/src/.covgate` contains `0`, meaning the coverage gate for `tools/lint` is threshold-skipped — but `tools/lint/scripts/covgate.sh` still runs its tests, so all tests must pass. Agent modules (`agent/src/*/.covgate`) have real thresholds enforced by `./scripts/covgate.sh`.
- `AGENTS.md` — "Linting" section documents the custom linter and the `lint:allow(field-by-field-assert)` escape hatch; must gain funclen documentation.

Reference semantics (gotools `funclen.go`, embedded here so the Go repo is not required): a function body's length is the number of lines strictly between its opening and closing braces (brace lines excluded) that are neither blank nor `//`-comment-only after trimming whitespace. Named functions and anonymous functions are both checked. Limit 50; `<= 0` disables. Test files are skipped. Suppression comment on the function's line or the line immediately before exempts it. Message formats: `function <name> is <n> lines (limit <max>)` / `anonymous function is <n> lines (limit <max>)`.

Term: "violation worklist" below lists measured counts using exactly these semantics (measured 2026-07-04 on branch `claude/agent-function-length-linter-tks6rb`, tip `321be41`).

## Plan of Work

### Milestone 1 — implement the funclen check in tools/lint

Create `tools/lint/src/funclen/mod.rs` (new module; add `mod funclen;` to `tools/lint/src/main.rs` in alphabetical order). Follow the repo import-order convention (`// standard crates` / `// internal crates` / `// external crates` headers). Public surface:

    pub struct Violation {
        pub file: PathBuf,
        pub line: usize,           // 1-based line of the fn keyword / closure start
        pub name: Option<String>,  // None for closures
        pub count: usize,          // non-blank, non-comment body lines
        pub limit: usize,
    }

    pub fn check_file(path: &Path, source: &str, max_lines: usize) -> Vec<Violation>

Behavior of `check_file` (each rule below needs a unit test):

1. `max_lines == 0` returns empty (disabled).
2. If any component of `path` equals `tests`, return empty (test files exempt).
3. Parse with `syn::parse_file`; on error return empty (consistent with `asserts::detect::check_file`).
4. Walk the AST with a `syn::visit::Visit` implementor holding `lines: Vec<&str>` (from `source.lines()`). Override:
   - `visit_item_mod`: return without recursing when the mod has a test cfg attribute (helper `is_test_cfg`, below).
   - `visit_item_impl` and `visit_item_trait`: same test-cfg skip, else delegate to `syn::visit::visit_item_*`.
   - `visit_item_fn` / `visit_impl_item_fn`: skip when `is_test_cfg(attrs)` or `is_test_fn(attrs)` (reuse the attribute-matching approach of `is_test_fn` in `tools/lint/src/asserts/detect.rs`: `#[test]`, `#[tokio::test]`, `#[tokio::test(...)]`). Otherwise check the block, then delegate so nested closures are also visited.
   - `visit_trait_item_fn`: check the default body if present (skip test-cfg), then delegate.
   - `visit_expr_closure`: when the body is `syn::Expr::Block`, check that block with `name = None`, anchor line = `closure.or1_token.span().start().line`; always delegate.
5. `is_test_cfg(attrs)`: any attribute whose meta is `Meta::List` with path `cfg` and whose token stream (`.tokens.to_string()`) contains `test`. This intentionally covers `#[cfg(test)]`, `#[cfg(feature = "test")]`, and `#[cfg(any(test, ...))]`.
6. Line counting for a `syn::Block`: `open = block.brace_token.span.open().start().line`, `close = block.brace_token.span.close().start().line` (both 1-based). Count source lines with 1-based numbers in `open+1 ..= close-1` whose trimmed text is non-empty and does not start with `//`. A single-line body (`open == close`) counts 0. This is the exact gotools rule.
7. Suppression: before recording a violation for anchor line `L` (the `fn` keyword line via `sig.fn_token.span().start().line`, or the closure start line), check whether source line `L` or line `L-1` contains the substring `lint:allow(funclen)`; if so, skip. Note for users: place the comment between any attributes and the `fn` line, or trailing on the `fn` line — a comment above doc comments/attributes will NOT match (same one-line window as gotools).
8. Record `Violation { file, line: L, name, count, limit: max_lines }` when `count > max_lines`.

Wire into `tools/lint/src/app/mod.rs`:

- Add to `Cli`: `#[arg(long = "funclen-threshold", default_value_t = 50)] pub funclen_threshold: usize,` (doc comment: max non-blank, non-comment body lines per function; 0 disables).
- Add `run_funclen_check(base_dir, cli, stdout) -> Vec<funclen::Violation>`, modeled on `run_assert_check`: when `cli.path` is `Some(p)` and `cli.funclen_threshold > 0`, iterate `rust_files(base_dir, p)`, read each file (skip on read error), collect `funclen::check_file` results; sort by `(file, line)`; print each, stripping the cwd prefix like `run_assert_check` does:

      {file}:{line}: function `{name}` is {count} lines (limit {limit}) [funclen]
      {file}:{line}: closure is {count} lines (limit {limit}) [funclen]

- Call it in `run_from_dir` after the import-check loop and assert check. Include the count in exit-code logic in BOTH modes: in check mode add to `total_issues`; in `--fix` mode return 1 when funclen violations exist (funclen has no auto-fix; same pattern as assert violations in fix mode). Update the two `Cli`-constructing test helpers (`cli(...)`, `assert_cli(...)`) in `app/mod.rs` tests with `funclen_threshold: 50` (or 0 where a test must not trigger funclen).

Unit tests: `#[cfg(test)] mod tests` inside `tools/lint/src/funclen/mod.rs` (the repo's pattern — tests inline in the module). Port the gotools test matrix, using small thresholds (e.g. 3–5) with generated bodies (`"let x = 1;\n".repeat(n)`) so fixtures stay short:

- under limit, exactly at limit (no violation), over limit (violation with correct `count`, `line`, `name`).
- blank lines not counted; `//` comment lines not counted.
- closure with block body over limit reported with `name == None`.
- `impl` method and trait default method over limit reported; trait method without body ignored.
- `#[test]` fn, `#[tokio::test]` fn, `#[cfg(test)] mod` content, and `#[cfg(feature = "test")]` fn all exempt.
- path containing a `tests` component exempt (pass `Path::new("agent/tests/foo.rs")`).
- suppression on the line above the fn; suppression trailing on the fn line; `lint:allow(field-by-field-assert)` does NOT exempt funclen.
- threshold 0 disables; unparseable source returns empty.

App-level tests in `tools/lint/src/app/mod.rs` tests: a temp dir with an over-limit fn → `run` returns 1 and stdout contains `[funclen]` and `violation(s) found.`; same dir with `--fix` → still returns 1; clean file → 0. Use `funclen_threshold: 3`-style small values.

Also verify the new module itself obeys the rule (it will be dogfooded): keep every function in `funclen/mod.rs` and the new app code under 50 body lines.

Commit (from repo root): `feat(lint): add funclen check enforcing 50-line function limit`

### Milestone 2 — refactor tools/lint violations

Measured worklist (file : line, function, body lines):

| Location | Function | Lines | Refactor guidance |
|----------|----------|-------|-------------------|
| `tools/lint/src/parser/mod.rs:54` | `parse` | 97 | Extract the multi-line `use`-statement consumption loop into `fn parse_use_statement(lines: &[&str], i: usize, pending_attrs: &mut Vec<String>) -> (UseStatement, usize)` (returns statement + next index), and the comment/attr branches into small helpers. Behavior must be byte-identical; the existing parser/checker/fixer tests are the safety net. |
| `tools/lint/src/fixer/mod.rs:13` | `fix_file` | 83 | Extract `fn bucket_by_group(...)`, `fn render_import_block(...) -> String`, and `fn splice_import_block(content, block, new_block) -> String`. |
| `tools/lint/src/checker/mod.rs:74` | `check_headers` | 60 | Extract the backward walk that locates the header comment for a group's first use into `fn find_header(block, first_line, expected_label, diagnostics)` (or return an enum `Found/Wrong/Missing`). |

After refactoring, from repo root: `LINT_FIX=0 ./tools/lint/scripts/lint.sh` passes (funclen included), and `cd tools/lint && cargo test` passes.

Commit (from repo root): `refactor(lint): split oversized lint tool functions under funclen limit`

### Milestone 3 — fix agent/src violations

Measured worklist:

| Location | Function | Lines | Action |
|----------|----------|-------|--------|
| `agent/src/cache/concurrent.rs:153` | `Worker::run` | 187 | Suppress: add `// lint:allow(funclen) — actor dispatch table; one arm per Command variant` immediately above `pub async fn run`. See Decision Log. |
| `agent/src/main.rs:153` | `run_agent` | 68 | Extract settings loading (`read settings file` + log-level reload) into a helper returning `Option<Settings>`/early-return value, and `AppOptions` construction into `fn build_app_options(settings) -> AppOptions`. |
| `agent/src/storage/mod.rs:90` | `Storage::init` | 67 | Seven near-identical "spawn store + Arc" blocks. Extract grouped helpers (e.g. config-instance stores; deployment/release/upload-rule/git-commit stores) each returning the Arcs plus join handles, or extract the `shutdown_handle` assembly and `Storage` struct literal into a builder helper. |
| `agent/src/workers/mqtt.rs:74` | `run_impl` | 59 | Extract pre-loop setup (syncer subscribe fallback, device read, client init → `State`) into `async fn init_state(...) -> (State, Arc<Device>, watch::Receiver<SyncEvent>)`, keeping the select loop in `run_impl`. |
| `agent/src/app/state.rs:28` | `AppState::init` | 54 | Extract the auth-file assertions + token file setup, or the `SyncerArgs` construction, into helpers. |
| `agent/src/workers/token_refresh.rs:34` | `run_token_refresh_worker` | 54 | Extract the refresh-attempt match into `async fn refresh_once(token_mngr, options, err_streak: &mut u32) -> Duration` returning the next wait. |
| `agent/src/app/run.rs:420` | `shutdown_impl` | 51 | Five near-identical worker-handle join blocks. Extract `async fn join_worker(handle: Option<JoinHandle<()>>, name: &str) -> Result<(), ServerErr>` for the four uniform ones (note: `socket_server_handle` uses `??` — its handle carries a `Result`, handle separately or via a second helper). |
| `agent/src/filesys/file.rs:172` | `File::write_bytes` | 51 | Two independent branches. Extract `async fn write_bytes_atomic(&self, buf, overwrite)` and `async fn write_bytes_direct(&self, buf, overwrite)`. |

Rules for the refactors: behavior-preserving only; helpers stay in the same file/module, private, named per repo conventions; keep the `// standard/internal/external crates` import headers intact; each extracted helper must itself be under 50 body lines. Do not touch `agent/tests` except where a helper rename would break compilation (none expected — helpers are new private fns). Watch `cargo diet` (runs in repo lint with `RUN_DIET=1`): every new helper must actually be called.

After refactoring, from repo root: `LINT_FIX=0 ./scripts/lint.sh` passes end-to-end; `./scripts/test.sh` passes; `./scripts/covgate.sh` passes (refactors are moves, coverage should hold; if a module dips below its `.covgate` threshold, add targeted unit coverage for the extracted helper rather than lowering the gate).

Commit (from repo root): `refactor: split oversized functions to satisfy funclen lint`

### Milestone 4 — documentation and preflight

Edit `AGENTS.md`, "Linting" section, first CI bullet: extend the description of the custom linter invocation to mention function-length linting — production functions and closures are limited to 50 non-blank, non-comment body lines (test code exempt); suppress with `// lint:allow(funclen)` on the `fn` line or the line immediately above. Keep it to 2–3 sentences.

Run `./scripts/preflight.sh` from the repo root and iterate until it prints `Preflight clean`.

Commit (from repo root): `docs: document funclen lint rule and suppression`

## Concrete Steps

All commands run from the repo root (the agent repository checkout, branch `claude/agent-function-length-linter-tks6rb`) unless noted.

1. Milestone 1 development loop:

       cd tools/lint
       cargo test                # all existing + new funclen tests pass
       cargo clippy --all-targets -- -D warnings

2. Verify the check fires on the current tree (before Milestones 2–3). Expected: exit code 1 and exactly these 8 findings (order: sorted by file, then line):

       cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/src --config .lint-imports.toml

       agent/src/app/run.rs:420: function `shutdown_impl` is 51 lines (limit 50) [funclen]
       agent/src/app/state.rs:28: function `init` is 54 lines (limit 50) [funclen]
       agent/src/cache/concurrent.rs:153: function `run` is 187 lines (limit 50) [funclen]
       agent/src/filesys/file.rs:172: function `write_bytes` is 51 lines (limit 50) [funclen]
       agent/src/main.rs:153: function `run_agent` is 68 lines (limit 50) [funclen]
       agent/src/storage/mod.rs:90: function `init` is 67 lines (limit 50) [funclen]
       agent/src/workers/mqtt.rs:74: function `run_impl` is 59 lines (limit 50) [funclen]
       agent/src/workers/token_refresh.rs:34: function `run_token_refresh_worker` is 54 lines (limit 50) [funclen]

       8 violation(s) found.

   (Line numbers may drift a few lines if main has moved; match on file + function name.) And on the tool itself, expected 3 findings (`check_headers` 60, `fix_file` 83, `parse` 97):

       cargo run --manifest-path tools/lint/Cargo.toml -- --path tools/lint/src --config tools/lint/.lint-imports.toml

3. Commit Milestone 1: `git add tools/lint && git commit -m "feat(lint): add funclen check enforcing 50-line function limit"`.

4. Milestone 2 loop: refactor, then

       cd tools/lint && cargo test
       cd .. && LINT_FIX=0 ./tools/lint/scripts/lint.sh    # expect "Lint complete", exit 0

   Commit: `refactor(lint): split oversized lint tool functions under funclen limit`.

5. Milestone 3 loop: refactor one function at a time, re-running after each:

       cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/src --config .lint-imports.toml
       ./scripts/test.sh          # cargo test --package miru-agent --features test (RUST_LOG=off)

   When the funclen run reports `0` findings (silent, exit 0):

       LINT_FIX=0 ./scripts/lint.sh   # full check-only lint: custom linter, fmt, machete, diet, audit, clippy
       ./scripts/covgate.sh           # per-module coverage gates

   Commit: `refactor: split oversized functions to satisfy funclen lint`.

6. Milestone 4: edit `AGENTS.md`, then

       ./scripts/preflight.sh

   Expected tail of output: `Preflight clean`. Commit: `docs: document funclen lint rule and suppression`.

Note: local `cargo audit` (inside lint) needs network access to refresh the advisory DB; if it fails for environmental reasons unrelated to this change, note it in Surprises & Discoveries and rely on CI for that sub-check — all other lint sub-checks must pass locally.

## Validation and Acceptance

Acceptance is behavioral:

1. Detection: create a scratch violation and see it caught, then remove it:

       cat >> agent/src/version/mod.rs <<'EOF'

       fn funclen_demo() {
       EOF
       for i in $(seq 1 51); do echo "    let _x = $i;" >> agent/src/version/mod.rs; done
       echo "}" >> agent/src/version/mod.rs
       cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/src --config .lint-imports.toml
       # expect: agent/src/version/mod.rs:<line>: function `funclen_demo` is 51 lines (limit 50) [funclen]
       #         1 violation(s) found.    (exit code 1)
       git checkout -- agent/src/version/mod.rs

2. Suppression: with the same scratch function plus `// lint:allow(funclen)` on the line above `fn funclen_demo`, the run above reports nothing and exits 0.

3. Clean tree: `LINT_FIX=0 ./scripts/lint.sh` and `LINT_FIX=0 ./tools/lint/scripts/lint.sh` both exit 0 on the final tree.

4. Tests: `cd tools/lint && cargo test` passes with the new funclen unit tests (roughly 15 new tests) and updated app tests; `./scripts/test.sh` passes; `./scripts/covgate.sh` and `./tools/lint/scripts/covgate.sh` pass (tools threshold is 0 = skip, but its tests must pass).

5. Preflight gate: `./scripts/preflight.sh` prints `Preflight clean`. **This must be observed before the branch is pushed / the PR is opened — do not publish changes with a non-clean preflight.**

6. CI parity: no `.github/workflows/*` changes; the existing `lint` and `tools` jobs exercise the new check through the unchanged script entry points.

## Idempotence and Recovery

- All lint/test commands are read-only and safely repeatable. `LINT_FIX=1` (default local `scripts/lint.sh`) rewrites imports/fmt in place; run it only on a committed tree so `git diff` shows what it changed.
- Each milestone is an independent commit; to back out a bad refactor, `git revert <commit>` (or `git checkout -- <file>` before committing). The linter (Milestone 1) has no behavioral coupling to the refactors, so reverting a Milestone 3 refactor only re-introduces a lint finding, never a build break of the tool.
- The riskiest step is the `parse` refactor in `tools/lint/src/parser/mod.rs` (the linter's own parser). Safety net: the existing parser/checker/fixer/app test suites; additionally verify `LINT_FIX=0 ./scripts/lint.sh` output is unchanged (zero import findings) before and after that refactor. If behavior drifts, revert just that file and re-attempt with smaller extractions.
- If a scratch demo file from Validation is left behind, `git status` shows it; `git checkout -- <file>` restores.
- Re-running `./scripts/preflight.sh` is always safe and is the final gate after any recovery.
