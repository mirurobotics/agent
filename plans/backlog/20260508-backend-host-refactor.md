# Replace `BackendUrl` with `BackendHost` newtype

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All code, tests, and the plan file live here. |

This plan lives in `agent/plans/` because every file changed is in this repo.

## Purpose / Big Picture

Today users configure the agent's backend with a full URL — `https://api.mirurobotics.com/agent/v1` — but the validator already forces every component except the host. Scheme is determined by whether the host is loopback; the path is fixed by the API contract in `libs/backend-api/`. That gives users two extra ways to misconfigure (wrong scheme, wrong path) without any real choice.

After this refactor the public surface is a bare hostname (optionally `host:port`), mirroring the existing `MqttHost` newtype in the same module. Scheme and path move inside the new `BackendHost` type as derived/hardcoded values. The CLI flag `--backend-host` is already host-shaped, so the main externally visible change is the on-disk `settings.json` field rename `backend.base_url` → `backend.host` (with one further CLI behavior change noted below). Behavior a user can observe: the agent runs against `api.mirurobotics.com` configured as a hostname; an old settings file with the legacy `base_url` field is treated as missing and the default is used (with a warn-log), exactly the same fallback pattern used today for any invalid backend value. The CLI flag `--backend-host` is already documented as a hostname, but the previous `BackendUrl` validator silently accepted scheme-prefixed values such as `https://api.mirurobotics.com`. Under the new validator those inputs are rejected and the default is used (with a warn-log), so any user or script passing a full URL via `--backend-host` must drop the scheme and path. This is the same reject-and-warn pattern the validator applies to disallowed domains today.

### Out of Scope

- `MqttHost` and its validation — pattern is mirrored, not modified.
- The `is_loopback_host` / `is_allowed_host` helpers in `network/mod.rs` — reused as-is.
- `http::Client::new(base_url: &str)` — continues to take a `&str`; only the producer of that string changes (callers now pass `BackendHost::as_url()` instead of `BackendUrl::as_str()`).
- Generated code under `libs/backend-api/` and `libs/device-api/`.

## Progress

- [ ] Milestone 1 — Add `BackendHost` type and unit tests.
- [ ] Milestone 2 — Migrate every call site and integration test.
- [ ] Milestone 3 — Delete `BackendUrl` and confirm no stale references.
- [ ] Final preflight pass (only if Milestone 3 leaves residue).

(Add timestamps and split partial work as implementation proceeds.)

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

- Decision: Hard rename of the settings field (`base_url` → `host`); no `#[serde(alias = "base_url")]`, no migration code.
  Rationale: `Backend::deserialize` in `agent/src/storage/settings.rs` already treats a missing or invalid backend value as "use default and warn-log" — that path covers existing on-disk files automatically. A serde alias would feed a full URL string into `BackendHost::new`, which rejects URLs (only hosts allowed), so the alias would not actually accept legacy data without extra URL-parsing logic; that is strictly more code than the warn-and-default path. The prior plan that introduced `BackendUrl` (`plans/completed/20260501-allowed-domains-newtypes.md`) used the same warn-and-default approach, so this is consistent. Note: after the rename, the warn-log message refers to the missing `host` field, not to the legacy `base_url` field — the legacy field is silently treated as an unknown key. If legacy-warning ergonomics matter, `Backend::deserialize` can additionally detect `base_url` presence and emit a more informative warning; this is out of scope for the rename itself but recorded here as a follow-up. The deserialize impl shape is therefore identical to today's — only the read field name and the warn-log key change.
  Date/Author: 2026-05-08 / ben.

- Decision: The `/agent/v1` API-version path is hardcoded inside `BackendHost::as_url()` rather than being part of the on-disk or CLI surface.
  Rationale: the path is fixed by the API contract in `libs/backend-api/`; exposing it to users provides no real choice and is a frequent source of misconfiguration today.
  Date/Author: 2026-05-08 / ben.

(Add further entries as decisions arise.)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent is a Rust binary in this repo (workspace package `miru-agent`) that runs on customer devices. It connects to the Miru backend over HTTPS for REST and an EMQX broker over MQTT.

Key files for this plan, all paths relative to `agent/`:

- `src/network/mod.rs` — defines newtypes for network endpoints. Lines 9-18 hold the validation helpers `is_loopback_host` and `is_allowed_host` (loopback matches `localhost`/`127.0.0.1`/`::1`; allowed-domain matches `mirurobotics.com` exactly or any `.mirurobotics.com` suffix). Lines 20-105 hold the current `BackendUrl` (wraps `url::Url`, validates URL is loopback-or-mirurobotics, scheme is `https` except `http` on loopback). Lines 107-163 hold `MqttHost`, which wraps a `String`, validates it as loopback-or-allowed-domain, and is the pattern this refactor mirrors. Both types implement `new`, `new_or` (warn-log + fallback), `as_str`, `Display`, serde, and `Default`.
- `src/storage/mod.rs` line 24 re-exports `BackendUrl` from `network`.
- `src/storage/settings.rs` — `Backend` struct with `pub base_url: BackendUrl` (line 93) and a hand-written `Deserialize` impl (lines 96-123) using a `deserialize_warn!` macro that falls back to the default value with a warn-log when a field is missing or invalid.
- `src/storage/layout.rs:29` — settings.json layout. Today the on-disk shape is `{ "backend": { "base_url": "https://api.mirurobotics.com/agent/v1" }, ... }`.
- `src/main.rs` — wires settings to runtime. `BackendUrl` is used at lines 16 (import), 62 and 115 (constructing `http::Client`), 168-169 and 233-240 (the `get_bootstrap_base_url` helper), and 216 (`AppOptions { backend_base_url: ... }`).
- `src/app/options.rs` — `AppOptions { pub backend_base_url: BackendUrl }` at line 46, with default at line 67.
- `src/app/run.rs:166` — passes the URL to `http::Client::new`.
- `src/provisioning/shared.rs` lines 36-49 — `determine_settings` currently does `let raw = format!("{host}/agent/v1"); BackendUrl::new_or(&raw, BackendUrl::default())`, which is the only place the `/agent/v1` suffix is appended at runtime today.
- `src/provisioning/provision.rs` (~lines 110-162) and `src/provisioning/reprovision.rs` (~lines 80-132) — tests assert on `defaults.backend.base_url`. Both files contain a test named `backend_host_appends_agent_v1_suffix` that pins this concatenation behavior, and an `invalid_backend_host_falls_back_to_default` test that exercises the warn-and-default path.
- `src/cli/mod.rs` — already exposes `--backend-host=<hostname>` (no scheme, no path). No CLI changes needed.
- `src/http/client.rs:71` — `http::Client::new(base_url: &str)` stores the string and downstream callers (`releases.rs`, `git_commits.rs`, `config_instances.rs`, `devices.rs`, `deployments.rs`) do `format!("{}/<resource>", client.base_url())`. The full URL passed in must therefore end at `/agent/v1` with no trailing slash.
- `tests/network/mod.rs` lines 4-164 — `mod backend_url_new`, ~19 tests covering validation. To be replaced with `mod backend_host_new`.
- `tests/storage/settings.rs` — fixtures at lines 18, 35, 72, 82, 89, and 144-161 build `BackendUrl` and reference the `base_url` field. The test `deserialize_backend_falls_back_on_disallowed_host` (line 144) exercises the warn-and-default path.
- `tests/app/options.rs` — constructs `AppOptions` with `backend_base_url`.
- `tests/provisioning/{provision,reprovision}.rs` — integration tests for the provisioning flow; they do not currently reference `backend.base_url` directly, so the migration touches them only if a backend-shaped fixture is added.

Repo conventions to follow (see `agent/AGENTS.md`):

- Build with `cargo build`. Run tests with `./scripts/test.sh` (= `RUST_LOG=off cargo test --features test`). Lint with `./scripts/lint.sh`. The full gate before opening a PR is `./scripts/preflight.sh`.
- Tests using shared OS resources annotate with `#[serial]` from `serial_test`.
- Conventional Commits style for messages (e.g., `refactor: replace BackendUrl with BackendHost`).
- Import order in each file: stdlib group → `use crate::` group → external crates group, blank line between groups.
- Custom clippy lint allows `new_without_default` — newtypes need not derive `Default`.
- The custom import linter has a "field-by-field-assert" rule: 4+ `assert_eq!` on the same variable's fields is a violation; suppress per test with `// lint:allow(field-by-field-assert)` if needed.
- `agent/src/storage/.covgate` requires 94.21% line coverage on `agent/src/storage/`. Tests touching `storage/settings.rs` must keep coverage at or above this.
- Generated code under `libs/backend-api/` and `libs/device-api/` must not be hand-edited.

## Plan of Work

### New type

In `agent/src/network/mod.rs`, add `BackendHost` next to `MqttHost`. Recommended representation:

    pub struct BackendHost {
        host: String,         // bare hostname, validated
        port: Option<u16>,
    }

    impl BackendHost {
        pub fn new(raw: &str) -> Result<Self, String> { ... }
        pub fn new_or(raw: &str, fallback: Self) -> Self { ... }
        pub fn as_url(&self) -> String { ... } // owned String, built per call
    }

Building `as_url()` per call is fine — it's invoked at startup, not in a hot loop. (Alternative: store the pre-built URL string alongside the host; if chosen, document the choice in the Decision Log.)

`new()` validation rules — reject any of the following:

- empty input,
- input containing `/` (a path is being passed, not a host),
- input containing `@` (userinfo),
- input containing `://` (a scheme; full URL was passed),
- a host that is neither loopback (per `is_loopback_host`) nor an allowed mirurobotics domain (per `is_allowed_host`),
- a non-numeric or out-of-range port.

Splitting host from port: there are two reasonable approaches. Pick one and note it in the Decision Log.

1. Parse via `url::Url::parse(&format!("http://{raw}"))`, then read `.host_str()` and `.port()`. This handles IPv6 bracket form (`[::1]:8080`) for free but means the public surface still requires brackets for IPv6+port — acceptable since IPv6 is loopback-only here.
2. Hand-split on the last `:` and treat IPv6 literals (`::1`) as the unbracketed special case (port-less IPv6 only). Simpler, narrower.

`new_or(raw, fallback)` mirrors `MqttHost::new_or`: log a warning containing the rejection reason and return `fallback` on `Err`.

`as_url()` rules:

- Scheme: `http` if `is_loopback_host(self.host.as_str())`, else `https`.
- Authority: `{host}` or `{host}:{port}` if port present (use bracket form for IPv6 if approach 1 was chosen — `url::Url` will format it correctly).
- Path: always `/agent/v1`.
- No trailing slash, so existing callers can `format!("{}/<resource>", base)` (see `agent/src/http/client.rs` and its consumers).

Trait impls: `Display` (writes the same string the user typed — host or `host:port`), `Serialize` as that string, `Deserialize` that calls `Self::new` and propagates errors via `serde::de::Error::custom`, and `Default` returning `Self::new("api.mirurobotics.com").expect("default backend host must be valid")` (mirrors `MqttHost::default`).

In Milestone 2, update the `pub use` in `agent/src/storage/mod.rs:24` to add `BackendHost` alongside `BackendUrl` so call-site fixtures can import either type during migration. In Milestone 3, drop `BackendUrl` from that re-export when its definition is removed.

### Call-site migration

- `agent/src/storage/settings.rs`:
  - Line 4: `use crate::network::{BackendUrl, MqttHost};` → `use crate::network::{BackendHost, MqttHost};`.
  - Line 93: `pub base_url: BackendUrl` → `pub host: BackendHost`.
  - Lines 96-123 (`Backend::deserialize`): read `host: Option<String>` instead of `base_url: Option<String>`; change the `deserialize_warn!` field name from `"base_url"` to `"host"`; keep the same warn-log + fallback shape, calling `BackendHost::new_or(&raw, default.host)`.
- `agent/src/main.rs`:
  - Line 16: import `BackendHost` instead of `BackendUrl`.
  - Lines 62, 115: `http::Client::new(settings.backend.base_url.as_str())?` → `http::Client::new(&settings.backend.host.as_url())?`.
  - Lines 168-169: rename `let url = get_bootstrap_base_url().await;` to `let host = get_bootstrap_backend_host().await;` and `http::Client::new(url.as_str())` → `http::Client::new(&host.as_url())`.
  - Line 216: `backend_base_url: settings.backend.base_url` → `backend_host: settings.backend.host`.
  - Lines 233-240: rename `async fn get_bootstrap_base_url() -> BackendUrl` to `get_bootstrap_backend_host() -> BackendHost`; update body to read `settings.backend.host` and fall back to `storage::Backend::default().host`.
- `agent/src/app/options.rs`:
  - Line 6: import `BackendHost`.
  - Line 46: `pub backend_base_url: BackendUrl` → `pub backend_host: BackendHost`.
  - Line 67 default: `BackendHost::default()`.
- `agent/src/app/run.rs:166`: `Arc::new(http::Client::new(options.backend_base_url.as_str())?)` → `Arc::new(http::Client::new(&options.backend_host.as_url())?)`.
- `agent/src/provisioning/shared.rs`:
  - Line 6: import `BackendHost`.
  - Lines 36-49 (`determine_settings`): replace the two-statement block `let raw = format!("{host}/agent/v1"); settings.backend.base_url = BackendUrl::new_or(&raw, BackendUrl::default());` with the single statement `settings.backend.host = BackendHost::new_or(host, BackendHost::default());`. The `/agent/v1` suffix is now produced by `BackendHost::as_url()` instead of by the call site.
- `agent/src/provisioning/provision.rs` and `reprovision.rs`:
  - Update test assertions that compare against `defaults.backend.base_url` to compare against `defaults.backend.host`.
  - Repurpose `backend_host_appends_agent_v1_suffix` in both `provision.rs` (lines ~110-120) and `reprovision.rs` (lines ~80-90): change the existing input `Some("https://custom.mirurobotics.com".to_string())` to a bare hostname `Some("custom.mirurobotics.com".to_string())` (the new `BackendHost::new` rejects scheme-prefixed inputs), then assert that the `determine_settings` return value's `backend.host.as_url()` ends with `/agent/v1` (the existing tests bind this as `let settings = determine_settings(&args);`, so the assertion is `settings.backend.host.as_url().ends_with("/agent/v1")`). If the new `BackendHost` unit tests already cover this, delete both tests outright and note the deletion in the Decision Log. Also review the `invalid_backend_host_falls_back_to_default` tests: the current input rejects on disallowed-domain; under the new validator the input may also be rejected for containing `://`. Update the test comment to reflect whichever rule the test is meant to exercise.

### Test changes

- `agent/tests/network/mod.rs` lines 4-164: replace `mod backend_url_new` with `mod backend_host_new` covering:
  - valid bare hostname (`api.mirurobotics.com`);
  - valid loopback (`localhost`, `127.0.0.1`, `::1`);
  - valid host with port (`api.mirurobotics.com:8443`, `localhost:8080`);
  - invalid host: empty string;
  - invalid host: wrong domain (`evil.com`, `notmirurobotics.com`);
  - invalid host: input contains a scheme (`https://api.mirurobotics.com`);
  - invalid host: input contains userinfo (`user:pass@api.mirurobotics.com`);
  - invalid host: input contains a path (`api.mirurobotics.com/agent/v1`);
  - invalid host: non-numeric or out-of-range port;
  - `Default` returns `api.mirurobotics.com`;
  - `Display` round-trip;
  - serde round-trip (string ⇄ `BackendHost`);
  - `new_or` returns the fallback on invalid input;
  - `as_url()` output: non-loopback → `https://<host>/agent/v1`; loopback (`localhost`, `127.0.0.1`, `::1`) → `http://<host>/agent/v1`; with port → `https://<host>:<port>/agent/v1` and `http://<host>:<port>/agent/v1`.
- `agent/tests/storage/settings.rs`: update fixtures at lines 18, 35, 72, 82, 89, and 144-161 to construct `BackendHost::new("api.mirurobotics.com")` and use the field name `host`. Reframe `deserialize_backend_falls_back_on_disallowed_host` (line 144) to also serve as the regression for legacy on-disk files: a JSON object containing only `{ "base_url": "https://api.mirurobotics.com/agent/v1" }` should deserialize to the default `Backend` (because the new code reads `host`, the legacy field is just an unknown key).
- `agent/tests/app/options.rs`: update the `AppOptions` fixture to use `backend_host` of type `BackendHost`.
- `agent/src/provisioning/provision.rs` and `agent/src/provisioning/reprovision.rs` (inline `#[cfg(test)] mod tests`): update assertions to use `backend.host`; repurpose or delete `backend_host_appends_agent_v1_suffix` per the Plan of Work above.

Watch for the import linter's "field-by-field-assert" rule when any test grows past 3 `assert_eq!` calls on the same value. Suppress with `// lint:allow(field-by-field-assert)` if intentional.

### Removal

After call sites compile and tests pass, delete the `BackendUrl` definition (lines 20-105 of `network/mod.rs`), its `pub use` from `storage/mod.rs`, and the `mod backend_url_new` block in `tests/network/mod.rs` (already gone if the test file was edited in Milestone 1).

## Concrete Steps

All commands run from the repo root: `cd /home/ben/miru/workbench2/repos/agent`. The branch `refactor/backend-host` is already checked out off `main`. Source paths in commands below are relative to that root (e.g. `agent/src/...`, `agent/tests/...`).

### Milestone 1 — Add `BackendHost`

1. Edit `agent/src/network/mod.rs` to add the `BackendHost` type, its impls (`new`, `new_or`, `as_url`, `Display`, `Serialize`, `Deserialize`, `Default`), keeping `BackendUrl` intact for now. Do not yet touch the `pub use` in `agent/src/storage/mod.rs`; the re-export update lands in Milestone 2.
2. Add `mod backend_host_new` (and any sibling modules for serde/Default/`as_url` round-trips) in `agent/tests/network/mod.rs`. Leave `mod backend_url_new` in place.
3. Run the gates:

       cargo build
       cargo fmt -p miru-agent -- --check
       cargo clippy --package miru-agent --all-features -- -D warnings
       ./scripts/test.sh

   Expected: build clean, fmt clean, clippy clean, all tests pass (the existing `BackendUrl` tests still pass because nothing else changed; the new `BackendHost` tests also pass).
4. Commit:

       git add agent/src/network/mod.rs agent/tests/network/mod.rs
       git commit -m "refactor(network): add BackendHost newtype alongside BackendUrl"

### Milestone 2 — Migrate call sites

1. Apply every edit listed under "Call-site migration" and "Test changes" in Plan of Work.
2. Run:

       cargo build
       cargo fmt -p miru-agent -- --check
       cargo clippy --package miru-agent --all-features -- -D warnings
       ./scripts/test.sh

   Expected: build clean, all tests pass. After this milestone the source tree compiles only because `BackendUrl` is unused — no caller references it anymore. (clippy may warn about `dead_code` on `BackendUrl` itself; suppress with `#[allow(dead_code)]` only if needed to keep clippy clean before Milestone 3, and remove the suppression in Milestone 3.)
3. Verify the call-site migration is complete:

       rg -n 'BackendUrl|backend_base_url|backend\.base_url' agent/src/ agent/tests/

   Expect hits only inside the `BackendUrl` definition itself in `agent/src/network/mod.rs` (still present, removed in Milestone 3) and any `mod backend_url_new` block remaining in `agent/tests/network/mod.rs`. Unrelated `base_url` symbols (`http::Client.base_url`, mock `server.base_url`, `invalid_base_url_returns_error`) are not migration targets.
4. Commit:

       git add -A
       git commit -m "refactor: migrate agent call sites from BackendUrl to BackendHost"

### Milestone 3 — Remove `BackendUrl`

1. Delete the `BackendUrl` struct, its impls, and any associated helpers in `agent/src/network/mod.rs`. Remove `BackendUrl` from the `pub use` in `agent/src/storage/mod.rs:24`. Delete `mod backend_url_new` from `agent/tests/network/mod.rs` if it still exists.
2. Verify:

       rg -n 'BackendUrl' agent/src/ agent/tests/

   Expect zero hits.
3. Run the full preflight:

       ./scripts/preflight.sh

   Expected: clean. This includes `./scripts/lint.sh` (custom import linter, fmt, clippy, machete, rustsec), `./scripts/covgate.sh` (storage threshold ≥94.21%), and the tools lint and tests. If `covgate.sh` reports a coverage dip on `agent/src/storage/` (threshold ≥94.21%), the most likely cause is that the renamed `Backend::deserialize` warn-and-default branch is no longer exercised end-to-end; restore coverage by adjusting fixtures in `agent/tests/storage/settings.rs` rather than lowering the threshold.
4. Commit:

       git add -A
       git commit -m "refactor: remove BackendUrl in favor of BackendHost"

### Milestone 4 (only if needed) — Cleanup

If preflight surfaced findings (clippy nags, fmt drift, covgate dip on `agent/src/storage/`), fix them and commit as `chore: address preflight findings after BackendHost refactor`. Re-run `./scripts/preflight.sh` and confirm clean.

## Validation and Acceptance

The acceptance gate is the four canonical commands plus preflight, all run from the repo root (`/home/ben/miru/workbench2/repos/agent`):

- `cargo build` — succeeds with no warnings.
- `cargo fmt -p miru-agent -- --check` — exits 0.
- `cargo clippy --package miru-agent --all-features -- -D warnings` — exits 0.
- `./scripts/test.sh` — all tests pass; in particular the new `mod backend_host_new` tests pass and all existing storage/provisioning/app tests pass after their fixtures were updated.
- `./scripts/preflight.sh` — reports clean (lint + covgate + tools lint + tools tests).

Behavioral checks a reader can confirm by inspection of the test output:

- A test in `mod backend_host_new` asserts `BackendHost::new("api.mirurobotics.com").unwrap().as_url() == "https://api.mirurobotics.com/agent/v1"`.
- A test asserts `BackendHost::new("localhost").unwrap().as_url() == "http://localhost/agent/v1"`.
- A test asserts `BackendHost::new("api.mirurobotics.com:8443").unwrap().as_url() == "https://api.mirurobotics.com:8443/agent/v1"`.
- `BackendHost::new("https://api.mirurobotics.com")` returns `Err`.
- `BackendHost::new("api.mirurobotics.com/agent/v1")` returns `Err`.
- A test in `agent/tests/storage/settings.rs` asserts that deserializing `{"backend": {"base_url": "https://api.mirurobotics.com/agent/v1"}, ...}` yields `Backend::default()` and emits a warn-log about the missing `host` field.

A regression check: before the refactor the `backend_host_appends_agent_v1_suffix` test in both `agent/src/provisioning/provision.rs` and `agent/src/provisioning/reprovision.rs` exercised the `format!("{host}/agent/v1")` concatenation. After the refactor those tests should either (a) be reframed to assert `settings.backend.host.as_url().ends_with("/agent/v1")` (matching the existing `let settings = determine_settings(&args);` binding) or (b) be deleted as redundant once `BackendHost::as_url()` is unit-tested. State whichever is chosen in the Decision Log.

## Idempotence and Recovery

- Every edit is a normal source-file edit; re-running `cargo build` or `./scripts/test.sh` is safe and idempotent.
- The three commits are independent and can be inspected or reverted individually:

      git revert <commit>           # revert a single milestone

  Reverts roll back cleanly because each milestone leaves the tree compiling.
- If Milestone 2 leaves the tree non-compiling at any intermediate point, that is normal during the edit pass; the milestone is committed only after `cargo build` and `./scripts/test.sh` succeed.
- No on-disk migration is performed, so there is no data to back up. Existing `settings.json` files with the old `base_url` field are handled by the deserialize-fallback path (see Decision Log) and silently fall back to defaults with a warn-log.
- If the work is interrupted, resume from the last completed milestone. The Progress checklist tracks which milestones are done.
- To restart from scratch: `git checkout main && git branch -D refactor/backend-host && git checkout -b refactor/backend-host`. Then re-execute Milestones 1-3.
