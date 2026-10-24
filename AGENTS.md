# Miru Agent

Agent-specific conventions for AI coding agents. Read this before making changes.

## Key files

- `ARCHITECTURE.md` — system design, codemap, invariants. Read this first.
- `Cargo.toml` — workspace layout, shared dependencies, MSRV.
- `agent/Cargo.toml` — binary package config, feature flags, dev dependencies.
- `agent/src/main.rs` — entry point (provision vs runtime).
- `agent/src/lib.rs` — module listing (all 23 public modules).

## Review questions

Before a PR leaves draft, ask these of the full diff. Fix each finding, or answer it in the PR description.

1. Do we need this? Would deleting it break a requirement of this PR?
2. Why is this optional? Is it a parameter every caller passes, or a field that always exists?
3. What does the closest sibling do? Match its layout, harness and tests.
4. Is it in the right layer and file (`ARCHITECTURE.md`), or does a shared helper already exist?
5. Does the name say exactly what it does, with one name per concept and no clash with an existing meaning?
6. Is each comment short, and will it stay true?

This copies the review questions from the `miru-internal` plugin's rules because cloud sessions don't load the plugin yet. Delete it once they do.

## Project conventions

### Import ordering

Every source file follows this order, with groups separated by a blank line and a comment:

```rust
// standard crates
use std::sync::Arc;

// internal crates
use crate::app::state::AppState;

// external crates
use tokio::sync::broadcast;
```

### Device API conversions

Handlers in `agent/src/server/handlers.rs` don't convert wire types. Response conversions live in `server/response.rs` and request conversions in `server/request.rs` ([agent#383](https://github.com/mirurobotics/agent/pull/383#discussion_r4363970314)).

### Error handling

All error types derive `thiserror::Error` and implement the custom `crate::errors::Error` trait (defined in `agent/src/errors/`). The trait provides default implementations for `code()`, `http_status()`, `params()`, and `is_network_conn_err()`. Aggregating enum errors use the `impl_error!` macro (also in `agent/src/errors/`).

Don't add a trait method for a failure only one module reacts to. Give the failure its own error variant and match it in that module ([agent#393](https://github.com/mirurobotics/agent/pull/393#discussion_r4368553524)).

### Test visibility

Use ordinary `#[cfg(test)]` for additive test inspection and state setup, with
private or `pub(crate)` visibility. Put reusable fixtures in
`agent/tests/test_utils/`. Production algorithms and client construction must
use the same code in normal and test builds; tests control dependencies and time.

### Enum conventions

Pick the enum facility by what the enum needs to do:

- **Backend-mirrored status enum** (has a `backend_api` and/or `device_api` twin) →
  `impl_status_enum!` (`agent/src/models/status.rs`) with `agent_type` +
  `backend_type` + `unknown_backend`. Generates the wire `Deserialize`
  (unknown→default), `variants()`, `as_str()`, all layer conversions, and inline
  serde + backend forward-compat tests.
- **Local wire/config enum with no backend twin** → `impl_status_enum!` local or
  agent-only form (omit `backend_type`; omit `agent_type` too when there is no
  agent-server twin). Add `display: true`, `aliases: [...]`,
  `case_insensitive: true`, and/or `on_non_string: default` only as needed (e.g.
  `LogLevel`). These clauses default off, so the wire-strict, case-sensitive,
  non-string-rejecting behavior is the baseline.
- **Error enum** (`thiserror::Error` + `crate::errors::Error`) → `impl_error!`;
  never `impl_status_enum!`. Data-carrying / one-way enums like `errors::Code`
  (a `BackendError(String)` variant and a one-way `as_str()`) are excluded.
- **Internal/actor/config enum with no wire contract** → plain derives, no macro.

Each `impl_status_enum!` enum also gets a hand-written `StatusEnumFixture` table
and `status_enum_tests!` in its integration test file
(`agent/tests/models/harnesses.rs`), so a slip in the macro's mapping fails a test.

## Testing

AI agents: don't run the commands in this section or in Linting locally. Push the branch and let CI run them, then fix failures from the CI logs. They are documented here for humans and so you know what CI checks. One-off checks outside the regular suite, such as benchmarks or spikes, may run locally.

Ordinary Cargo tests and the logging-suppressed wrapper are supported:

```bash
cargo test
cargo test --package miru-agent
./scripts/test.sh
# Wrapper runs: RUST_LOG=off cargo test --package miru-agent
```

No custom feature or preconfigured `RUST_LOG` is required. Cargo discovers the
integration suite in `agent/tests/mod.rs` and the separate `logs_init_smoke` and
`logs_init_locked` targets, which isolate logging initialization in their own
processes. Each integration target uses the ordinary library; integration
fixture imports use direct paths such as `crate::test_utils::...`.

Keep unit tests in inline `#[cfg(test)] mod tests` modules at the bottom of the
source files they test, not in separate files or nested test directories.
Shared fixtures stay in `agent/tests/test_utils/`. `agent/src/lib.rs` mounts the
curated unit-test subset `agent/tests/test_utils/unit.rs` as `crate::test_utils`
(see that file's header for the path rules the shared sources must follow); the
library does not mount the integration suite.

Tests run in parallel by default. Tests that bind shared OS resources (e.g.,
`/tmp/miru.sock`) are annotated with `#[serial]` from the `serial_test` crate,
which serializes them relative to each other while leaving all other tests
parallel. When adding a test that uses a fixed path or other global state, add
`#[serial]` to that test function.

Gate a test with `#[cfg(unix)]` only when it asserts Unix-specific semantics
(mode bits, mode-induced permission denial, symlinks). Otherwise use a portable
fixture so the test also runs in the `windows-check` CI job.
`File::new` and `Dir::new` normalize path components, so `/`-rooted literals are
fine for them on every platform; build a raw `PathBuf` fixture that will be
compared or displayed with `test_utils::filesys::abs_path`.

Integration test files in `agent/tests/` mirror the `agent/src/` module structure.

Route tests in `agent/tests/server/handlers.rs` check the API contract only: status codes and response or error bodies. What a service does with a request, such as how a report is stored, belongs in the service tests ([agent#383](https://github.com/mirurobotics/agent/pull/383#discussion_r4364316671)).

### Coverage gates

Each module has a `.covgate` file with a minimum coverage percentage. Run `scripts/covgate.sh` to enforce. When adding or modifying code, verify coverage still passes.

`./scripts/coverage.sh` runs tests and generates HTML.

## Linting

Use `scripts/update-deps.sh` to refresh `Cargo.lock` before linting. Then run `scripts/lint.sh` for a full local lint pass. It runs: the custom import linter, `cargo fmt`, unused dependency checks (machete, diet), security audit, and clippy.

In CI, the Lint workflow runs:
- The custom linter checks imports in `agent/src/` and `agent/tests/`, function length, and field-by-field assertions (4+ `assert_eq!` on fields of the same variable in a test function). Assertion checks cover all of `agent/src/` and `agent/tests/`. Production functions and closures are limited to 50 non-blank, non-comment body lines (test code exempt); suppress with `// lint:allow(funclen)` on the `fn` line or the line immediately above. Suppress assert findings with `// lint:allow(field-by-field-assert)` inside the test body.
- `cargo fmt -p miru-agent -- --check`
- `cargo clippy --package miru-agent --fix --allow-dirty --all-features -- -D warnings`
- `cargo machete`
- `rustsec/audit-check`

## Generated code

`libs/backend-api/` and `libs/device-api/` are auto-generated from OpenAPI specs. Do not edit by hand. Regenerate via `make -C api` or `api/regen.sh`. Clippy warnings in generated code are expected and unrelated to agent source quality.

## Common tasks

### Adding a new module

1. Create `agent/src/<module>/mod.rs` (and `errors.rs` if needed).
2. Add `pub mod <module>;` to `agent/src/lib.rs`.
3. Add public-behavior tests under `agent/tests/<module>/mod.rs` and declare the module in `agent/tests/mod.rs`; place tests needing private access in a local `#[cfg(test)]` module.
4. Add a `.covgate` file in the new module directory with the minimum coverage threshold.

### Adding or changing an API endpoint

1. Update the OpenAPI spec in `api/specs/`.
2. Run `api/regen.sh` to regenerate client/server code in `libs/`.
3. Update the agent source to use the new or changed types.
