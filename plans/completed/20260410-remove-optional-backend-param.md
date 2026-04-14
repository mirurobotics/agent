# Remove Optional Backend Parameter from Service `get` Functions

**Status**: active
**Branch**: refactor/services-backend-fetcher (PR #25)

## Goal

Make the `backend` parameter required (`&B`) instead of `Option<&B>` in the
three service `get` functions. In production, `Some(&backend)` is always
passed — `None` only appears in tests, where stub backends (`PanicBackend`)
already exist for the same purpose.

## Scope

### Source changes (3 files)
- `agent/src/services/deployment/get.rs` — `Option<&B>` → `&B`, remove `let Some(backend) = backend else { ... };`
- `agent/src/services/release/get.rs` — same
- `agent/src/services/git_commit/get.rs` — same

### Handler changes (1 file)
- `agent/src/server/handlers.rs` — `Some(&backend)` → `&backend` at 3 call sites

### Test changes (3 files)
- `agent/tests/services/deployment/get.rs`
- `agent/tests/services/release/get.rs`
- `agent/tests/services/git_commit/get.rs`

At each:
- `None::<&PanicBackend>` → `&PanicBackend` (cache-hit tests only)
- `Some(&PanicBackend)` → `&PanicBackend`
- `Some(&stub)` → `&stub`
- Delete `not_found_returns_error` — tested cache miss with `None`; redundant with `cache_miss_backend_404_returns_not_found`
- Delete `cache_miss_no_backend_returns_not_found` — code path no longer exists

## Test steps

- `cargo check` — compilation
- `cargo clippy` — lint
- `cargo test -p miru-agent --test mod -- services::deployment` — deployment tests
- `cargo test -p miru-agent --test mod -- services::release` — release tests
- `cargo test -p miru-agent --test mod -- services::git_commit` — git_commit tests

## Validation

Preflight must report `clean` before pushing.
