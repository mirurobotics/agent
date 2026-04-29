# Harden allowed domains for backend base URL and MQTT broker host

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, base branch `main`) | read-write | Source, tests, plan file, `Cargo.toml` updates. |

This plan lives in `plans/backlog/` of the agent repo because all code changes are in this repo.

## Purpose / Big Picture

After this change, the agent refuses to talk to backend or MQTT servers outside the `mirurobotics.com` family (with a loopback exception for local development). Today both endpoints are user-controlled at provisioning time and through the on-disk settings file with no validation, so a stolen settings file or a tampered provisioning command can silently redirect a device to an attacker-controlled host. After the change, an operator who runs `miru-agent --provision --backend-host=https://attacker.com ...` sees a clear error and exits non-zero, and an attacker who edits `settings.json` to contain `"base_url": "https://evilmirurobotics.com"` causes the agent to fail to read the file rather than connect to the wrong host. The validation rule applies symmetrically at both entry points (CLI and disk) and is implemented once in a shared helper.

## Progress

- [ ] M1: Add the `url` crate to the workspace and to `agent/Cargo.toml`.
- [ ] M2: Implement the shared validation module `agent/src/storage/validation.rs` with backend-URL and MQTT-host validators.
- [ ] M3: Wire validation into `provision::entry::determine_settings`; surface errors via a new `ProvisionErr::InvalidSettingsErr` variant.
- [ ] M4: Wire validation into the `Backend` and `MQTTBroker` `Deserialize` impls in `agent/src/storage/settings.rs` so a tampered settings file fails to deserialize.
- [ ] M5: Add `ConnectAddress::validate` to enforce the `Protocol::SSL`-unless-loopback rule, called from `agent/src/main.rs::run_agent` before constructing `MqttOptions`.
- [ ] M6: Run preflight; iterate until it reports `Preflight clean`.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

(Add entries as you go.)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Repo conventions (from `agent/AGENTS.md` and `agent/CLAUDE.md`):

- Tests: `./scripts/test.sh` (runs `RUST_LOG=off cargo test --features test`). The `test` feature is required.
- Lint: `./scripts/lint.sh` (custom import linter, `cargo fmt`, machete, audit, clippy with `-D warnings`).
- Preflight: `./scripts/preflight.sh` runs lint + tests + tools-lint + tools-tests in parallel; success prints `Preflight clean`.
- Coverage gate: `./scripts/covgate.sh` enforces per-module thresholds in `.covgate` files. Relevant gates:
  - `agent/src/storage/.covgate` = `94.21`
  - `agent/src/provision/.covgate` = `95.66`
  - `agent/src/mqtt/.covgate` = `96.18`
- Source files use a fixed import order: `// standard crates`, `// internal crates`, `// external crates`, separated by blank lines (configured in `.lint-imports.toml`).
- Errors: each error type derives `thiserror::Error` and implements `crate::errors::Error`; aggregating enums use the `impl_error!` macro (in `agent/src/errors/mod.rs`). New error types go in the module's `errors.rs`.

Key files (full paths from repo root `/home/ben/miru/workbench2/repos/agent/`):

- `agent/src/cli/mod.rs` — `ProvisionArgs { backend_host, mqtt_broker_host, device_name }`. Today `parse` simply records strings; values flow into `provision::determine_settings`.
- `agent/src/provision/entry.rs` lines 90-99 — `pub fn determine_settings(args: &cli::ProvisionArgs) -> settings::Settings`. Currently infallible. It rewrites `settings.backend.base_url = format!("{}/agent/v1", backend_host)` and `settings.mqtt_broker.host = mqtt_broker_host.to_string()`.
- `agent/src/provision/errors.rs` — `pub enum ProvisionErr { MissingEnvVarErr, AuthnErr, CryptErr, FileSysErr, HTTPErr, LogsErr, StorageErr }`. Aggregated via `impl_error!`. We will add `InvalidSettingsErr`.
- `agent/src/main.rs` — `run_provision` calls `provision::determine_settings(&args)` (line 54). Today the signature returns `Settings`; after this change it returns `Result<Settings, ProvisionErr>` and `run_provision` will `?` it. `run_agent` builds `ConnectAddress { broker: settings.mqtt_broker.host, ..Default::default() }` at line 155 — the protocol comes from `Default` (`Protocol::SSL`), but a future refactor that lets settings drive protocol is what `ConnectAddress::validate` exists to guard.
- `agent/src/storage/settings.rs` — manual `Deserialize` impls for `Settings`, `Backend`, `MQTTBroker`. The current pattern uses `deserialize_warn!(...)` to log a warning and substitute defaults when a field is missing. We change behaviour so an invalid (non-empty but rule-violating) value hard-errors via `serde::de::Error::custom`; missing fields still fall back to defaults.
- `agent/src/storage/mod.rs` — re-exports `Backend, MQTTBroker, Settings`. We will add `pub mod validation;` and re-export the validator functions for unit-testability.
- `agent/src/mqtt/options.rs` — `enum Protocol { TCP, SSL }` and `struct ConnectAddress { protocol, broker, port }`. We add `ConnectAddress::validate(&self) -> Result<(), InvalidConnectAddressErr>` that delegates host validation to the shared helper and additionally enforces SSL-unless-loopback. The new `InvalidConnectAddressErr` lives in the existing `agent/src/mqtt/errors.rs`.
- `agent/src/mqtt/client.rs` line 54 — `match options.connect_address.protocol { ... }` is the only consumer of `Protocol`. The validation gate prevents constructing an `MqttOptions` whose `ConnectAddress` violates the rule.
- `agent/Cargo.toml` — currently does not depend on `url`. The `url` crate (v2.5.x) is already in `Cargo.lock` transitively but not declared. We add it as a workspace dependency in the root `Cargo.toml` and as a `{ workspace = true }` entry in `agent/Cargo.toml`.
- `agent/tests/storage/settings.rs`, `agent/tests/provision/entry.rs`, `agent/tests/mqtt/options.rs`, `agent/tests/cli/mod.rs` — existing test files; we extend them with the cases listed in **Validation and Acceptance**.

Definitions:

- **Backend base URL**: the URL the agent uses for HTTPS calls to the Miru control plane. Stored as `settings.backend.base_url` (a `String`). Default: `https://api.mirurobotics.com/agent/v1`.
- **MQTT broker host**: a bare hostname (not a URL) used to open a TCP/TLS connection to an MQTT broker. Stored as `settings.mqtt_broker.host` (a `String`). Default: `mqtt.mirurobotics.com`. The protocol/port live in `ConnectAddress::default()` as `Protocol::SSL` / `8883`.
- **Loopback host**: any of the literal strings `localhost`, `127.0.0.1`, `::1`. Note `::1` is the canonical form returned by `url::Host::Ipv6(Ipv6Addr::LOCALHOST).to_string()`; our string comparison must accept it.
- **Allowed-domain suffix rule**: a host is allowed iff it is exactly `mirurobotics.com` OR ends with the literal suffix `.mirurobotics.com` (note the leading dot, which prevents `evilmirurobotics.com` from matching).
- **Userinfo**: the `user:password@` part of a URL. We require `url.username().is_empty()` and `url.password().is_none()` to defeat the `https://attacker.com@api.mirurobotics.com` confusion attack (where `attacker.com` is the userinfo and `api.mirurobotics.com` is the host).

## Plan of Work

### Milestone 1 — Add the `url` crate as a workspace dependency

In the root `/home/ben/miru/workbench2/repos/agent/Cargo.toml`, add to the `[workspace.dependencies]` block (alphabetical order, after `tracing-subscriber`):

    url = "2.5.8"

In `/home/ben/miru/workbench2/repos/agent/agent/Cargo.toml`, add to `[dependencies]` (alphabetical position after `uuid`):

    url = { workspace = true }

### Milestone 2 — Shared validation helper

Create `agent/src/storage/validation.rs` with this surface:

    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use url::Url;

    const ALLOWED_DOMAIN: &str = "mirurobotics.com";
    const ALLOWED_DOMAIN_SUFFIX: &str = ".mirurobotics.com";

    pub fn is_loopback_host(host: &str) -> bool {
        matches!(host, "localhost" | "127.0.0.1" | "::1")
    }

    fn is_allowed_domain(host: &str) -> bool {
        host == ALLOWED_DOMAIN || host.ends_with(ALLOWED_DOMAIN_SUFFIX)
    }

    /// Validates a backend base URL. Returns the parsed `Url` on success.
    pub fn validate_backend_url(raw: &str) -> Result<Url, String> {
        let url = Url::parse(raw).map_err(|e| format!("invalid URL: {e}"))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err("URL must not contain userinfo".into());
        }
        let host = url
            .host_str()
            .ok_or_else(|| "URL must contain a host".to_string())?;
        // host_str() preserves IPv6 brackets; strip them so the loopback
        // literal "::1" matches our string set.
        let bare_host = host.trim_start_matches('[').trim_end_matches(']');
        let loopback = is_loopback_host(bare_host);
        match (url.scheme(), loopback) {
            ("https", _) => {}
            ("http", true) => {}
            ("http", false) => return Err("non-loopback URL must use https".into()),
            (other, _) => return Err(format!("scheme `{other}` not allowed")),
        }
        if !loopback && !is_allowed_domain(bare_host) {
            return Err(format!("host `{bare_host}` is not allowed"));
        }
        // Defence in depth: parsed-IP rejection for any non-loopback IP that
        // somehow slipped past the suffix check (e.g. future allowlist edits).
        if let Ok(ip) = bare_host.parse::<IpAddr>() {
            let is_loopback_ip = matches!(ip, IpAddr::V4(v4) if v4 == Ipv4Addr::LOCALHOST)
                || matches!(ip, IpAddr::V6(v6) if v6 == Ipv6Addr::LOCALHOST);
            if !is_loopback_ip {
                return Err(format!("IP host `{ip}` is not allowed"));
            }
        }
        Ok(url)
    }

    /// Validates a bare MQTT broker hostname.
    pub fn validate_mqtt_host(host: &str) -> Result<(), String> {
        if is_loopback_host(host) || is_allowed_domain(host) {
            Ok(())
        } else {
            Err(format!("MQTT host `{host}` is not allowed"))
        }
    }

Add `pub mod validation;` to `agent/src/storage/mod.rs` and a re-export `pub use self::validation::{validate_backend_url, validate_mqtt_host, is_loopback_host};`.

The helper deliberately returns `String` errors so it stays thin and serde-friendly. Both call sites (provisioning, deserialization) wrap the returned message in their own typed error.

### Milestone 3 — Provisioning entry point validation

Edit `agent/src/provision/errors.rs`:

- Add a new error type:

      #[derive(Debug, thiserror::Error)]
      #[error("invalid settings: {msg}")]
      pub struct InvalidSettingsErr {
          pub msg: String,
          pub trace: Box<Trace>,
      }

      impl crate::errors::Error for InvalidSettingsErr {}

- Add `InvalidSettingsErr(InvalidSettingsErr)` to `pub enum ProvisionErr`.
- Add `From<InvalidSettingsErr> for ProvisionErr` and the variant to the `impl_error!` macro list.

Edit `agent/src/provision/entry.rs::determine_settings`:

- Change signature to `pub fn determine_settings(args: &cli::ProvisionArgs) -> Result<settings::Settings, ProvisionErr>`.
- After applying each override, call the shared helper. The full body becomes:

      pub fn determine_settings(args: &cli::ProvisionArgs) -> Result<settings::Settings, ProvisionErr> {
          let mut settings = settings::Settings::default();
          if let Some(backend_host) = &args.backend_host {
              let candidate = format!("{}/agent/v1", backend_host);
              storage::validation::validate_backend_url(&candidate).map_err(|msg| {
                  ProvisionErr::InvalidSettingsErr(InvalidSettingsErr {
                      msg: format!("backend.base_url `{candidate}`: {msg}"),
                      trace: crate::trace!(),
                  })
              })?;
              settings.backend.base_url = candidate;
          }
          if let Some(mqtt_broker_host) = &args.mqtt_broker_host {
              storage::validation::validate_mqtt_host(mqtt_broker_host).map_err(|msg| {
                  ProvisionErr::InvalidSettingsErr(InvalidSettingsErr {
                      msg: format!("mqtt_broker.host `{mqtt_broker_host}`: {msg}"),
                      trace: crate::trace!(),
                  })
              })?;
              settings.mqtt_broker.host = mqtt_broker_host.to_string();
          }
          Ok(settings)
      }

Edit `agent/src/main.rs::run_provision` line 54: `let settings = provision::determine_settings(&args)?;`. The function already returns `Result<_, ProvisionErr>` so `?` works.

Update existing tests in the inline `mod determine_settings` block (lines 153-193 of `agent/src/provision/entry.rs`) to call `.unwrap()` on the new result and use allowed values (e.g. `https://custom.mirurobotics.com` instead of `https://custom.example.com`).

### Milestone 4 — Settings deserialization validation

Edit `agent/src/storage/settings.rs`:

- In `impl<'de> Deserialize<'de> for Backend`, after the existing `result.base_url.unwrap_or_else(|| deserialize_warn!(...))` line, validate the resolved value and hard-error on failure:

      let base_url = result
          .base_url
          .unwrap_or_else(|| deserialize_warn!("backend", "base_url", default.base_url));
      crate::storage::validation::validate_backend_url(&base_url)
          .map_err(|msg| serde::de::Error::custom(format!("backend.base_url: {msg}")))?;
      Ok(Backend { base_url })

- In `impl<'de> Deserialize<'de> for MQTTBroker`, do the analogous thing with `validate_mqtt_host`.

This means the default value is still trusted (it's hard-coded and already passes), missing fields still fall back to default with a warning, but a present-but-invalid value causes serde to return an error. The `error!` log already in place fires on the way out.

Add `use crate::storage::validation;` to the file's `// internal crates` block.

### Milestone 5 — `ConnectAddress::validate` and runtime gate

Add to the existing `agent/src/mqtt/errors.rs`:

    #[derive(Debug, thiserror::Error)]
    #[error("invalid mqtt connect address: {msg}")]
    pub struct InvalidConnectAddressErr {
        pub msg: String,
        pub trace: Box<crate::errors::Trace>,
    }

    impl crate::errors::Error for InvalidConnectAddressErr {}

Then in `agent/src/mqtt/options.rs`:

    impl ConnectAddress {
        pub fn validate(&self) -> Result<(), crate::mqtt::errors::InvalidConnectAddressErr> {
            use crate::mqtt::errors::InvalidConnectAddressErr;
            use crate::storage::validation;
            let loopback = validation::is_loopback_host(&self.broker);
            validation::validate_mqtt_host(&self.broker).map_err(|msg| {
                InvalidConnectAddressErr {
                    msg: format!("broker `{}`: {msg}", self.broker),
                    trace: crate::trace!(),
                }
            })?;
            if !loopback && !matches!(self.protocol, Protocol::SSL) {
                return Err(InvalidConnectAddressErr {
                    msg: format!(
                        "non-loopback broker `{}` requires Protocol::SSL",
                        self.broker
                    ),
                    trace: crate::trace!(),
                });
            }
            Ok(())
        }
    }

In `agent/src/main.rs::run_agent`, before constructing `AppOptions`, call `validate()` on the address. Concretely, replace the `mqtt_worker: mqtt::Options { broker_address: ConnectAddress { broker: settings.mqtt_broker.host, ..Default::default() }, ..Default::default() }` block with one that builds the `ConnectAddress` first, then calls `.validate()` and aborts with an `error!` log + `return` on failure (matching the existing failure-handling style elsewhere in `run_agent`):

    let broker_address = ConnectAddress {
        broker: settings.mqtt_broker.host,
        ..Default::default()
    };
    if let Err(e) = broker_address.validate() {
        error!("Invalid MQTT connect address: {e}");
        return;
    }

Then use `broker_address` inside `AppOptions { mqtt_worker: mqtt::Options { broker_address, ..Default::default() }, ... }`.

### Milestone 6 — Preflight

Run `./scripts/preflight.sh` and iterate until the final line is `Preflight clean`. Common follow-ups:

- Coverage shortfall under `agent/src/storage` or `agent/src/provision` or `agent/src/mqtt`. Add tests rather than lowering thresholds in the `.covgate` files.
- Lint failures from new imports — re-order per `.lint-imports.toml` rules.

## Concrete Steps

All commands run from the repo root unless stated: `cd /home/ben/miru/workbench2/repos/agent`.

### M1: Add the `url` crate

1. Edit `Cargo.toml` (workspace root): insert `url = "2.5.8"` in `[workspace.dependencies]` between `tracing-subscriber` and `users`.
2. Edit `agent/Cargo.toml`: insert `url = { workspace = true }` in `[dependencies]` after `uuid`.
3. Refresh the lockfile and confirm the build still resolves:

       ./scripts/update-deps.sh
       cargo build --features test --package miru-agent

   Expect a clean build. The `url` crate is already in `Cargo.lock` transitively, so this should be a no-op for resolution.

4. Commit:

       git add Cargo.toml agent/Cargo.toml Cargo.lock
       git commit -m "build(agent): declare url crate as workspace dependency"

### M2: Shared validation module

1. Create `agent/src/storage/validation.rs` with the body shown in **Plan of Work §M2**. Use the import order:

       // standard crates
       use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

       // external crates
       use url::Url;

2. Edit `agent/src/storage/mod.rs`: add `pub mod validation;` next to the other `pub mod` lines (alphabetical between `setup` and existing entries) and add `pub use self::validation::{is_loopback_host, validate_backend_url, validate_mqtt_host};` in the existing `pub use` block.

3. Create `agent/tests/storage/validation.rs` with the cases listed in **Validation and Acceptance §validation tests**. Add `pub mod validation;` to `agent/tests/storage/mod.rs`.

4. Run focused tests:

       CARGO_TEST_ARGS="storage::validation::" ./scripts/test.sh

   Expect all new tests to pass.

5. Commit:

       git add agent/src/storage/validation.rs agent/src/storage/mod.rs agent/tests/storage/validation.rs agent/tests/storage/mod.rs
       git commit -m "feat(storage): add shared validation helpers for backend URL and mqtt host"

### M3: Provisioning entry point

1. Edit `agent/src/provision/errors.rs` to add `InvalidSettingsErr`, the `From` impl, and the `impl_error!` entry per **Plan of Work §M3**.
2. Edit `agent/src/provision/entry.rs::determine_settings` to the new signature and body per **Plan of Work §M3**. Update the inline `mod determine_settings` tests at lines 153-193:
   - In `backend_host_appends_agent_v1_suffix`, change the host to `https://custom.mirurobotics.com` (allowed) and adjust the `assert_eq!`.
   - In `mqtt_broker_host_override`, change the host to `mqtt.custom.mirurobotics.com`.
   - In `no_overrides_preserves_defaults`, call `determine_settings(&args).unwrap()`.
3. Edit `agent/src/main.rs::run_provision` line 54 to `let settings = provision::determine_settings(&args)?;`.
4. Extend `agent/tests/provision/entry.rs` with a new `mod determine_settings { ... }` block covering the reject cases listed in **Validation and Acceptance §provisioning tests**.
5. Run:

       CARGO_TEST_ARGS="provision::" ./scripts/test.sh

   Expect all provision tests to pass.

6. Commit:

       git add agent/src/provision/ agent/src/main.rs agent/tests/provision/entry.rs
       git commit -m "feat(provision): validate backend and mqtt overrides at provisioning time"

### M4: Settings deserialization

1. Edit `agent/src/storage/settings.rs`:
   - Add `use crate::storage::validation;` to the `// internal crates` group.
   - Update the `Backend` `Deserialize` impl per **Plan of Work §M4**.
   - Update the `MQTTBroker` `Deserialize` impl per **Plan of Work §M4**.
2. Extend `agent/tests/storage/settings.rs` with the deserialization-reject cases listed in **Validation and Acceptance §settings tests**. Confirm the existing `serialize_deserialize_*` tests still pass — they use `mqtt.arglebargle.com` and `arglebargle.com`-suffix hosts which violate the new rule, so they must be updated to allowed values (e.g. `mqtt.staging.mirurobotics.com`, `https://staging.mirurobotics.com/agent/v1`). Note this change in the Decision Log.
3. Run:

       CARGO_TEST_ARGS="storage::settings::" ./scripts/test.sh

4. Commit:

       git add agent/src/storage/settings.rs agent/tests/storage/settings.rs
       git commit -m "feat(storage): hard-fail settings deserialization on disallowed host"

### M5: `ConnectAddress::validate` and runtime gate

1. Append the `InvalidConnectAddressErr` definition to `agent/src/mqtt/errors.rs` per **Plan of Work §M5**.
2. Edit `agent/src/mqtt/options.rs` to add the `impl ConnectAddress { pub fn validate ... }` block per **Plan of Work §M5**.
3. Edit `agent/src/main.rs::run_agent` to construct `broker_address` separately, call `.validate()`, and abort with `error!` + `return` on failure per **Plan of Work §M5**.
4. Extend `agent/tests/mqtt/options.rs` with a new `mod validate { ... }` block covering the cases in **Validation and Acceptance §mqtt tests**.
5. Run:

       CARGO_TEST_ARGS="mqtt::options::" ./scripts/test.sh

6. Commit:

       git add agent/src/mqtt/ agent/src/main.rs agent/tests/mqtt/options.rs
       git commit -m "feat(mqtt): require SSL for non-loopback brokers via ConnectAddress::validate"

### M6: Preflight

1. From repo root:

       ./scripts/preflight.sh

   Expect final line `Preflight clean`. **The PR is not opened unless preflight reports `clean`.**

2. If covgate fails, add tests in the affected module (do not lower thresholds). If clippy/fmt fails, fix in place and re-run preflight.

3. Final commit (only if step 1 produced fix-ups):

       git add -A
       git commit -m "chore: address preflight findings for domain-allowlist hardening"

## Validation and Acceptance

Acceptance is verifiable by behavior:

1. `cargo build --features test --package miru-agent` exits 0.
2. `./scripts/test.sh` reports all tests passing.
3. `./scripts/preflight.sh` final line is **`Preflight clean`**. This is a hard gate; the PR must not be opened until preflight is clean.

### Validation tests (`agent/tests/storage/validation.rs`)

Add test modules `mod validate_backend_url` and `mod validate_mqtt_host`.

Backend URL **accept** (each `validate_backend_url(...).is_ok()`):

- `https://api.mirurobotics.com/agent/v1`
- `https://staging.mirurobotics.com/x`
- `http://localhost:8080`
- `http://127.0.0.1:8080`

Backend URL **reject** (each `validate_backend_url(...).is_err()`):

- `http://api.mirurobotics.com` — non-loopback http
- `https://evilmirurobotics.com` — suffix-rule defeat (no leading dot)
- `https://api.mirurobotics.com.attacker.com` — suffix-rule defeat (suffix is at the wrong end)
- `https://user:pass@api.mirurobotics.com` — has userinfo
- `https://attacker.com@api.mirurobotics.com` — userinfo confusion (parses as host=`api.mirurobotics.com`, username=`attacker.com`)
- `https://192.168.1.1` — RFC1918, non-loopback IP
- `ftp://api.mirurobotics.com` — disallowed scheme

MQTT host **accept**:

- `mqtt.mirurobotics.com`
- `localhost`
- `127.0.0.1`
- `::1`
- `mirurobotics.com` — exact match

MQTT host **reject**:

- `evilmirurobotics.com`
- `mqtt.mirurobotics.com.attacker.com`
- `192.168.1.1`

### Provisioning tests (`agent/tests/provision/entry.rs`, new module)

- `accepts_allowed_backend_host` — `backend_host = Some("https://api.mirurobotics.com")` → `Ok` and `settings.backend.base_url == "https://api.mirurobotics.com/agent/v1"`.
- `rejects_disallowed_backend_host` — `backend_host = Some("https://evilmirurobotics.com")` → `Err(ProvisionErr::InvalidSettingsErr(_))`, and the message contains `evilmirurobotics.com`.
- `rejects_http_non_loopback_backend_host` — `backend_host = Some("http://api.mirurobotics.com")` → `Err`.
- `accepts_loopback_backend_host` — `backend_host = Some("http://localhost:8080")` → `Ok`.
- `accepts_allowed_mqtt_host` — `mqtt_broker_host = Some("mqtt.mirurobotics.com")` → `Ok`.
- `rejects_disallowed_mqtt_host` — `mqtt_broker_host = Some("evilmirurobotics.com")` → `Err(ProvisionErr::InvalidSettingsErr(_))`.

### Settings tests (`agent/tests/storage/settings.rs`, extend)

- `deserialize_backend_rejects_disallowed_host` — `serde_json::from_value::<Backend>(json!({"base_url": "https://evilmirurobotics.com"}))` returns `Err`. The error message contains `evilmirurobotics.com`.
- `deserialize_backend_accepts_allowed_host` — `https://api.mirurobotics.com/agent/v1` deserializes successfully.
- `deserialize_backend_rejects_http_non_loopback` — `http://api.mirurobotics.com` returns `Err`.
- `deserialize_mqtt_broker_rejects_disallowed_host` — `serde_json::from_value::<MQTTBroker>(json!({"host": "evilmirurobotics.com"}))` returns `Err`.
- `deserialize_mqtt_broker_accepts_allowed_host` — `mqtt.mirurobotics.com` deserializes successfully.
- `deserialize_settings_with_invalid_backend_url_fails` — full settings JSON with `backend.base_url = "https://evilmirurobotics.com"` returns `Err` (does **not** silently fall back to default).
- Update existing `serialize_deserialize_*` tests to use `mqtt.staging.mirurobotics.com` / `https://staging.mirurobotics.com/agent/v1` so they pass under the new rule.

### MQTT tests (`agent/tests/mqtt/options.rs`, new `mod validate`)

- `accepts_default` — `ConnectAddress::default().validate().is_ok()`.
- `accepts_loopback_tcp` — `ConnectAddress { protocol: Protocol::TCP, broker: "localhost".into(), port: 1883 }.validate().is_ok()`.
- `accepts_loopback_ssl` — `ConnectAddress { protocol: Protocol::SSL, broker: "127.0.0.1".into(), port: 8883 }.validate().is_ok()`.
- `accepts_allowed_host_ssl` — `ConnectAddress { protocol: Protocol::SSL, broker: "mqtt.mirurobotics.com".into(), port: 8883 }.validate().is_ok()`.
- `rejects_allowed_host_tcp` — `ConnectAddress { protocol: Protocol::TCP, broker: "mqtt.mirurobotics.com".into(), port: 1883 }.validate().is_err()` (non-loopback requires SSL).
- `rejects_disallowed_host_ssl` — `ConnectAddress { protocol: Protocol::SSL, broker: "evilmirurobotics.com".into(), port: 8883 }.validate().is_err()`.
- `rejects_suffix_attack` — broker `"mqtt.mirurobotics.com.attacker.com"` fails.
- `rejects_private_ip` — broker `"192.168.1.1"` fails.

### Pre/post behaviour check

- **Pre-change**: `cargo run --bin miru-agent -- --provision --backend-host=https://attacker.com --mqtt-broker-host=evil.com` succeeds (writes a settings file pointing at attacker hosts). After the change, the same command exits non-zero with a message naming the offending value.
- **Pre-change**: tampering `~/.miru-agent/settings.json` to set `"base_url": "https://evilmirurobotics.com"` and starting the agent silently uses that URL. After the change, `settings_file.read_json::<Settings>()` returns `Err`; the existing `error!("Unable to read settings file: {}", e)` in `agent/src/main.rs::run_agent` line 134 surfaces the failure and the agent exits.

## Idempotence and Recovery

- All edits are pure source/test changes; rerunning steps re-edits the same content, which is safe.
- Each milestone ends with a commit, so reverting any milestone is one `git revert <sha>`.
- M1 (`url` crate addition) is additive; if the rest of the work is rolled back, the `url` declaration is unused and `cargo machete` will flag it. In that case, also revert the `Cargo.toml` edits.
- M3 changes a public function signature (`determine_settings` returns `Result`). The single caller (`agent/src/main.rs::run_provision`) is updated in the same milestone, so the tree never sits in a broken intermediate state across commits.
- M5 introduces `ConnectAddress::validate` as an additive method. Failure to call it from `run_agent` is a behavioural regression but not a compile failure — keep the test that constructs an invalid `ConnectAddress` and asserts `validate()` errors so the contract is enforced.
- If preflight (M6) flags a coverage shortfall, add tests rather than lowering the `.covgate` threshold.
