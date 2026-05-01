# Push allowed-domain invariant into newtypes for backend URL and MQTT host

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, base branch `feat/harden-allowed-domains`) | read-write | Source, tests, plan file. |

This plan lives in `plans/backlog/` of the agent repo because all code changes are in this repo. The base branch is the existing PR `feat/harden-allowed-domains` (PR #49); the new work is a refactor of what that branch already shipped.

## Purpose / Big Picture

After this change, it is impossible at compile time to construct a `Settings` value (or hand a `ConnectAddress` to the MQTT client) whose backend URL or MQTT broker host violates the allowed-domain rule. Today the rule is enforced by free-function validators (`validate_backend_url`, `validate_mqtt_host`) called from three edge sites (CLI provisioning, `Deserialize` impls, `ConnectAddress::validate`); a future caller that constructs a `Backend` or `MQTTBroker` literal directly can sidestep all of them. After the change, the `String` field is replaced with a newtype whose only constructor runs the validator, so any `Settings` instance in memory is necessarily valid. The runtime `ConnectAddress::validate` call in `agent/src/main.rs` and the defence-in-depth IP check inside `validate_backend_url` collapse into "this is just how the type works."

User-visible behavior is unchanged. The `19ee1ac` warn-and-fall-back semantics for invalid on-disk settings is preserved by the newtype's `Deserialize` impl. The `de0f505` hard-fail at provisioning time is preserved by surfacing the constructor error from `determine_settings`. Operators still see the same warnings in logs and the same CLI exit codes.

## Progress

- [x] M1–M4 (combined commit c535244, 2026-04-30): Introduced `BackendUrl` and `MqttHost` newtypes in `agent/src/storage/validation.rs`; replaced `Backend.base_url: String` and `MQTTBroker.host: String` with the newtypes; replaced `ConnectAddress.broker: String` with `MqttHost`, deleted `ConnectAddress::validate`, and added a gated `ConnectAddress::new` constructor that enforces only the residual SSL-unless-loopback rule (host validity is type-enforced); kept `InvalidConnectAddressErr` (option (a)); propagated `BackendUrl` into `AppOptions::backend_base_url` (option (b)); updated `provision::determine_settings` to construct the newtypes directly. The orchestrator opted for one combined source-side commit rather than four micro-commits because main.rs touches all four milestones and per-milestone HEAD states would not all compile.
- [x] M5 (commit 22072f5, 2026-04-30): Renamed validator test modules (`backend_url_new`, `mqtt_host_new`); replaced `mod validate` for `ConnectAddress` with smaller `mod connect_address_new`; updated all `ConnectAddress` literals to construct via `MqttHost::new(...)`. Added `Default::default` smoke tests for both newtypes plus deserialize-strictness tests. Switched `mqtt/client.rs::invalid_broker_url` from `192.0.2.1` (no longer constructable) to a closed loopback port to preserve the network-error path.
- [x] M6 (commit c5de76a, 2026-04-30): Preflight clean. Storage covgate (94.21%) was breached after M5 (drop to 92.99%) because the refactor mechanically moved lines around; added `display_matches_as_str` and `rejects_url_without_host` tests to bring it back to 94.89% without lowering the threshold. One flaky `app::run::shutdown_signal_received` timeout under llvm-cov instrumentation; passed on rerun.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Take option (a) for `ConnectAddress` — keep `InvalidConnectAddressErr` and add a gated `ConnectAddress::new` constructor that enforces only the SSL-unless-loopback rule.
  Rationale: Preserves the invariant added in cafa8ef. Host validity is now statically guaranteed by `MqttHost`, leaving only the SSL-unless-loopback rule. The constructor is small and worth keeping.
  Date/Author: 2026-04-30 / Claude (implementation subagent)

- Decision: Propagate `BackendUrl` into `AppOptions::backend_base_url` (option (b) under M2).
  Rationale: Pushes the type-level invariant one layer further into runtime; no boxing or stringification needed at the boundary, only `as_str()` when handing off to `http::Client::new`.
  Date/Author: 2026-04-30 / Claude (implementation subagent)

- Decision: Keep `ConnectAddress` fields `pub` (option (i) under M3 field-visibility).
  Rationale: Tests routinely build literals; the SSL-unless-loopback rule is a soft preference rather than a security boundary, so enforcement at `ConnectAddress::new` is sufficient. Documented on the type.
  Date/Author: 2026-04-30 / Claude (implementation subagent)

- Decision: Combine M1–M4 source edits into one commit rather than four per-milestone commits.
  Rationale: `main.rs` has changes spanning M2, M3, and M4. Splitting per milestone would produce HEAD states that fail to compile (e.g. M1 alone removes free functions still used by M2's consumers), breaking bisectability. One combined refactor commit + one tests commit + one preflight-fixup commit is cleaner.
  Date/Author: 2026-04-30 / Claude (implementation subagent)

## Outcomes & Retrospective

2026-04-30 — Refactor complete. Preflight clean (storage covgate 94.89% > 94.21%). User-visible behavior unchanged; the new property is a compile-time guarantee that `Settings`, `Backend`, `MQTTBroker`, and `ConnectAddress` cannot be constructed with a disallowed host. `main.rs` lost ~14 lines (the validate-and-fall-back block for `ConnectAddress`); `mqtt/options.rs` has a smaller, more focused `new` constructor in place of the multi-rule `validate`. Total commits: 3 (source refactor, tests, preflight follow-up).

## Context and Orientation

The reader has only this plan file and the current working tree. Repo layout (full paths under `/home/ben/miru/workbench2/repos/agent/`):

- `agent/src/storage/validation.rs` — currently holds free-function validators. Three public/`pub(crate)` items:
  - `is_loopback_host(host: &str) -> bool` (line 12) — accepts `localhost`, `127.0.0.1`, `::1`.
  - `reject_non_loopback_ip(host: &str) -> Result<(), String>` (line 28, `pub(crate)`) — defence-in-depth IP rejection.
  - `validate_backend_url(raw: &str) -> Result<Url, String>` (line 50) — full URL validation: parse, no userinfo, scheme rule (`https`, or `http` for loopback only), allowed-domain or loopback host, defence-in-depth IP check.
  - `validate_mqtt_host(host: &str) -> Result<(), String>` (line 81) — bare hostname check: loopback or allowed-domain suffix.
  - `is_allowed_host(host: &str) -> bool` (line 16, private) — exact-match `mirurobotics.com` or `.mirurobotics.com` suffix.
- `agent/src/storage/settings.rs` — manual `Deserialize` impls for `Settings`, `Backend`, `MQTTBroker`. Today:
  - `Backend.base_url: String` (line 93) with default `"https://api.mirurobotics.com/agent/v1"`.
  - `MQTTBroker.host: String` (line 143) with default `"mqtt.mirurobotics.com"`.
  - `Backend::deserialize` (line 104-139) reads the raw string, runs `validation::validate_backend_url`, and on `Err` warns + substitutes `Backend::default().base_url`. The same pattern for `MQTTBroker::deserialize` (line 154-189).
  - `Settings::deserialize` (line 35-89) uses the `deserialize_warn!` macro from `agent/src/errors/mod.rs` lines 103-114 for missing fields; nested `Backend`/`MQTTBroker` deserialize handles invariant violations themselves.
- `agent/src/storage/mod.rs` line 25 re-exports `validate_backend_url`, `validate_mqtt_host`, `is_loopback_host` at the storage module root.
- `agent/src/provision/entry.rs::determine_settings` (line 90-112) — for each CLI override, runs `validate_backend_url` / `validate_mqtt_host`, on `Err` returns `ProvisionErr::InvalidSettingsErr` (defined in `agent/src/provision/errors.rs` line 19-26), on `Ok` assigns the raw string into `settings.backend.base_url` / `settings.mqtt_broker.host`.
- `agent/src/mqtt/options.rs` — `Protocol { TCP, SSL }` (line 9), `ConnectAddress { protocol, broker: String, port }` (line 14-19), `Default` is `{ Protocol::SSL, "mqtt.mirurobotics.com", 8883 }` (line 21-29). `ConnectAddress::validate(&self) -> Result<(), InvalidConnectAddressErr>` (line 36-52) calls `validate_mqtt_host` for the host and additionally rejects non-loopback hosts paired with `Protocol::TCP`.
- `agent/src/mqtt/errors.rs` line 109-116 — `InvalidConnectAddressErr { msg: String, trace: Box<Trace> }` plus `impl crate::errors::Error`. Listed in the `MQTTError` enum's `impl_error!` block? No — grep shows it isn't part of `MQTTError`; it's a free error type returned by `ConnectAddress::validate`.
- `agent/src/main.rs` — `run_agent` (line 87-186) reads settings from disk, builds `ConnectAddress { broker: settings.mqtt_broker.host, ..Default::default() }` (line 149-152), calls `.validate()` (line 153), warns and falls back to `ConnectAddress::default()` on `Err`. `run_provision` (line 43-68) calls `provision::determine_settings` (line 54).
- `agent/src/workers/mqtt.rs` line 31 — `Options { backoff, broker_address: ConnectAddress }` is the worker-side struct that holds the validated address.
- `agent/src/http/client.rs::Client::new` (line 71) accepts a `&str` `base_url`. The plan does not change this signature; we coerce `BackendUrl` to `&str` at the call site.
- `agent/Cargo.toml` already declares `url = { workspace = true }` (added in commit `d4aa68e`).

Tests:

- `agent/tests/storage/validation.rs` — 21 tests for `validate_backend_url` and `validate_mqtt_host`. Will be retargeted at `BackendUrl::new` / `MqttHost::new`.
- `agent/tests/storage/settings.rs` — round-trip serde tests, plus `deserialize_backend_falls_back_on_disallowed_host` (line 137), `_falls_back_on_http_non_loopback` (line 154), `deserialize_mqtt_broker_falls_back_on_disallowed_host` (line 161), `deserialize_settings_with_invalid_backend_url_falls_back` (line 175). These continue to pass and document the warn-and-fall-back behavior at the on-disk boundary.
- `agent/tests/provision/entry.rs::determine_settings` (line 256-336) — six cases asserting accept/reject for backend and mqtt overrides. These continue to pass; the constructor is what produces the rejection.
- `agent/tests/mqtt/options.rs::validate` (line 19-116) — eight cases exercising `ConnectAddress::validate`. These will be replaced (the equivalent assertions move to `MqttHost::new` tests; the SSL-unless-loopback rule no longer exists at runtime — see Decision Log placeholder below).
- `agent/tests/mqtt/client.rs` and `agent/tests/mqtt/errors.rs` construct `ConnectAddress` literals with `broker: String`. These are updated to use `MqttHost::new` with a loopback or allowed-domain host (or a new `MqttHost::loopback()` helper — see Plan of Work).

Definitions:

- **Backend URL**: the URL used for HTTPS calls to the Miru control plane. Today `String`; after this change `BackendUrl` (a newtype around `Url`).
- **MQTT broker host**: bare hostname for the MQTT broker connection. Today `String`; after this change `MqttHost` (a newtype around `String`).
- **Newtype with gated constructor**: a `pub struct Foo(Inner)` where `Inner` is private and the only public constructor is `Foo::new(...) -> Result<Self, Err>`. This forces every in-memory instance through validation. Existing pattern in this codebase: see `agent/src/authn/Token` for similar wrapping.
- **Defence-in-depth IP check**: the `reject_non_loopback_ip` helper inside `validate_backend_url`. It is unreachable today (the allowed-domain check rejects non-loopback IPs first) and was kept as a guard against future allowlist edits. After the refactor it remains an internal step of the constructor; we keep it for the same reason.
- **SSL-unless-loopback rule**: today `ConnectAddress::validate` rejects `(broker = mirurobotics-host, protocol = TCP)`. This rule does **not** belong on `MqttHost` (which is host-only). See Decision Log on whether to keep it as a `ConnectAddress::new` constructor or delete it.

## Plan of Work

### Decision required up front (record in Decision Log on M1)

1. **Should `ConnectAddress` get a gated constructor too, or do we delete the SSL-unless-loopback rule?** The current `ConnectAddress::validate` enforces (a) host validity — moves into `MqttHost::new`, and (b) SSL-unless-loopback — has no host-typed home. Two options:
   - (a) Add `ConnectAddress::new(host: MqttHost, protocol: Protocol, port: u16) -> Result<Self, InvalidConnectAddressErr>` that enforces the SSL rule, then keep `InvalidConnectAddressErr` for that single case. `Default` returns the production address (unchanged). All `ConnectAddress { ... }` literals (including in `agent/src/main.rs` line 149) become `ConnectAddress::new(...)?` calls.
   - (b) Delete the rule entirely. The runtime check fires only at startup, and today the only producer of `Protocol` is `Default::default()` which is already `SSL`. No on-disk setting drives `Protocol`. The rule was anticipatory hardening for a not-yet-built feature.
   - **Recommendation:** option (a). It preserves the existing invariant and is a small constructor; deleting it would lose a guard the harden-allowed-domains effort explicitly added (commit `cafa8ef`). Implement (a) unless the orchestrator decides otherwise.
2. **Naming.** Use `BackendUrl` and `MqttHost`. They are short, match repo style (PascalCase, no `Validated` prefix), and sit naturally in `crate::storage::validation`.
3. **Inner representation.** `BackendUrl` wraps `url::Url` (the constructor already parses one). `MqttHost` wraps `String` (host string is what the MQTT crate consumes). Both expose `as_str(&self) -> &str` and a `Display` impl for log output.

### Milestone 1 — Introduce `BackendUrl` and `MqttHost` newtypes

Edit `agent/src/storage/validation.rs`:

- Add at the bottom: `#[derive(Clone, Debug, PartialEq, Eq, Serialize)] pub struct BackendUrl(Url);` and `#[derive(Clone, Debug, PartialEq, Eq, Serialize)] pub struct MqttHost(String);`. Use `serde::Serialize`; `Url` is already `Serialize` via the `serde` feature of the `url` crate, but check `Cargo.toml` — if not enabled, write `Serialize` manually as `serializer.serialize_str(self.as_str())`. (Cheaper to write the manual impl than to flip the feature; do that.)
- Move the body of `validate_backend_url` into `impl BackendUrl { pub fn new(raw: &str) -> Result<Self, String> { ... Ok(Self(url)) } }`. The existing private helpers (`is_allowed_host`, `reject_non_loopback_ip`, `is_loopback_host`) remain and are called from inside `new`.
- Move the body of `validate_mqtt_host` into `impl MqttHost { pub fn new(host: &str) -> Result<Self, String> { ... Ok(Self(host.to_string())) } }`.
- Add `impl BackendUrl { pub fn as_str(&self) -> &str { self.0.as_str() } }` and likewise for `MqttHost`. Add `impl Display`. Add `impl AsRef<str>` only if downstream needs it (decide during M2).
- Add `impl Default for BackendUrl { fn default() -> Self { Self::new("https://api.mirurobotics.com/agent/v1").expect("default backend URL must be valid") } }` and likewise for `MqttHost` with `"mqtt.mirurobotics.com"`. Cover both with unit tests so the `expect` cannot regress silently.
- Add custom `impl<'de> Deserialize<'de> for BackendUrl { ... }`: deserialize as `String`, call `Self::new`, on `Err` return `serde::de::Error::custom(...)`. **Note:** the warn-and-fall-back behavior lives one layer up in `Backend::deserialize` (M2) — the newtype's deserialize is "strict" so a direct `serde_json::from_str::<BackendUrl>(...)` on a bad input fails. Mirror for `MqttHost`.
- **Delete** the now-orphaned free functions `validate_backend_url` and `validate_mqtt_host`. Keep `is_loopback_host` (still used by the worker mqtt code path? — check during M3) and `reject_non_loopback_ip` (called only from inside `BackendUrl::new` now; can be made `fn` instead of `pub(crate) fn`).
- Update the unit test module inside `validation.rs` (currently exercises `reject_non_loopback_ip` directly) — keep as-is, the helper still exists.

Edit `agent/src/storage/mod.rs` line 25: change the re-export from `pub use self::validation::{is_loopback_host, validate_backend_url, validate_mqtt_host};` to `pub use self::validation::{BackendUrl, MqttHost};` plus `is_loopback_host` if it survives M3.

Commit. Suggested message: `feat(storage/validation): introduce BackendUrl and MqttHost newtypes`.

### Milestone 2 — Convert `Backend` and `MQTTBroker` to hold the newtypes

Edit `agent/src/storage/settings.rs`:

- Change `Backend.base_url: String` to `Backend.base_url: BackendUrl`. Update `Default` to use `BackendUrl::default()` instead of `"https://api.mirurobotics.com/agent/v1".to_string()`.
- Change `MQTTBroker.host: String` to `MQTTBroker.host: MqttHost`. Update `Default` similarly.
- Rewrite `Backend::deserialize` and `MQTTBroker::deserialize`. The on-disk shape remains a `String` field, so the inner deserialize struct keeps `base_url: Option<String>` / `host: Option<String>`. After unwrap-or-default, call `BackendUrl::new` / `MqttHost::new`. On `Err`, log `warn!` exactly as today and substitute `BackendUrl::default()` / `MqttHost::default()`. Preserve the existing log message format (`"backend.base_url ` + value + ` rejected ({msg}); falling back to default ` + default + `"`) so log-grepping operators see no diff.
- The `Serialize` derive on `Backend` and `MQTTBroker` continues to work: `BackendUrl: Serialize` produces a string, so the on-disk JSON shape is identical.
- Imports: remove `use crate::storage::validation;`. Add `use crate::storage::validation::{BackendUrl, MqttHost};`.

Update `agent/src/main.rs`:

- Line 55: `let http_client = http::Client::new(&settings.backend.base_url)?;` becomes `let http_client = http::Client::new(settings.backend.base_url.as_str())?;`.
- Line 171: `backend_base_url: settings.backend.base_url,` — `app::options::AppOptions::backend_base_url` is `String` (`agent/src/app/options.rs` line 45). Either (a) keep as `String` and write `settings.backend.base_url.as_str().to_string()`, or (b) change `AppOptions::backend_base_url` to `BackendUrl`. **Recommendation:** (b). It propagates the invariant one layer further. Check `agent/src/app/run.rs` line 166 for the only consumer.
- Line 191: `return settings.backend.base_url;` — function `get_bootstrap_base_url` returns `String`. Change return type to `BackendUrl` and update the `bootstrap_http_client` call at line 110 (`http::Client::new(&url)` becomes `http::Client::new(url.as_str())`). Line 194 already uses `Backend::default().base_url`, which now is a `BackendUrl`.

If the orchestrator opts not to propagate `BackendUrl` into `AppOptions` (option (a) above), that's fine — confine the change to the immediate caller. Decide at M2 start and record in Decision Log.

Commit. Suggested message: `refactor(storage/settings): hold BackendUrl and MqttHost instead of String`.

### Milestone 3 — Update `ConnectAddress` and remove `ConnectAddress::validate`

Edit `agent/src/mqtt/options.rs`:

- Change `ConnectAddress.broker: String` to `ConnectAddress.broker: MqttHost`. Update `Default::default` to `MqttHost::default()`.
- **Add a gated constructor:** `pub fn new(broker: MqttHost, protocol: Protocol, port: u16) -> Result<Self, InvalidConnectAddressErr>`. Body enforces only the SSL-unless-loopback rule (host is already valid by typing). The struct fields remain `pub`? — see below.
- Field visibility: making `broker: MqttHost` `pub` means anyone can build `ConnectAddress { broker, protocol: TCP, port }` and skip `new`. Two options:
  - (i) Keep fields `pub` and accept that the SSL rule is enforced only at the constructor. Document on the type. Acceptable because the rule is a soft preference for the production environment, and tests routinely construct `Protocol::TCP` for loopback brokers.
  - (ii) Make `broker` `pub` (it's a validated newtype, no risk) but make `protocol` and `port` `pub(crate)` and force callers through `new` or `Default`. This affects `agent/tests/mqtt/options.rs`, `client.rs`, `errors.rs` which construct literals.
  - **Recommendation:** (i). Lower-friction for tests; the SSL rule is partial defence-in-depth. Document the rationale at the type definition.
- Delete `ConnectAddress::validate`. Replace its only call site (`agent/src/main.rs` line 153-163) with `ConnectAddress::new(settings.mqtt_broker.host, Protocol::SSL, 8883)` — but `MqttHost` already encodes host validity, so the `Err` here is only the SSL-rule violation, which is impossible when we hardcode `Protocol::SSL`. Therefore the cleaner replacement at line 149 is just `ConnectAddress { broker: settings.mqtt_broker.host, ..Default::default() }` with no validation step at all. The fall-back-to-default block (line 153-163) goes away; `main.rs` is shorter by ~15 lines.
- Drop the `use crate::storage::validation;` import (no longer needed).

Edit `agent/src/mqtt/errors.rs`:

- Keep `InvalidConnectAddressErr` (still produced by `ConnectAddress::new` for the SSL-unless-loopback case). Verify it isn't part of any `impl_error!` aggregate; the existing comment says it isn't, so no edits there.
- If the orchestrator chooses option (b) "delete the SSL rule entirely" from the up-front decision, **also** delete `InvalidConnectAddressErr` and its `impl crate::errors::Error`.

Edit `agent/src/workers/mqtt.rs`: no source change expected — `Options.broker_address: ConnectAddress` continues to work and `&options.broker_address.broker` (used in `agent/src/mqtt/client.rs` line 47) is the `MqttHost`'s `as_str()` via `&MqttHost` coercion. Verify `rumqttc::MqttOptions::new` accepts `&str` (it does); call `.as_str()` if needed.

Commit. Suggested message: `refactor(mqtt): replace ConnectAddress::validate with type-level guarantee on broker host`.

### Milestone 4 — Update `provision::entry::determine_settings` and the CLI flow

Edit `agent/src/provision/entry.rs::determine_settings`:

- Replace the `validate_backend_url(&candidate)` call with `BackendUrl::new(&candidate)`. Map the `String` error to `ProvisionErr::InvalidSettingsErr` exactly as today. Assign the resulting `BackendUrl` to `settings.backend.base_url` (no second conversion).
- Replace the `validate_mqtt_host(mqtt_broker_host)` call with `MqttHost::new(mqtt_broker_host)` and assign similarly.
- Drop `use crate::storage::validation;` if no other reference remains.

Edit `agent/src/main.rs::run_provision` line 55: identical to M2's adjustment (`settings.backend.base_url.as_str()` for `http::Client::new`).

Commit. Suggested message: `refactor(provision): construct BackendUrl/MqttHost directly in determine_settings`.

### Milestone 5 — Tests

`agent/tests/storage/validation.rs`:

- Rename the file's two top-level modules from `validate_backend_url` and `validate_mqtt_host` to `backend_url_new` and `mqtt_host_new`. Replace `validate_backend_url(...)` calls with `BackendUrl::new(...)`. Same for `MqttHost::new`.
- All 21 existing assertions remain, just retargeted. Add two new tests: `BackendUrl::default()` succeeds and equals the documented production URL; `MqttHost::default()` likewise.
- Add tests covering `BackendUrl` / `MqttHost` deserialize with valid and invalid string inputs (confirming the type's own deserialize is strict).

`agent/tests/storage/settings.rs`:

- The four warn-and-fall-back tests (lines 137, 154, 161, 175) continue to pass without source changes; the on-disk JSON shape is unchanged. Verify, do not delete.
- Round-trip tests (`serialize_deserialize_settings`, etc.) — the in-memory struct now holds `BackendUrl`/`MqttHost`. Update the literals (lines 16-21, 31-37, 70-71, 82, 104-105, 115-116) to construct via `BackendUrl::new(...).unwrap()` / `MqttHost::new(...).unwrap()`. The serialized JSON is the same.

`agent/tests/provision/entry.rs::determine_settings`:

- The six existing cases (lines 256-336) continue to pass — they exercise the provisioning entry point's accept/reject behavior, which is unchanged. No source edits needed unless the assertion at line 266 (`settings.backend.base_url == "..."`) trips on the type change — adapt to `settings.backend.base_url.as_str() == "..."`.

`agent/tests/mqtt/options.rs`:

- The `mod connect_address` `default` test (line 12-16) continues to pass; the broker assertion becomes `assert_eq!(addr.broker.as_str(), "mqtt.mirurobotics.com")`.
- **Delete** the entire `mod validate` (lines 19-116). The host-validity cases moved to `mqtt_host_new`; the SSL-unless-loopback cases move into a new small `mod connect_address_new` if option (a) is taken (covering only `accepts_loopback_tcp`, `accepts_allowed_host_ssl`, `rejects_allowed_host_tcp`, `accepts_loopback_ssl`). If option (b) is taken, no replacement is needed.
- The `mod opts` tests (lines 145-228) construct `ConnectAddress` literals with `broker: "local".to_string()`. Update each occurrence to `broker: MqttHost::new("localhost").unwrap()` (or another loopback / allowed value as appropriate). Same for any existing test in `agent/tests/mqtt/client.rs` and `agent/tests/mqtt/errors.rs` that builds a `ConnectAddress` literal.

`agent/src/storage/validation.rs` — the inline `mod tests::reject_non_loopback_ip` (lines 89-134) keeps working because `reject_non_loopback_ip` survives. Verify; no edits expected.

Commit. Suggested message: `test(allowed-domains-newtypes): retarget validator tests onto type constructors`.

### Milestone 6 — Preflight

From `/home/ben/miru/workbench2/repos/agent/`, run `./scripts/preflight.sh`. Iterate on any lint/test/coverage failure until the script prints `Preflight clean`. Verify the four module gates in particular: `agent/src/storage/.covgate` = 94.21, `agent/src/provision/.covgate` = 95.66, `agent/src/mqtt/.covgate` = 96.18, `agent/src/app/.covgate` = 88.62. The refactor reduces lines (deleted `ConnectAddress::validate`, deleted free-function validators, deleted main.rs fall-back block) and should not reduce coverage; if it does, add tests to compensate before adjusting `.covgate`.

Commit. Suggested message: `chore: preflight clean for allowed-domains-newtypes refactor`.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent/` unless otherwise noted.

1. Read `agent/src/storage/validation.rs`, `agent/src/storage/settings.rs`, `agent/src/mqtt/options.rs`, `agent/src/mqtt/errors.rs`, `agent/src/provision/entry.rs`, `agent/src/main.rs`, `agent/src/app/options.rs`, `agent/src/workers/mqtt.rs`.
2. Confirm the up-front decisions (option (a) vs (b) for `ConnectAddress::new`; whether to propagate `BackendUrl` into `AppOptions`). Record in the Decision Log.
3. M1: edit `agent/src/storage/validation.rs` and `agent/src/storage/mod.rs`. Run `cargo check --package miru-agent --features test` to confirm the type compiles in isolation. Expected: clean build (no warnings on the new types).
4. M1 commit: `git add agent/src/storage/validation.rs agent/src/storage/mod.rs && git commit` with the message above.
5. M2: edit `agent/src/storage/settings.rs`, `agent/src/main.rs`, `agent/src/app/options.rs`, `agent/src/app/run.rs`. Run `cargo check --package miru-agent --features test` again. Expect compilation errors at usage sites — fix until clean.
6. M2 commit: `git add` the listed files and commit.
7. M3: edit `agent/src/mqtt/options.rs`, `agent/src/mqtt/errors.rs`, `agent/src/main.rs`. Run `cargo check`. Expect compile errors in tests touching `ConnectAddress`; do NOT fix yet (M5 covers tests).
8. M3 commit.
9. M4: edit `agent/src/provision/entry.rs`. Run `cargo check`. Should compile clean.
10. M4 commit.
11. M5: update tests in `agent/tests/storage/validation.rs`, `agent/tests/storage/settings.rs`, `agent/tests/mqtt/options.rs`, `agent/tests/mqtt/client.rs`, `agent/tests/mqtt/errors.rs`, `agent/tests/provision/entry.rs`. Run `./scripts/test.sh`. Expect: all tests pass.
12. M5 commit.
13. M6: run `./scripts/preflight.sh`. On any failure, fix and re-run. Expected end-state output: `Preflight clean`.
14. M6 commit (only if preflight required additional touch-ups; otherwise skip).
15. The branch is now ready for review on PR #49 (`feat/harden-allowed-domains`). Do not push until the orchestrator approves.

## Validation and Acceptance

Behavior the user can verify:

- **`miru-agent --provision --backend-host=https://attacker.com ...` exits non-zero** with a message containing `attacker.com`. Same as before; preserved by `determine_settings` propagating the constructor's `Err`.
- **`miru-agent --provision --mqtt-broker-host=evilmirurobotics.com ...` exits non-zero** with a message containing `evilmirurobotics.com`. Same as before.
- **`miru-agent --provision --backend-host=https://api.mirurobotics.com ...`** succeeds and writes a `settings.json` with the validated URL. Same as before.
- **A tampered on-disk `settings.json`** with `"base_url": "https://evilmirurobotics.com"` does not crash the daemon; instead `WARN backend.base_url ... rejected; falling back to default ...` appears in the log and the agent reaches `https://api.mirurobotics.com/agent/v1`. Same as before.
- **Type-level guarantee:** `let bad = Backend { base_url: "https://attacker.com".to_string() };` no longer compiles (the field is `BackendUrl`, not `String`). This is the new property. Tested by adding a `#[test]` that calls `BackendUrl::new("https://attacker.com")` and asserts `Err` — establishes that the only way into the type goes through validation.

Test commands (from `/home/ben/miru/workbench2/repos/agent/`):

- `./scripts/test.sh` — expect all tests pass. Test counts: validation tests ~21 (renamed), settings deserialize tests 11 (unchanged), provision determine-settings tests 6 (unchanged), mqtt connect-address tests reduced from ~8 to ~4 (or 0 in option (b)), plus new `BackendUrl::default` and `MqttHost::default` smoke tests.
- `./scripts/lint.sh` — clean (clippy `-D warnings`).
- `./scripts/covgate.sh` — coverage thresholds for `storage`, `provision`, `mqtt`, `app` all hold. If any drops, add tests rather than lower the gate.
- `./scripts/preflight.sh` — final gate; output ends with `Preflight clean`.

Acceptance: preflight clean, behavior diffs above hold, the four warn-and-fall-back tests in `agent/tests/storage/settings.rs` (matching the `19ee1ac` decision) still pass without modification of their source-side semantics.

## Idempotence and Recovery

- All edits are local to the working tree. Re-running steps is safe: `cargo check` and `./scripts/test.sh` are idempotent. If a milestone is half-applied and you need to roll back, `git restore --source=HEAD --staged --worktree` on the milestone's listed files returns to the previous good commit.
- Milestone commits are independent and bisectable. If preflight fails in M6, the failure usually points to a missed test update; iterate without rolling back the source-side commits.
- The newtype `Default` impls use `expect(...)`. If the default constants regress and the panic fires at startup, the agent crashes immediately rather than silently using a bogus value — this is intentional. Cover both defaults with unit tests so a regression is caught at `cargo test`, not at runtime.
- Coverage gate constraint: do not lower `.covgate` thresholds. If the refactor mechanically reduces a module's coverage, add tests until the existing gate holds. The gates exist precisely to prevent silent erosion during refactors.
