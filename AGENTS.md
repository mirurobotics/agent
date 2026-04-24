# Miru Agent

Agent-specific conventions for AI coding agents. Read this before making changes.

## Key files

- `ARCHITECTURE.md` — system design, codemap, invariants. Read this first.
- `Cargo.toml` — workspace layout, shared dependencies, MSRV.
- `agent/Cargo.toml` — binary package config, feature flags, dev dependencies.
- `agent/src/main.rs` — entry point (provision vs runtime).
- `agent/src/lib.rs` — module listing (all 22 public modules).

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

### Error handling

All error types derive `thiserror::Error` and implement the custom `crate::errors::Error` trait (defined in `agent/src/errors/`). The trait provides default implementations for `code()`, `http_status()`, `params()`, and `is_network_conn_err()`. Aggregating enum errors use the `impl_error!` macro (also in `agent/src/errors/`).

### Feature flags

`#[cfg(feature = "test")]` gates test-only code (mock implementations, state setters). Never use this flag in production code paths.

## Testing

Always use `scripts/test.sh`:

```bash
./scripts/test.sh
# Runs: RUST_LOG=off cargo test --features test
```

The `--features test` flag is required — many test helpers and mocks are behind
`#[cfg(feature = "test")]`. Without it, tests will fail with misleading errors
(missing test helpers).

Tests run in parallel by default. Tests that bind shared OS resources (e.g.,
`/tmp/miru.sock`) are annotated with `#[serial]` from the `serial_test` crate,
which serializes them relative to each other while leaving all other tests
parallel. When adding a test that uses a fixed path or other global state, add
`#[serial]` to that test function.

Test files in `agent/tests/` mirror the `agent/src/` module structure.

### Coverage gates

Each module has a `.covgate` file with a minimum coverage percentage. Run `scripts/covgate.sh` to enforce. When adding or modifying code, verify coverage still passes.

## Linting

Use `scripts/update-deps.sh` to refresh `Cargo.lock` before linting. Then run `scripts/lint.sh` for a full local lint pass. It runs: the custom import linter, `cargo fmt`, unused dependency checks (machete, diet), security audit, and clippy.

In CI, the Lint workflow runs:
- `cargo run --manifest-path tools/lint/Cargo.toml -- --path agent/src --config .lint-imports.toml --assert-paths agent/tests` — runs import linting and field-by-field assert detection (4+ `assert_eq!` on fields of the same variable in a test function). Suppress assert findings with `// lint:allow(field-by-field-assert)` inside the test body.
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
3. Create matching test file at `agent/tests/<module>/mod.rs`.
4. Add a `.covgate` file in the new module directory with the minimum coverage threshold.

### Adding or changing an API endpoint

1. Update the OpenAPI spec in `api/specs/`.
2. Run `api/regen.sh` to regenerate client/server code in `libs/`.
3. Update the agent source to use the new or changed types.
