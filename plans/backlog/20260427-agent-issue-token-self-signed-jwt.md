# Refactor agent issue_token flow to v0.3.0-beta.3 self-signed JWT auth

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, branch `feat/idempotent-upgrade-reset`, base `main`) | read-write | Source, tests, and plan file. |
| `libs/backend-api/` (inside this repo, generated) | read-only | Confirm regenerated models are present; do not edit. |

This plan lives in `plans/backlog/` of the agent repo because all code changes are in this repo.

## Purpose / Big Picture

After this change the agent talks to backend `v0.3.0-beta.3` correctly. Today the build is broken: `agent/src/authn/issue.rs` and `agent/src/http/devices.rs` import `backend_api::models::{IssueDeviceClaims, IssueDeviceTokenRequest}`, which were deleted from `libs/backend-api/` in commit `2e1e256 chore(api): regenerate backend-api for v0.3.0-beta.3`. The new contract requires the agent to authenticate to `POST /devices/issue_token` with a self-signed RS512 JWT carrying its public key as a JWK in the header (RFC 7517). The server identifies the device by the SHA-256 fingerprint of that key and returns a `TokenResponse`. After this change a developer can run `./scripts/test.sh` from the repo root and see all tests pass, including new tests that verify the JWK serializer is deterministic and the issue-token JWT is a valid 3-segment RS512 JWT whose signature verifies against the device's public key.

## Progress

- [ ] M1: Add JWK serializer + RS512 sign helper in `crypt`.
- [ ] M2: Rewrite `authn::issue::prepare_issue_token_request` and `issue_token` to build a self-signed JWT and plumb the public key path.
- [ ] M3: Update `http::devices::issue_token` + `IssueTokenParams` to the new URL/auth/no-body shape.
- [ ] M4: Update callers in `app::upgrade::reconcile`, `authn::token_mngr`, `app::state::AppState::init`, and tests in `tests/authn/token_mngr.rs` and `tests/sync/syncer.rs` to plumb the public key file.
- [ ] M5: Update `tests/http/devices.rs` issue_token cases and verify `tests/mocks/http_client.rs` route match still works.
- [ ] M6: Run preflight; iterate until it reports `Preflight clean`.

Add timestamps when you complete steps.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

(Add entries as you go.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Repo conventions (from `agent/AGENTS.md`):

- Tests: `./scripts/test.sh` (runs `RUST_LOG=off cargo test --features test`). The `test` feature is required.
- Lint: `./scripts/lint.sh` (custom import linter, `cargo fmt`, machete, audit, clippy with `-D warnings`).
- Preflight: `./scripts/preflight.sh` runs lint + tests + tools-lint + tools-tests in parallel; success prints `Preflight clean`.
- Coverage gate: `./scripts/covgate.sh` enforces per-module thresholds in `.covgate` files. Relevant gates:
  - `agent/src/authn/.covgate` = `95.31`
  - `agent/src/crypt/.covgate` = `94.98`
  - `agent/src/http/.covgate` exists; do not regress.
- Generated code under `libs/backend-api/` is regenerated via `api/regen.sh`; do not hand-edit.
- Source files use a fixed import order: `// standard crates`, `// internal crates`, `// external crates`, separated by blank lines.
- Errors: each error type derives `thiserror::Error` and implements `crate::errors::Error`; aggregating enums use the `impl_error!` macro. New errors go in the module's `errors.rs`.

Key files (full paths from repo root `/home/ben/miru/workbench2/repos/agent/`):

- `agent/src/authn/issue.rs` — `prepare_issue_token_request` and `issue_token`. Currently signs an `IssueTokenClaim` with `rsa::sign` (SHA-256) and posts an `IssueDeviceTokenRequest` body.
- `agent/src/authn/mod.rs` — re-exports `issue::issue_token`.
- `agent/src/authn/token_mngr.rs` — `SingleThreadTokenManager` and `TokenManager::spawn`. Stores `private_key_file`. Calls `issue::issue_token` at line 76 with `(http_client, &private_key_file, &device_id)`.
- `agent/src/authn/errors.rs` — `AuthnErr` enum with `CryptErr`, `FileSysErr`, `HTTPErr`, `SerdeErr`, `TimestampConversionErr`, `MockError`, etc. Add no new variants — JWT building uses existing `SerdeErr` + `CryptErr` paths.
- `agent/src/crypt/rsa.rs` — `gen_key_pair`, `read_private_key`, `read_public_key`, `sign` (SHA-256), `verify` (SHA-256). The only non-test caller of `sign` is `authn::issue::prepare_issue_token_request:87` (verified via grep `rsa::sign`).
- `agent/src/crypt/jwt.rs` — JWT *decoder* for backend-issued tokens. There is no JWT *builder* yet. The new builder lives in a new file `agent/src/crypt/jwt_builder.rs` (or extends `jwt.rs`; see Decision Log entry M1).
- `agent/src/crypt/base64.rs` — has `encode_bytes_url_safe_no_pad`, `encode_string_url_safe_no_pad`, plus standard variants. Use the URL-safe-no-pad variant for JWT encoding.
- `agent/src/crypt/errors.rs` — `CryptErr` with `Base64DecodeErr`, `ConvertBytesToStringErr`, `ReadKeyErr`, `RSAToPKeyErr`, `SignDataErr`, etc.
- `agent/src/crypt/mod.rs` — `pub mod base64; pub mod errors; pub mod jwt; pub mod rsa;`.
- `agent/src/http/devices.rs` — `IssueTokenParams { id, payload }` and `pub async fn issue_token` posting to `/devices/{id}/issue_token` with the request body.
- `agent/src/http/request.rs` — `Params::post(url, body: String)` (body is `Option<String>` internally; see line 71). `with_token(token)` sets the bearer. There is no no-body POST helper today; passing an empty `String::new()` is fine — `reqwest` will accept it and the body will be the empty string.
- `agent/src/app/upgrade.rs::reconcile` — calls `authn::issue_token(http_client, &private_key_file, &device_id)` at lines 58 and 75. `private_key_file` comes from `layout.auth().private_key()`.
- `agent/src/app/state.rs::AppState::init` — at lines 36, 51-57 spawns the `TokenManager` with `private_key_file`. Will need `public_key_file` passed in too.
- `agent/src/storage/layout.rs` — `AuthLayout::private_key()` returns `auth/private_key.pem`; `AuthLayout::public_key()` returns `auth/public_key.pem`.
- `agent/tests/http/devices.rs` — `issue_token::success` and `issue_token::error_propagates`. They construct an `IssueDeviceTokenRequest::default()` and assert path `/devices/dvc_1/issue_token`.
- `agent/tests/mocks/http_client.rs` — `MockClient::match_route` matches `(POST, p) if p.ends_with("/issue_token")` to `Call::IssueDeviceToken`. The new path `/devices/issue_token` still ends with `/issue_token`, so the existing match works without code change. Confirm in M5 by running tests.
- `agent/tests/authn/token_mngr.rs` — `setup_with_rsa` already generates both `private_key.pem` and `public_key.pem`; `setup` only writes a dummy private key. The fakery `setup` flow needs a parallel dummy public key once the function signatures plumb a `public_key_file` through.
- `agent/tests/sync/syncer.rs:44` — also calls `TokenManager::spawn`; needs the public key file plumbed in.

Definitions:

- **JWT (JSON Web Token, RFC 7519)**: `base64url(header).base64url(payload).base64url(signature)` with no padding. Three segments separated by `.`.
- **JWK (JSON Web Key, RFC 7517)**: JSON representation of a public key. For RSA: `{"kty":"RSA","n":<modulus base64url-no-pad>,"e":<public exponent base64url-no-pad>}`. Field order is not significant for JWK fingerprinting unless using RFC 7638 thumbprints; for our wire format the server just parses the JSON.
- **RS512 (RFC 7518 §3.3)**: RSASSA-PKCS1-v1_5 with SHA-512. In OpenSSL: `Signer::new(MessageDigest::sha512(), &private_key)`.
- **base64url no-pad (RFC 4648 §5)**: URL-safe base64 alphabet (`-`, `_`) with trailing `=` stripped.

## Plan of Work

### Milestone 1 — JWK serializer and RS512 sign helper in `crypt`

Add a JWK helper exposed as `pub fn rsa_public_key_to_jwk(key: &Rsa<Public>) -> Result<serde_json::Value, CryptErr>` (or returning a typed `Jwk` struct that derives `Serialize`; the typed struct is preferred because it documents the field shape and is testable for deterministic output). It takes an already-parsed `Rsa<Public>` so callers can decide how to read the key. Place it in `agent/src/crypt/rsa.rs` next to the other RSA helpers. Implementation: `key.n().to_vec()` and `key.e().to_vec()` return big-endian `Vec<u8>`; encode each with `crate::crypt::base64::encode_bytes_url_safe_no_pad`; serialize as `{"kty":"RSA","n":<n>,"e":<e>}` using a `#[derive(Serialize)] struct Jwk { kty: &'static str, n: String, e: String }`.

Add an RS512 sign helper. The current `rsa::sign` uses SHA-256 and is called only by `authn::issue::prepare_issue_token_request` (non-test) — but that is the path being rewritten, so the digest can change without affecting other production code. Decision: add a sibling `pub async fn sign_rs512(private_key_file: &filesys::File, data: &[u8]) -> Result<Vec<u8>, CryptErr>` that mirrors `sign` but uses `MessageDigest::sha512()`. Keep `sign` (SHA-256) for backward compatibility with its existing test surface, since that surface contributes to the `crypt/.covgate` floor. Record this decision in the Decision Log.

### Milestone 2 — Rewrite `authn::issue`

Rewrite `prepare_issue_token_request` and `issue_token` in `agent/src/authn/issue.rs`:

- New signature: `pub async fn issue_token<HTTPClientT: http::ClientI>(http_client: &HTTPClientT, private_key_file: &File, public_key_file: &File) -> Result<Token, AuthnErr>`. Drop the `device_id` parameter.
- New private builder: `async fn build_self_signed_jwt(private_key_file: &File, public_key_file: &File) -> Result<String, AuthnErr>`.
- The builder:
  1. `let rsa_pub = crypt::rsa::read_public_key(public_key_file).await?;`
  2. Build `Jwk` via `crypt::rsa::rsa_public_key_to_jwk(&rsa_pub)?`.
  3. Build header `{"alg":"RS512","typ":"JWT","jwk":<jwk>}` as a typed struct with `Serialize`.
  4. Build payload `{"jti":<uuid v4 string>,"iat":<now unix ts>,"exp":<now+120 unix ts>}` as a typed struct.
  5. JSON-serialize header and payload via `serde_json::to_vec` (mapped to `AuthnErr::SerdeErr`).
  6. Build signing input: `format!("{}.{}", base64::encode_bytes_url_safe_no_pad(&header_bytes), base64::encode_bytes_url_safe_no_pad(&payload_bytes))`.
  7. Sign with `crypt::rsa::sign_rs512(private_key_file, signing_input.as_bytes()).await?`.
  8. Return `format!("{signing_input}.{}", base64::encode_bytes_url_safe_no_pad(&sig))`.
- Replace `IssueTokenClaim`, `IssueDeviceClaims`, `IssueDeviceTokenRequest` usage. Delete the `backend_api::models::{IssueDeviceClaims, IssueDeviceTokenRequest}` import.
- Call `devices::issue_token` with the new params shape (see M3).

### Milestone 3 — `http::devices::issue_token` + `IssueTokenParams`

In `agent/src/http/devices.rs`:

- Drop `IssueDeviceTokenRequest` from the `backend_api::models` import.
- Replace `IssueTokenParams` with `pub struct IssueTokenParams<'a> { pub token: &'a str }`.
- Rewrite `issue_token`:

      pub async fn issue_token(
          client: &impl ClientI,
          params: IssueTokenParams<'_>,
      ) -> Result<TokenResponse, HTTPErr> {
          let url = format!("{}/devices/issue_token", client.base_url());
          let request = request::Params::post(&url, String::new()).with_token(params.token);
          super::client::fetch(client, request).await
      }

  Pass `String::new()` as the body — `Params::post` requires a `String`, and an empty body serialises as zero bytes, which the backend tolerates for this endpoint per the spec change. Document this in the Decision Log.

### Milestone 4 — Plumb public key path through callers

`agent/src/authn/token_mngr.rs`:

- Add `public_key_file: File` to `SingleThreadTokenManager` and the constructor; assert it exists in `new`.
- Add `public_key_file: File` to `TokenManager::spawn` (between `token_file` and current trailing args). Forward it through `Worker`/`SingleThreadTokenManager`.
- Update `issue_token` (line 75) to pass `&self.private_key_file, &self.public_key_file` to `issue::issue_token`.

`agent/src/app/state.rs::AppState::init`:

- After `let private_key_file = auth_dir.private_key();` add `let public_key_file = auth_dir.public_key();`. Assert it exists.
- Pass `public_key_file` through to `TokenManager::spawn`.

`agent/src/app/upgrade.rs::reconcile` (lines 58, 75):

- Add `let public_key_file = auth_dir.public_key();` next to the existing `private_key_file`. Assert it exists.
- Update both `authn::issue_token` calls to pass `&private_key_file, &public_key_file` and remove `&device_id`.
- Note: `device_id` is still needed for the PATCH at line 76 (`UpdateParams { id: &device_id, ... }`); keep it.

`agent/tests/authn/token_mngr.rs`:

- In `setup`, add a dummy `public_key_file` (write the literal string `"public_key"` to `dir.file("public_key.pem")` with `WriteOptions::default()`) and pass it into `TokenManager::spawn`.
- In `setup_with_rsa`, the `public_key.pem` is already generated; pass `public_key_file` into `spawn`.
- In the two `spawn` failure cases (token-file-missing, private-key-missing), supply a valid public-key file so the failure mode under test is the only failing precondition. Add a third case `public_key_file_does_not_exist` that asserts `AuthnErr::FileSysErr` when only the public key is missing.

`agent/tests/sync/syncer.rs:44`:

- Read this call site fully and pass an additional public-key file matching how the test creates its private key (mirror the existing `setup`/`setup_with_rsa` pattern from `tests/authn/token_mngr.rs`).

### Milestone 5 — Update `tests/http/devices.rs` issue_token cases + verify mock route

In `agent/tests/http/devices.rs`:

- Drop `IssueDeviceTokenRequest` from the import block at lines 3-6.
- Rewrite `issue_token::success`:

      let result = devices::issue_token(
          &mock,
          IssueTokenParams { token: "test-token" },
      )
      .await
      .unwrap();
      assert_eq!(result, TokenResponse::default());
      assert_eq!(mock.call_count(Call::IssueDeviceToken), 1);
      assert_eq!(
          mock.requests(),
          vec![CapturedRequest {
              call: Call::IssueDeviceToken,
              method: reqwest::Method::POST,
              path: "/devices/issue_token".into(),
              url: "http://mock/devices/issue_token".into(),
              query: vec![],
              body: Some(String::new()),
              token: Some("test-token".into()),
          }]
      );

- Rewrite `issue_token::error_propagates` similarly: drop the `IssueDeviceTokenRequest::default()`, build `IssueTokenParams { token: "test-token" }`, expect `Err(HTTPErr::MockErr(_))`.
- Confirm `agent/tests/mocks/http_client.rs::match_route` line 196 (`p.ends_with("/issue_token")`) still maps the new path to `Call::IssueDeviceToken`. No code change there is expected; the test in this milestone is the verification.

### Milestone 6 — Preflight

Run `./scripts/preflight.sh` and iterate until it prints `Preflight clean`.

## Concrete Steps

All commands are run from the repo root unless stated otherwise: `cd /home/ben/miru/workbench2/repos/agent`.

### M1: JWK serializer + RS512 sign helper

1. Edit `agent/src/crypt/rsa.rs`:
   - Add `use serde::Serialize;` to the external-crates group.
   - Add struct + helper:

         #[derive(Serialize, Debug, PartialEq, Eq)]
         pub struct Jwk {
             pub kty: &'static str,
             pub n: String,
             pub e: String,
         }

         pub fn rsa_public_key_to_jwk(key: &Rsa<Public>) -> Jwk {
             Jwk {
                 kty: "RSA",
                 n: crate::crypt::base64::encode_bytes_url_safe_no_pad(&key.n().to_vec()),
                 e: crate::crypt::base64::encode_bytes_url_safe_no_pad(&key.e().to_vec()),
             }
         }

   - Add `sign_rs512` next to `sign`:

         pub async fn sign_rs512(
             private_key_file: &filesys::File,
             data: &[u8],
         ) -> Result<Vec<u8>, CryptErr> {
             let rsa_private_key = read_private_key(private_key_file).await?;
             let private_key = ssl_err!(RSAToPKeyErr, PKey::from_rsa(rsa_private_key))?;
             let mut signer = ssl_err!(
                 SignDataErr,
                 Signer::new(MessageDigest::sha512(), &private_key)
             )?;
             ssl_err!(SignDataErr, signer.update(data))?;
             let signature = ssl_err!(SignDataErr, signer.sign_to_vec())?;
             Ok(signature)
         }

2. Add tests in `agent/tests/crypt/rsa.rs` (append new modules at the end, mirroring style):

   - `pub mod rsa_public_key_to_jwk` with:
     - `success_deterministic_for_known_key` — generate a key pair with `gen_key_pair(2048, ...)`, read the public key, call `rsa_public_key_to_jwk` twice, assert both JWK structs are equal (deterministic) and that `kty == "RSA"`, `n` and `e` are non-empty, only `[A-Za-z0-9_-]` characters (URL-safe-no-pad).
     - `serialized_json_field_order` — `serde_json::to_string(&jwk)` contains `"kty":"RSA"`, `"n":` and `"e":` keys.
   - `pub mod sign_rs512` with:
     - `success` — generate key pair, sign data, assert signature is non-empty and `len > 200` (RS512 with 2048-bit key produces 256-byte signature).
     - `verifies_with_sha512` — sign data, then verify by reading the public key and using `openssl::sign::Verifier::new(MessageDigest::sha512(), ...)` directly to assert true; verifying with SHA-256 (i.e. via the existing `rsa::verify`) should yield false. The latter is sentinel that the digest is genuinely SHA-512.
     - `missing_file` — passes a nonexistent private-key file, asserts `Err`.

3. Run the focused test pass:

       ./scripts/test.sh -- --test crypt

   Expect `test result: ok` for all crypt tests.

4. Commit:

       git add agent/src/crypt/rsa.rs agent/tests/crypt/rsa.rs
       git commit -m "feat(crypt): add JWK serializer and RS512 sign helper"

### M2: Rewrite `authn::issue`

1. Replace `agent/src/authn/issue.rs` body with the new flow from Plan of Work §M2. Key changes:
   - Remove `backend_api::models::{IssueDeviceClaims, IssueDeviceTokenRequest}` import.
   - Remove `IssueTokenClaim` struct.
   - Add `JwtHeader` and `JwtPayload` `#[derive(Serialize)]` structs:

         #[derive(Serialize)]
         struct JwtHeader {
             alg: &'static str,
             typ: &'static str,
             jwk: crate::crypt::rsa::Jwk,
         }

         #[derive(Serialize)]
         struct JwtPayload {
             jti: String,
             iat: i64,
             exp: i64,
         }

   - Implement `async fn build_self_signed_jwt(private_key_file: &File, public_key_file: &File) -> Result<String, AuthnErr>` per Plan of Work §M2.
   - Rewrite `pub async fn issue_token` to take `(http_client, private_key_file, public_key_file)` and call `devices::issue_token(http_client, devices::IssueTokenParams { token: &jwt }).await?`.

2. Update `agent/src/authn/mod.rs` re-export — the public name `issue_token` is unchanged; no edit needed.

3. Add new tests in `agent/tests/authn/issue.rs` (create the file if it does not exist; if it exists, extend). Module path `crate::authn::issue` for the in-tree test harness; otherwise expose `build_self_signed_jwt` via `#[cfg(feature = "test")] pub fn build_jwt_for_test(...) -> Result<String, AuthnErr>` in `issue.rs` (or mark the helper `pub(crate)` plus a `#[cfg(test)]` module). Decision: add a `#[cfg(feature = "test")] pub` shim in `issue.rs` so integration tests under `agent/tests/` can call it. Tests:

   - `jwt_has_three_segments` — generate key pair via `rsa::gen_key_pair`, call the shim, assert `jwt.split('.').count() == 3`.
   - `jwt_header_decodes_to_rs512_with_jwk` — split, base64url-no-pad-decode segment 0, parse as JSON, assert `alg == "RS512"`, `typ == "JWT"`, and `jwk.kty == "RSA"`, `jwk.n != ""`, `jwk.e != ""`.
   - `jwt_payload_decodes_with_jti_iat_exp` — base64url-no-pad-decode segment 1, parse as JSON, assert `jti` is a non-empty string parseable by `uuid::Uuid::parse_str`, `iat` is within ±5s of `Utc::now().timestamp()`, `exp == iat + 120`.
   - `jwt_signature_verifies_with_public_key` — re-construct `signing_input = format!("{}.{}", parts[0], parts[1])`, base64url-no-pad-decode `parts[2]`, then verify directly via openssl `Verifier::new(MessageDigest::sha512(), &pkey)` — assert `true`. Also assert that flipping a byte in `signing_input` causes verification to return `false`.
   - `jti_is_unique_across_calls` — call the shim twice, decode both payloads, assert their `jti` fields differ.

4. Add `agent/tests/authn/mod.rs` line `pub mod issue;` if not present.

5. Run:

       ./scripts/test.sh -- --test authn

   Expect all authn tests to pass.

6. Commit:

       git add agent/src/authn/issue.rs agent/tests/authn/
       git commit -m "feat(authn): build self-signed RS512 JWT for issue_token"

### M3: `http::devices::issue_token`

1. Edit `agent/src/http/devices.rs`:
   - Change import to `use backend_api::models::{Device, ProvisionDeviceRequest, TokenResponse, UpdateDeviceFromAgentRequest};` (drop `IssueDeviceTokenRequest`).
   - Replace `IssueTokenParams` per Plan of Work §M3.
   - Replace `issue_token` body per Plan of Work §M3.

2. Commit (will be part of M5 once tests are updated, since this would otherwise break the build mid-milestone — we sequence: do M3 + M5 in one milestone). See M5 final commit.

### M4: Plumb public key path

1. Edit `agent/src/authn/token_mngr.rs`:
   - Add `public_key_file: File` field to `SingleThreadTokenManager`.
   - Update `new` to accept and assert `public_key_file`.
   - Update `TokenManager::spawn` signature to accept `public_key_file: File`.
   - Update the `issue_token` method (line 75) to pass `&self.public_key_file` after `&self.private_key_file` and drop the `&self.device_id` argument. The `device_id` field is no longer used by `issue_token`; remove the field if it has no other readers (run `grep -n "device_id" agent/src/authn/token_mngr.rs` to confirm). Decision-log the removal if applicable.

2. Edit `agent/src/app/state.rs::AppState::init`:
   - After `let private_key_file = auth_dir.private_key();` add:

         let public_key_file = auth_dir.public_key();
         public_key_file.assert_exists()?;

   - Pass `public_key_file` to `TokenManager::spawn` in the corresponding argument slot.

3. Edit `agent/src/app/upgrade.rs::reconcile`:
   - Below `let private_key_file = auth_dir.private_key();` add `let public_key_file = auth_dir.public_key(); public_key_file.assert_exists()?;`.
   - Replace both `authn::issue_token(http_client, &private_key_file, &device_id)` calls with `authn::issue_token(http_client, &private_key_file, &public_key_file)`.
   - Confirm `device_id` is still used downstream (PATCH at line 76).

4. Edit `agent/tests/authn/token_mngr.rs`:
   - In `setup`, before `TokenManager::spawn`, add:

         let public_key_file = dir.file("public_key.pem");
         public_key_file
             .write_string("public_key", WriteOptions::default())
             .await
             .unwrap();

     and pass `public_key_file` to `spawn`.
   - In `setup_with_rsa`, pass `public_key_file` (already created on disk) to `spawn`.
   - In the existing failure-case tests `token_file_does_not_exist` and `private_key_file_does_not_exist`, write a valid `public_key.pem` so the failure under test is the only missing precondition.
   - Add new test `pub mod spawn { ... } public_key_file_does_not_exist` that asserts `AuthnErr::FileSysErr` when only the public key is missing.
   - Update the `invalid_private_key` refresh test: it currently uses the dummy-key `setup`, which now also has a dummy public key — fine because the failure happens in `read_private_key` first. But the new code path also reads the public key. Confirm by running the test that the error variant remains `AuthnErr::CryptErr(_)` — it should, because `read_public_key` would fail on the dummy `"public_key"` PEM the same way. If the variant changes, update the assertion.

5. Edit `agent/tests/sync/syncer.rs`:
   - Read full file, mirror the `setup`/`setup_with_rsa` pattern for the public key, pass `public_key_file` to `TokenManager::spawn` at line 44.

6. Run:

       cargo build --features test --package miru-agent

   Expect a clean build.

### M5: Update http-layer issue_token tests + commit M3+M5

1. Edit `agent/tests/http/devices.rs` per Plan of Work §M5.

2. Run targeted test:

       ./scripts/test.sh -- --test http::devices

   Expect `issue_token::success` and `issue_token::error_propagates` to pass.

3. Run the full integration test suite to confirm `tests/mocks/http_client.rs::match_route` still routes the new path correctly:

       ./scripts/test.sh

   Expect all tests passing. If any test panics with `MockClient: unhandled route: POST /devices/issue_token`, edit `tests/mocks/http_client.rs::match_route` to add an explicit `(m, p) if *m == Method::POST && p == "/devices/issue_token" => Call::IssueDeviceToken,` arm. The current `p.ends_with("/issue_token")` should already cover this.

4. Commit M3 + M4 + M5 together (they share a build dependency):

       git add agent/src/http/devices.rs agent/src/authn/token_mngr.rs agent/src/app/state.rs agent/src/app/upgrade.rs agent/tests/http/devices.rs agent/tests/authn/token_mngr.rs agent/tests/sync/syncer.rs
       git commit -m "refactor: switch issue_token to /devices/issue_token with self-signed JWT auth"

   (If `tests/mocks/http_client.rs` was edited, include it.)

### M6: Preflight

1. From repo root:

       ./scripts/preflight.sh

   Expect final line `Preflight clean`. Common follow-ups:
   - Coverage gate failure under `agent/src/crypt` or `agent/src/authn` — add tests in `agent/tests/crypt/rsa.rs` or `agent/tests/authn/issue.rs` covering branches that were missed (e.g. error mapping for `read_public_key` failure in `build_self_signed_jwt`, error mapping for `sign_rs512` failure on a missing private key).
   - Lint: import order or unused-import warnings — fix per `agent/AGENTS.md` import ordering rules.

2. If any new errors variants were added, run `./scripts/lint.sh` separately to surface clippy errors before re-running preflight.

3. Final commit (only if step 1 produced extra fix-ups):

       git add -A
       git commit -m "chore: address preflight findings for issue_token refactor"

## Validation and Acceptance

Acceptance is verifiable by behavior, not implementation details:

1. Build passes: `cargo build --features test --package miru-agent` from repo root exits 0.
2. Tests pass: `./scripts/test.sh` reports all tests passing.
3. New tests pass and exercise the intended behavior:
   - `crypt::rsa::tests::rsa_public_key_to_jwk::success_deterministic_for_known_key` — calling the JWK serializer twice on the same parsed public key returns identical output.
   - `crypt::rsa::tests::sign_rs512::verifies_with_sha512` — signature produced by `sign_rs512` verifies with `Verifier::new(MessageDigest::sha512(), ...)` and not with `MessageDigest::sha256()`.
   - `authn::issue::tests::jwt_has_three_segments` — built JWT has exactly three `.`-separated segments.
   - `authn::issue::tests::jwt_header_decodes_to_rs512_with_jwk` — header decodes to `{"alg":"RS512","typ":"JWT","jwk":{"kty":"RSA",...}}`.
   - `authn::issue::tests::jwt_payload_decodes_with_jti_iat_exp` — payload has `jti`, `iat`, `exp` with the expected ranges.
   - `authn::issue::tests::jwt_signature_verifies_with_public_key` — signature verifies against the device's public key.
   - `authn::issue::tests::jti_is_unique_across_calls` — two calls produce two different `jti` values.
   - `http::devices::issue_token::success` — POST is to path `/devices/issue_token`, body is empty, bearer token is set.
4. Coverage gates: `./scripts/covgate.sh` completes with `✅ All modules meet minimum coverage requirement`. In particular, `agent/src/authn` ≥ 95.31 and `agent/src/crypt` ≥ 94.98.
5. Preflight: `./scripts/preflight.sh` final line is `Preflight clean`. The PR is not opened unless this is true.

Pre-change state: `cargo build --features test --package miru-agent` fails with `unresolved import backend_api::models::IssueDeviceClaims` (and `IssueDeviceTokenRequest`). Post-change: build succeeds and the dangling import is gone.

## Idempotence and Recovery

- All edits are pure source/test changes; rerunning steps re-edits the same file content, which is safe.
- Each milestone ends with a commit, so reverting any milestone is a single `git revert <sha>`.
- The added `sign_rs512` is additive; if M2 needs to be rolled back, M1's helper sits unused but harmless. The new `Jwk` struct is similarly inert when unused.
- The two-arg → three-arg signature changes for `TokenManager::spawn` and `issue::issue_token` are coordinated within M3+M4+M5's single commit, so the tree is never in a state where one side of the API is updated and the other is not.
- If preflight (M6) flags coverage shortfalls, add tests rather than lowering the `.covgate` threshold.
