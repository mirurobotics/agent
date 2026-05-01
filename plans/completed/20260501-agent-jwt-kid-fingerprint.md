# Send public key fingerprint as `kid` in agent self-signed JWT

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

This plan is **retrospective**: the implementation is already present in the working tree on branch `refactor/agent-jwt-kid-fingerprint` and is about to be committed. The plan documents what was done, why, and how to validate it.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, branch `refactor/agent-jwt-kid-fingerprint`, base `main`) | read-write | Source and tests for the JWT header change. Plan file lives here. |
| `mirurobotics/openapi` | read-only | Companion spec change in PR #47 swaps the `jwk` header for `kid` on the `POST /devices/token` request. Not consumed from this repo. |
| `mirurobotics/core` (`pkg/rsa.Fingerprint`, Go) | read-only | Reference implementation of the same fingerprint algorithm. Cross-language byte-for-byte compatibility is a design requirement. |

This plan lives in `agent/plans/backlog/` because all code changes are in this repo.

## Purpose / Big Picture

After this change, the self-signed RS512 JWT the agent mints for `POST /devices/token` carries a 64-character lowercase-hex SHA-256 fingerprint of its RSA public key in the JWS `kid` header (RFC 7515 §4.1.4) instead of the full public key as a `jwk` (RFC 7517). The wire format shrinks substantially — a 64-char hex string replaces ~344 base64url chars of modulus plus the exponent — and the JWT follows the standard registered-header convention. Future backend code will use the fingerprint as a lookup key for the enrolled device, and verify the token's signature against the public key stored at enrollment time, instead of trusting the key embedded in the header.

A developer can verify the change by running `./scripts/test.sh` from the repo root and observing that the new and updated tests pass. The minted JWT's header decodes to `{ "alg": "RS512", "typ": "JWT", "kid": "<64 hex chars>" }`, and the `kid` value matches `rsa::fingerprint(&public_key)` for the device's public key.

## Progress

- [x] M1: Add `pub fn fingerprint` to `agent/src/crypt/rsa.rs` and a matching `ConvertPublicKeyToDERErr` variant in `agent/src/crypt/errors.rs`. Remove the now-dead `Jwk` struct and `pub_key_to_jwk` function.
- [x] M2: Update `agent/src/authn/issue.rs::JwtHeader` to use `kid: String` instead of `jwk: rsa::Jwk`, and have `mint_jwt` compute the fingerprint and emit it in the header.
- [x] M3: Replace the `pub_key_to_jwk` test module in `agent/tests/crypt/rsa.rs` with a `fingerprint` test module exercising determinism and uniqueness across distinct keys.
- [x] M4: Rename and rewrite the JWT-header test in `agent/tests/authn/issue.rs` to assert `kid` rather than `jwk`, including a check that the value equals `rsa::fingerprint(&public_key)` and is 64 chars.
- [ ] M5: Run preflight (`./scripts/preflight.sh`) from the repo root and iterate until it reports `Preflight clean`. Commit the resulting working-tree changes as a single milestone commit.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- 2026-05-01 — Authoring this plan retrospectively after the implementation was already done in the working tree. The plan describes the change accurately so the implement subagent can verify and commit.
- 2026-05-01 — The plan as briefed referenced a pinned cross-language vector test (`matches_known_pem_vector` against `agent/tests/testdata/crypt/public_key.pem`, expected hex `633412a0909835879d0d199be10c8da5675ce0cb3e96a954830cff7eed1fb899`). That test and the testdata file are **not** present in the working tree. This plan documents only what is actually there. See the "Cross-language compatibility" subsection of Validation for the practical implication (compatibility is asserted by spec, not enforced by an in-repo pinned vector).

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Repo conventions (from `agent/AGENTS.md`):

- Tests: `./scripts/test.sh` (runs `RUST_LOG=off cargo test --features test`). The `test` feature is required.
- Lint: `./scripts/lint.sh` (custom import linter, `cargo fmt`, machete, audit, clippy with `-D warnings`).
- Preflight: `./scripts/preflight.sh` runs lint, tests (via `covgate.sh`), tools-lint, and tools-tests in parallel; success prints `Preflight clean`.
- Coverage gate: `./scripts/covgate.sh` enforces per-module thresholds in `.covgate` files. Relevant gates here are `agent/src/authn/.covgate` and `agent/src/crypt/.covgate`; do not regress them.
- Generated code under `libs/backend-api/` and `libs/device-api/` is regenerated via `api/regen.sh`; do not hand-edit.
- Source files use a fixed import order: standard crates, internal crates, external crates — separated by blank lines and group comments.
- Errors: each error type derives `thiserror::Error` and implements `crate::errors::Error`; aggregating enums use the `impl_error!` macro. New variants go in the module's `errors.rs`.

Key files (full paths from repo root `/home/ben/miru/workbench2/repos/agent/`):

- `agent/src/crypt/rsa.rs` — RSA key utilities: `gen_key_pair`, `read_public_key`, `read_private_key`, `sign`, `sign_rs512`, `verify`. Pre-change it also exported `Jwk` and `pub_key_to_jwk`. Post-change it exports `fingerprint`.
- `agent/src/crypt/errors.rs` — `CryptErr` enum and per-variant structs. Pre-existing variants follow the pattern `#[derive(Debug, thiserror::Error)] #[error("…: {source}")] pub struct FooErr { pub source: openssl::error::ErrorStack, pub trace: Box<Trace> }`. The new `ConvertPublicKeyToDERErr` mirrors `ConvertPublicKeyToPEMErr` exactly.
- `agent/src/authn/issue.rs` — `mint_jwt(private_key_file, public_key_file)` builds the self-signed RS512 JWT. The `JwtHeader` struct's third field is the only field that changes. The payload (`jti`, `iat`, `exp` two minutes in the future), the signing helper, and the assembled `header.payload.signature` shape are all unchanged.
- `agent/src/crypt/base64.rs` — already provides `encode_bytes_url_safe_no_pad` used elsewhere in JWT assembly. Unchanged.
- `agent/tests/crypt/rsa.rs` — integration tests for the `crypt::rsa` module. Each `pub mod foo` block tests one function.
- `agent/tests/authn/issue.rs` — integration tests for `authn::issue::mint_jwt` and `issue_token`.

Definitions:

- **Self-signed JWT**: a JWT whose signature is verifiable only with a key the holder controls. Used here as a proof-of-possession at the moment the agent first calls `POST /devices/token` to obtain a backend-issued bearer token.
- **`kid` (Key ID)**: standard JWS header parameter (RFC 7515 §4.1.4). A short opaque hint the verifier uses to pick which key to verify with. Here it is the public-key fingerprint.
- **Fingerprint**: lowercase hex SHA-256 of the DER-encoded SubjectPublicKeyInfo. Always 64 characters. See "Plan of Work / Algorithm specification" below.
- **DER PKIX SubjectPublicKeyInfo (SPKI)**: the standard ASN.1/DER encoding of an RSA public key as produced by OpenSSL's `public_key_to_der`, equivalent to PEM `-----BEGIN PUBLIC KEY-----` blocks decoded from base64. This is the input to the hash and is what makes the fingerprint stable across languages and tooling.

## Plan of Work

### Algorithm specification

The fingerprint is defined by three steps, in order:

1. Serialize the RSA public key to its DER PKIX SubjectPublicKeyInfo encoding. In Rust/openssl: `key.public_key_to_der()`. In Go/`crypto/x509`: `x509.MarshalPKIXPublicKey(key)`.
2. Hash the DER bytes with SHA-256 (32-byte digest).
3. Encode the digest as lowercase hex; the result is exactly 64 ASCII characters from the alphabet `[0-9a-f]`.

The same algorithm is implemented in `mirurobotics/core` `pkg/rsa.Fingerprint` (Go). The two implementations produce byte-identical output for any given RSA public key, because both feed the SPKI DER bytes into SHA-256 and lowercase-hex-encode the result. This cross-language equivalence is a design requirement; the agent and any backend consumer must agree on the fingerprint of a single key.

Forward-looking note: the agent OpenAPI spec change is in https://github.com/mirurobotics/openapi/pull/47. The backend that will consume this JWT does not exist yet, so there is no live wire-format break to coordinate. When the backend is implemented, it should pin `alg=RS512`, treat `kid` as a lookup hint only (never as a key source), and reject any unknown `kid` with a generic `401`.

### Edits

**`agent/src/crypt/errors.rs`** — add a new error variant for DER serialization failures, mirroring `ConvertPublicKeyToPEMErr`:

- New struct `ConvertPublicKeyToDERErr { source: openssl::error::ErrorStack, trace: Box<Trace> }` with the standard `thiserror::Error` derive, `#[error("Convert public key to DER error: {source}")]`, and `impl crate::errors::Error`.
- Add `ConvertPublicKeyToDERErr(ConvertPublicKeyToDERErr)` to the `CryptErr` enum and to the `impl_error!(CryptErr { ... })` list (alphabetically with the other `ConvertPublicKey*` entries).

**`agent/src/crypt/rsa.rs`** — replace JWK handling with fingerprinting:

- Remove the `use serde::Serialize;` import (no longer needed).
- Remove the `pub struct Jwk { kty, n, e }` and `pub fn pub_key_to_jwk(...)` items in their entirety.
- Add `use openssl::sha::sha256;` to the external-crates import group.
- Add a new public function:

      /// Canonical fingerprint of an RSA public key: lowercase hex SHA-256 over the
      /// DER-encoded SubjectPublicKeyInfo
      pub fn fingerprint(key: &Rsa<Public>) -> Result<String, CryptErr> {
          let der = ssl_err!(ConvertPublicKeyToDERErr, key.public_key_to_der())?;
          let digest = sha256(&der);
          let mut out = String::with_capacity(digest.len() * 2);
          for b in digest {
              use std::fmt::Write;
              let _ = write!(out, "{b:02x}");
          }
          Ok(out)
      }

- The `ssl_err!` macro is already defined in this file; it maps an `openssl::error::ErrorStack` failure to the named `CryptErr` variant.

**`agent/src/authn/issue.rs`** — switch the header field and update the docstring:

- Change `JwtHeader`'s third field from `jwk: rsa::Jwk` to `kid: String`. Field order remains `alg`, `typ`, then the key-related field; serde preserves declaration order.
- In `mint_jwt`, replace the `let jwk = rsa::pub_key_to_jwk(&public_key);` line with `let kid = rsa::fingerprint(&public_key)?;`. Note the `?` — `fingerprint` is fallible, while `pub_key_to_jwk` was infallible. `AuthnErr` already aggregates `CryptErr` via `impl_error!`, so the conversion compiles without changes to `agent/src/authn/errors.rs`.
- Update the docstring on `mint_jwt` to describe the new behavior: the header carries the device's public-key fingerprint as `kid` (RFC 7515 §4.1.4); the backend looks up the enrolled device by this fingerprint and verifies the signature with the stored public key. The payload claims (`jti`, `iat`, `exp` two minutes in the future) are unchanged.

**`agent/tests/crypt/rsa.rs`** — replace the `pub_key_to_jwk` test module with a `fingerprint` module:

- Rename `pub mod pub_key_to_jwk { ... }` to `pub mod fingerprint { ... }`. Update the inner `use miru_agent::crypt::rsa::pub_key_to_jwk;` to `use miru_agent::crypt::rsa::fingerprint;`.
- Replace the existing two tests with the following two:
  - `success_deterministic_for_known_key`: generate a 2048-bit key pair to a temp dir, read the public key, call `fingerprint` twice on it, assert the two strings are equal, assert the length is `64`, and assert all characters are ASCII hex digits and not uppercase. Async (`#[tokio::test]`) because `rsa::gen_key_pair` and `rsa::read_public_key` are async.
  - `differs_across_keys`: generate two distinct 2048-bit key pairs (`priv1.pem`/`pub1.pem` and `priv2.pem`/`pub2.pem`) into the same temp dir, fingerprint each public key, and `assert_ne!` the results.

**`agent/tests/authn/issue.rs`** — rename and rewrite the header-shape test in `mod mint_jwt`:

- Rename `header_decodes_to_rs512_with_jwk` to `header_decodes_to_rs512_with_kid`.
- Keep the prelude (generate keys, mint JWT, split on `.`, base64url-decode header segment, parse JSON).
- Keep the `assert_eq!("RS512", header["alg"])` and `assert_eq!("JWT", header["typ"])` assertions.
- Replace the `kty`/`n`/`e` JWK assertions with: read the public key via `rsa::read_public_key`, compute `expected_kid = rsa::fingerprint(&public_key).unwrap()`, then `assert_eq!(header["kid"].as_str().unwrap(), expected_kid)` and `assert_eq!(kid.len(), 64)`.
- Other tests in the file (signature verification, payload claims, lifetime) remain untouched because none of them inspect the header's third field.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent/` unless stated otherwise.

1. Confirm branch:

       git rev-parse --abbrev-ref HEAD

   Expected: `refactor/agent-jwt-kid-fingerprint`.

2. Confirm working-tree changes match the plan:

       git diff --stat main

   Expected: five modified files — `agent/src/authn/issue.rs`, `agent/src/crypt/errors.rs`, `agent/src/crypt/rsa.rs`, `agent/tests/authn/issue.rs`, `agent/tests/crypt/rsa.rs`. No new or deleted files.

3. Run preflight; iterate until clean:

       ./scripts/preflight.sh

   Expected final line: `Preflight clean`. If lint or coverage fails, fix the cause (do not lower coverage thresholds) and rerun.

4. Commit the milestone in a single commit:

       git add agent/src/authn/issue.rs agent/src/crypt/errors.rs agent/src/crypt/rsa.rs \
               agent/tests/authn/issue.rs agent/tests/crypt/rsa.rs
       git commit

   Suggested commit message subject: `refactor(authn): send public key fingerprint as kid in self-signed JWT`. Body should mention the wire-size reduction, RFC 7515 §4.1.4 alignment, and the cross-language fingerprint equivalence with `mirurobotics/core` `pkg/rsa.Fingerprint`.

5. Confirm the commit landed:

       git log --oneline -1

## Validation and Acceptance

### Tests

From the repo root:

    ./scripts/test.sh

Expected: all tests pass. The four tests of interest are:

- `crypt::rsa::fingerprint::success_deterministic_for_known_key` — passes after the change; failed to compile before because `fingerprint` did not exist.
- `crypt::rsa::fingerprint::differs_across_keys` — passes after the change.
- `authn::issue::mint_jwt::header_decodes_to_rs512_with_kid` — passes after the change; the pre-change test (`header_decodes_to_rs512_with_jwk`) is removed.
- All other pre-existing tests in `mod mint_jwt` (signature verifies, payload `jti` is a UUID, `exp` is two minutes after `iat`, etc.) continue to pass — none of them inspect the header's key-related field.

### Coverage

    ./scripts/covgate.sh

Expected: `agent/src/authn/.covgate` and `agent/src/crypt/.covgate` thresholds remain met. The `fingerprint` function is exercised by both new `crypt` tests and indirectly by the updated `mint_jwt` header test, so coverage of the new code is high. No `.covgate` threshold should be lowered.

### Lint

    ./scripts/lint.sh

Expected: passes. The custom import linter is satisfied because the only new external import (`openssl::sha::sha256`) is added to the existing external-crates group; no new crates are introduced. `cargo machete` does not flag a removal because `serde` is still used elsewhere in the agent. `cargo audit` is unaffected because no `Cargo.toml` changed.

### Acceptance

The change is accepted when all three of these are true:

1. `./scripts/preflight.sh` from the repo root prints `Preflight clean`. This must be true before the change is published — preflight clean is a hard prerequisite for opening a PR.
2. The minted JWT's header, when its first segment is base64url-decoded and parsed as JSON, has exactly the keys `alg`, `typ`, `kid`, with `alg == "RS512"`, `typ == "JWT"`, and `kid` matching `rsa::fingerprint(public_key)` for the device's public key (64 lowercase hex chars).
3. The fingerprint function, given a public key whose DER PKIX SPKI bytes are `D`, returns the lowercase hex of `SHA-256(D)`. This is the same value that `mirurobotics/core` `pkg/rsa.Fingerprint(key)` returns for the same key.

### Cross-language compatibility

Cross-language equivalence with `mirurobotics/core` `pkg/rsa.Fingerprint` is asserted by specification (both implementations apply DER PKIX SPKI → SHA-256 → lowercase hex), but is **not** enforced inside this repo by a pinned-vector test against a checked-in PEM. If a future change wants to harden this — e.g. drop a `agent/tests/testdata/crypt/public_key.pem` and assert `fingerprint` against a known hex digest — it can be added as a follow-up. For now, the determinism and uniqueness tests cover everything observable from inside the agent crate.

## Out of Scope / Non-goals

- Provisioning still posts the full PEM-encoded public key in `public_key_pem`. That is a different endpoint and a different code path; nothing about provisioning changes here.
- RS512 signing semantics are unchanged: same private key, same digest, same `Signer` flow in `agent/src/crypt/rsa.rs::sign_rs512`.
- The JWT payload (`jti`, `iat`, `exp`) and the two-minute lifetime are unchanged.
- The HTTP endpoint shape (`POST /devices/token`, no body, bearer-style header) is unchanged.
- The `AuthnErr` enum gains no new variants — the new `CryptErr::ConvertPublicKeyToDERErr` is converted via the existing `From<CryptErr> for AuthnErr` plumbing produced by `impl_error!`.

## Idempotence and Recovery

- All steps above are idempotent. `git diff --stat main` and `./scripts/preflight.sh` are read-only and can be rerun. `git add` followed by `git commit` is safe; if the commit fails (e.g. a pre-commit hook), fix the cause and re-run.
- Recovery from a bad commit: `git reset --soft HEAD~1` brings the changes back to the index without losing work; `git reset --hard HEAD~1` is destructive and should not be used unless the user explicitly requests it.
- If preflight reveals a regression that was masked by uncommitted state (e.g. a coverage drop), fix the source — do not lower a `.covgate` threshold to make the gate pass.
