# Relocate `authn::issue` inline tests and add `issue_token` integration tests

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, base `main`) | read-write | Source visibility widening, inline-test deletion, integration-test additions. |

All edits live under `/home/ben/miru/workbench2/repos/agent/agent/`.

## Purpose / Big Picture

Two things happen here:

1. **Relocation.** `agent/src/authn/issue.rs` carries an in-source `#[cfg(test)] mod tests` block that tests `mint_jwt` and `encode_part`. Move those tests verbatim to `agent/tests/authn/issue.rs` so they run as integration tests, mirroring `agent/tests/authn/token.rs` and `agent/tests/authn/token_mngr.rs`.
2. **Coverage gap.** `pub async fn issue_token` at `agent/src/authn/issue.rs:30` has no direct tests today; it is exercised only transitively by the token-manager tests. Add focused tests for its happy path, RFC3339 parse failure, HTTP-error bubbling, and missing-key bubbling.

When this plan completes, `./scripts/test.sh` passes, the new file `agent/tests/authn/issue.rs` carries every original test (names, helpers, and assertions byte-identical) plus four new `issue_token` cases, and `scripts/covgate.sh` for `authn` still passes.

## Progress

- [ ] M1: Widen visibility of `mint_jwt` and `encode_part` in `agent/src/authn/issue.rs`.
- [ ] M2: Create `agent/tests/authn/issue.rs` with the relocated tests (verbatim bodies, names, helpers, `AlwaysFails`).
- [ ] M3: Delete the `#[cfg(test)] mod tests { ... }` block from `agent/src/authn/issue.rs`.
- [ ] M4: Register `pub mod issue;` in `agent/tests/authn/mod.rs` (alphabetically before `token`).
- [ ] M5: Add the four new `issue_token` integration tests in the same `agent/tests/authn/issue.rs` file.
- [ ] M6: Validate (`./scripts/test.sh`, `cargo fmt --check`, `cargo clippy -D warnings`, `./scripts/lint.sh`, `scripts/covgate.sh`).
- [ ] Final: preflight reports `clean`.

## Surprises & Discoveries

(Populate during implementation.)

## Decision Log

- Decision: Widen `mint_jwt` from `pub(crate)` to `pub`, and `encode_part` from private to `pub`. Rationale: integration tests live in a separate crate and cannot see `pub(crate)` symbols. The brief explicitly forbids `#[cfg(feature = "test")]` gating for these symbols. Date/Author: 2026-04-28 / orchestrator.
- Decision: Place new `issue_token` tests in the same file as the relocated `mint_jwt` tests. Rationale: one file per source file is the established pattern (`tests/authn/token.rs` mirrors `src/authn/token.rs`). Date/Author: 2026-04-28 / orchestrator.
- Decision: Reuse `MockClient` from `agent/tests/mocks/http_client.rs` (`crate::mocks::http_client::MockClient`) for `issue_token` cases, mirroring `tests/authn/token_mngr.rs`. Its `requests()` + `call_count(Call::IssueDeviceToken)` give one-line assertions for "called exactly once". Date/Author: 2026-04-28 / orchestrator.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Crate: `miru-agent` at `agent/agent/` (manifest `agent/agent/Cargo.toml`).

Source under change:
- `agent/src/authn/issue.rs` — `issue_token` (line 30), `mint_jwt` (line 61), `encode_part` (line 92), inline `mod tests` (lines 102–end) to be removed.
- `agent/src/authn/mod.rs` — exports unchanged.
- `agent/src/authn/errors.rs` — variants `AuthnErr::SerdeErr`, `AuthnErr::TimestampConversionErr`, `AuthnErr::HTTPErr` are the relevant return types; the implementer must verify the variant names.

Tests harness:
- `agent/tests/mod.rs` — unchanged.
- `agent/tests/authn/mod.rs` — currently has `pub mod token;` and `pub mod token_mngr;`; M4 inserts `pub mod issue;` first.
- `agent/tests/mocks/http_client.rs` — `MockClient`, `Call::IssueDeviceToken`, route matcher for `/devices/token`.

Dev-deps:
- `chrono`, `openssl`, `serde`, `serde_json`, `uuid` are already available (regular dependencies are visible to integration tests). No `Cargo.toml` change expected.
- `serial_test`, `tokio`, `reqwest` are present in `[dev-dependencies]`. The new tests use plain `#[tokio::test]` without `#[serial]` (no shared OS resources).

Coverage gate: `agent/src/authn/.covgate`. Same code paths run from outside `crate::`; coverage should be unchanged. Confirm via `scripts/covgate.sh` after M5.

## Step-by-step plan

### M1 — Widen visibility in `agent/src/authn/issue.rs`

- `pub(crate) async fn mint_jwt(` → `pub async fn mint_jwt(`
- `fn encode_part<T: Serialize>(` → `pub fn encode_part<T: Serialize>(`

Do **not** widen any other items. Do **not** add `#[cfg(feature = "test")]`. The structs `JwtHeader` / `JwtPayload` stay private.

### M2 — Create `agent/tests/authn/issue.rs` (relocated tests)

Copy the existing `mod tests` body byte-for-byte (function names, helpers including `AlwaysFails` and `generate_keys`, assertion order, all comments). Replace `use super::*;` and `use crate::filesys::{self, Overwrite};` with the integration-test import preamble:

```rust
// internal crates
use miru_agent::authn::errors::AuthnErr;
use miru_agent::authn::issue::{encode_part, mint_jwt};
use miru_agent::crypt::{base64, rsa};
use miru_agent::filesys::{self, Overwrite};

// external crates
use chrono::Utc;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Verifier;
use serde::ser::Error as _;
use serde::{Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;
```

(The implementer must verify each path against the actual workspace before relying on it; e.g. confirm `miru_agent::crypt::base64` resolves the way `tests/crypt/base64.rs` shows.)

### M3 — Delete inline tests

Remove the entire `#[cfg(test)] mod tests { ... }` block from `agent/src/authn/issue.rs`.

### M4 — Register the new test module

`agent/tests/authn/mod.rs`:

```rust
pub mod issue;
pub mod token;
pub mod token_mngr;
```

### M5 — Add `issue_token` tests

Append to the same `agent/tests/authn/issue.rs` file. Add to the import preamble:

```rust
// standard crates
// (none)

// internal crates
use crate::mocks::http_client::{Call, MockClient};
use miru_agent::authn::Token;
use miru_agent::authn::issue::issue_token;
use miru_agent::http::HTTPErr;
use miru_agent::http::errors::MockErr;
// (already-listed AuthnErr / mint_jwt / encode_part / crypt / filesys imports remain)

// external crates
use backend_api::models::TokenResponse;
use chrono::{Duration, Utc};
// (already-listed external imports remain)
```

Tests (exact variant names verified against `agent/src/authn/errors.rs` at implementation time):

- `issue_token_happy_path_returns_token_and_records_one_call`: mock returns valid `TokenResponse`. Assert returned `Token.token` equals expected, `expires_at` within 1s tolerance, `call_count(Call::IssueDeviceToken) == 1`, and the mock's captured request bearer-token is a 3-part JWT.
- `issue_token_invalid_rfc3339_returns_timestamp_conversion_err`: mock returns `expires_at: "not-a-timestamp"` → `AuthnErr::TimestampConversionErr(_)`.
- `issue_token_bubbles_http_err_from_backend`: mock returns `Err(HTTPErr::MockErr(MockErr { is_network_conn_err: false }))` → `AuthnErr::HTTPErr(_)`.
- `issue_token_bubbles_filesys_err_when_public_key_missing`: gen a real key pair, delete the public key, assert `issue_token` returns `Err(_)` and `call_count(Call::IssueDeviceToken) == 0`.

Constraints:
- Don't duplicate `mint_jwt` coverage.
- Max 3 field-by-field `assert_eq!` per test on the same value (lint rule).
- No new dev-dependencies.
- No `#[serial]` (each test gets its own temp dir).

### M6 — Validate locally

1. `./scripts/test.sh`
2. `cargo fmt -p miru-agent -- --check`
3. `cargo clippy --package miru-agent --all-features -- -D warnings`
4. `./scripts/lint.sh`
5. `scripts/covgate.sh` (authn module)

If the import linter flags ordering, re-order to match the three-block convention.

## Test plan (per added/changed item)

| Item | Test |
|------|------|
| `mint_jwt` visibility | New tests in `agent/tests/authn/issue.rs` invoke `miru_agent::authn::issue::mint_jwt` and compile/run. |
| `encode_part` visibility | `encode_part_maps_serialize_failure_to_serde_err` compiles and passes. |
| In-source `mod tests` removed | `cargo build -p miru-agent` and clippy succeed. |
| Test module registered | `cargo test -p miru-agent --test mod authn::issue` discovers all tests. |
| Happy-path `issue_token` | `issue_token_happy_path_returns_token_and_records_one_call` passes. |
| Bad RFC3339 | `issue_token_invalid_rfc3339_returns_timestamp_conversion_err` passes. |
| Backend HTTP error | `issue_token_bubbles_http_err_from_backend` passes. |
| Missing public key | `issue_token_bubbles_filesys_err_when_public_key_missing` passes. |

## Validation

- `./scripts/test.sh` passes cleanly.
- `cargo fmt -p miru-agent -- --check` passes.
- `cargo clippy --package miru-agent --all-features -- -D warnings` passes.
- `./scripts/lint.sh` passes cleanly (or document any deferred sub-step).
- `scripts/covgate.sh` for `authn` still passes.
- **Preflight must report `clean` before changes are published.**

## Out of scope

- No edits under `/home/ben/miru/workbench2` outside of `repos/agent/`.
- No refactoring of `issue_token`, `mint_jwt`, or `encode_part` bodies.
- No new dev-dependencies in `agent/Cargo.toml`.
- No widening of any visibility beyond `mint_jwt` and `encode_part`.
- No `#[cfg(feature = "test")]` gating on the widened functions.

## Critical Files for Implementation

- `/home/ben/miru/workbench2/repos/agent/agent/src/authn/issue.rs`
- `/home/ben/miru/workbench2/repos/agent/agent/tests/authn/issue.rs` (new)
- `/home/ben/miru/workbench2/repos/agent/agent/tests/authn/mod.rs`
- `/home/ben/miru/workbench2/repos/agent/agent/tests/mocks/http_client.rs`
- `/home/ben/miru/workbench2/repos/agent/agent/src/authn/errors.rs`
