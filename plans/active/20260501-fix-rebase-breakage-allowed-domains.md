# Fix rebase breakage on feat/harden-allowed-domains-rebase

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Fix compile errors in `agent/src/provisioning/shared.rs` and `agent/src/provisioning/reprovision.rs` introduced when rebasing `feat/harden-allowed-domains-rebase` onto `main`. Adjust `agent/src/main.rs` so the provision call site matches the bare-`Settings` signature. |

This plan lives in `agent/plans/` because all changes are inside the `agent` repo (cloned at `/home/ben/miru/workbench2/repos/agent`).

## Purpose / Big Picture

The branch `feat/harden-allowed-domains-rebase` was rebased onto the latest `main` (which now contains the `BackendUrl` / `MqttHost` newtypes and `pub mod network;`). A partial manual fix-up left `agent/src/provisioning/shared.rs` calling the new `BackendUrl::new` / `MqttHost::new` constructors with the wrong argument types and ignoring their `Result` return values, and left `agent/src/provisioning/reprovision.rs` tests comparing the new newtypes against bare `&str` literals. The build is broken (4 errors in `shared.rs`, plus 2 more in `reprovision.rs` tests when checked with `--all-targets`).

After this change, `cargo check --package miru-agent --features test --all-targets` and `bash scripts/preflight.sh` complete cleanly. The provision and reprovision commands continue to honour the existing CLI overrides (`--backend-host=…`, `--mqtt-broker-host=…`); when those overrides fail allowed-domain validation, the agent logs a warning and falls back to the production default for that field, matching the "fall back, don't crash" pattern established in commit `57e0080 refactor(domain-validation): fall back on invalid settings, don't crash`.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Milestone 1: fix `agent/src/provisioning/shared.rs` so it compiles cleanly with the new newtype constructors and falls back to defaults on invalid CLI overrides.
- [ ] Milestone 2: fix `agent/src/provisioning/reprovision.rs` test comparisons that still target bare `&str`.
- [ ] Milestone 3: drop the spurious `?` on the `provision::determine_settings(&args)` call in `agent/src/main.rs:60` so it matches the bare-`Settings` signature.
- [ ] Milestone 4: run `cargo check --package miru-agent --features test --all-targets` and `bash scripts/preflight.sh` from the agent repo root and confirm both report success ("Preflight clean").

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

(Add entries as you go.)

- Decision: When a CLI host override fails `BackendUrl::new` / `MqttHost::new` validation, log a `warn!` and fall back to `BackendUrl::default()` / `MqttHost::default()` rather than returning an error or panicking.
  Rationale: The branch's rebase deliberately moved `determine_settings` to a bare-`Settings` signature (provision tests were updated to drop `.unwrap()`). Falling back with a warning keeps that signature and matches the "fall back, don't crash" pattern from commit `57e0080`. CLI parsing in `agent/src/cli/mod.rs` does not validate hosts, so this code path is reachable.
  Date/Author: 2026-05-01 / Ben Smidt (plan author).

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent repo is a Rust workspace at `/home/ben/miru/workbench2/repos/agent`. Run all commands from that directory unless otherwise noted.

Key files for this work:

- `agent/src/network/mod.rs` — defines the `BackendUrl` and `MqttHost` newtypes added in commit `7d0dfe6 refactor(network): extract URL/host newtypes into network module`. Constructor signatures:
    - `BackendUrl::new(raw: &str) -> Result<Self, String>`
    - `MqttHost::new(host: &str) -> Result<Self, String>`
  Both types implement `Default`: `BackendUrl::default()` is `https://api.mirurobotics.com/agent/v1`; `MqttHost::default()` is `mqtt.mirurobotics.com`. Both implement `Display` and a `.as_str()` accessor.
- `agent/src/provisioning/shared.rs` — contains `pub(super) fn determine_settings(backend_host: Option<&str>, mqtt_broker_host: Option<&str>) -> settings::Settings`. Currently broken: it calls the new constructors with `String` instead of `&str`, and assigns the `Result` directly into the newtype field. (4 compile errors.)
- `agent/src/provisioning/provision.rs` — wraps `shared::determine_settings`. Tests in the `tests::determine_settings` submodule have already been updated in the working tree to drop `.unwrap()` and use `.as_str()` for newtype comparison; do **not** touch these.
- `agent/src/provisioning/reprovision.rs` — wraps `shared::determine_settings` for the `reprovision` subcommand. Tests in `tests::determine_reprovision_settings` still compare `settings.backend.base_url` and `settings.mqtt_broker.host` against bare `&str` literals — these need the same `.as_str()` treatment as `provision.rs` got. (2 compile errors when `--all-targets`.)
- `agent/src/main.rs` — `run_provision` at line 60 currently calls `provision::determine_settings(&args)?`. The `?` is inappropriate now that `determine_settings` returns bare `Settings`. `run_reprovision` at line 113 already calls without `?` (correct).
- `agent/src/cli/mod.rs` — defines `ProvisionArgs` and `ReprovisionArgs`. Both expose `backend_host: Option<String>` and `mqtt_broker_host: Option<String>` populated by raw `--key=value` parsing with **no** validation. So invalid host strings are reachable post-CLI.
- `agent/src/lib.rs` — already correctly adds `pub mod network;` in the working tree (uncommitted). Do **not** modify.
- `agent/src/storage/settings.rs` — reference for the established "fall back, don't crash" pattern (commit `57e0080`). The `Backend` and `MQTTBroker` `Deserialize` impls call `validate_…` and on `Err`, emit `warn!("… rejected (…); falling back to default `…`")` and substitute the default. Mirror this `warn!` wording for consistency.
- `scripts/preflight.sh` — the validation gate for this work. Prints `Preflight clean` on success and exits non-zero with `Preflight FAILED (...)` on failure.

Definitions:

- **Newtype** — a Rust struct that wraps a single value to encode invariants in the type system. Here `BackendUrl(Url)` and `MqttHost(String)` enforce the allowed-domain rule via their fallible constructors.
- **Allowed-domain rule** — backend URLs and MQTT hosts must either be a loopback literal (`localhost`, `127.0.0.1`, `::1`) or end with `.mirurobotics.com` / equal `mirurobotics.com`. Backend URLs must additionally be `https` (or `http` for loopback) with no userinfo and a host component.
- **Preflight** — the umbrella check at `scripts/preflight.sh` that runs lint + tests in both the agent crate and the `tools/` workspace in parallel. It prints `Preflight clean` on success.

## Plan of Work

The work is three small edits and a validation pass. Keep changes minimal — do not adjust signatures, names, or unrelated behaviour.

1. **`agent/src/provisioning/shared.rs`, function `determine_settings`** (lines 36–48). Rewrite the body so it:
    - Passes `&str` (not `String`) to the newtype constructors. Use `&format!("{host}/agent/v1")` for the backend URL; pass `host` directly for the MQTT host (it is already `&str`).
    - Handles the `Result`. On `Err(msg)`, emit a `warn!` matching the storage-layer wording and fall back to the field default. Use `.unwrap_or_else(|msg| { warn!(...); <Newtype>::default() })`.
    - Continues to return bare `settings::Settings` (do not change the signature).

   Concretely, replace:

       if let Some(host) = backend_host {
           settings.backend.base_url = BackendUrl::new(format!("{}/agent/v1", host));
       }
       if let Some(host) = mqtt_broker_host {
           settings.mqtt_broker.host = MqttHost::new(host.to_string());
       }

   with:

       if let Some(host) = backend_host {
           let raw = format!("{host}/agent/v1");
           settings.backend.base_url = BackendUrl::new(&raw).unwrap_or_else(|msg| {
               let fallback = BackendUrl::default();
               warn!(
                   "backend host override `{raw}` rejected ({msg}); falling back to default `{fallback}`"
               );
               fallback
           });
       }
       if let Some(host) = mqtt_broker_host {
           settings.mqtt_broker.host = MqttHost::new(host).unwrap_or_else(|msg| {
               let fallback = MqttHost::default();
               warn!(
                   "mqtt broker host override `{host}` rejected ({msg}); falling back to default `{fallback}`"
               );
               fallback
           });
       }

   The `warn!` import is already in scope via `use tracing::{debug, error, info, warn};` at the top of the file.

2. **`agent/src/provisioning/reprovision.rs`, `tests::determine_reprovision_settings`** (lines 79–115). Apply the same shape the working tree already applies in `provision.rs`:
    - Change the host string literals from `https://custom.example.com` / `mqtt.custom.example.com` to allowed-domain hosts (`https://custom.mirurobotics.com` / `mqtt.custom.mirurobotics.com`). The on-disk `provision.rs` already shows this is what is wanted; the same validation rule applies to both code paths.
    - Compare via `.as_str()` against the expected literal, exactly as `provision.rs` now does:

          assert_eq!(
              settings.backend.base_url.as_str(),
              "https://custom.mirurobotics.com/agent/v1"
          );

      and

          assert_eq!(
              settings.mqtt_broker.host.as_str(),
              "mqtt.custom.mirurobotics.com"
          );

    - Leave the `no_overrides_preserves_defaults` test alone — its `assert_eq!(settings.backend.base_url, defaults.backend.base_url)` compares two newtypes and already works because both implement `PartialEq`.
    - Do not change the function signature or its caller.

3. **`agent/src/main.rs`, `run_provision`** (line 60). Drop the trailing `?` so the call matches `provision::determine_settings`'s bare-`Settings` return type. Change

       let settings = provision::determine_settings(&args)?;

   to

       let settings = provision::determine_settings(&args);

   Leave the rest of `run_provision` and the entire `run_reprovision` unchanged.

4. **Validate**. Run the steps in *Concrete Steps* below.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent` unless stated otherwise.

### Milestone 1: fix `shared.rs`

1. Apply the rewrite described in *Plan of Work* item 1 to `agent/src/provisioning/shared.rs`.
2. Verify the lib compiles:

       cargo check --package miru-agent --features test 2>&1 | tail -20

   Expected: no errors mentioning `shared.rs`. The build may still fail with the remaining 2 errors in `reprovision.rs` tests when `--all-targets` is used; that is fine for this milestone.
3. Commit (from the agent repo root):

       git add agent/src/lib.rs agent/src/provisioning/shared.rs
       git commit -m "fix(provisioning): adapt shared::determine_settings to BackendUrl/MqttHost newtypes"

   Note: `agent/src/lib.rs` is bundled into this commit because its `pub mod network;` addition is what makes `shared.rs` compile; the two changes belong together.

### Milestone 2: fix `reprovision.rs` tests

1. Apply the test edits described in *Plan of Work* item 2 to `agent/src/provisioning/reprovision.rs`.
2. Verify the lib + tests compile:

       cargo check --package miru-agent --features test --all-targets 2>&1 | tail -20

   Expected: zero `error[E…]` lines.
3. Commit:

       git add agent/src/provisioning/reprovision.rs agent/src/provisioning/provision.rs
       git commit -m "test(provisioning): retarget determine_settings tests onto newtype getters"

   Note: `provision.rs` is bundled here because its working-tree changes (drop `.unwrap()`, switch to `.as_str()`) are the matching change for the same refactor.

### Milestone 3: fix `main.rs` call site

1. Edit `agent/src/main.rs` line 60 per *Plan of Work* item 3.
2. Verify the binary compiles:

       cargo check --package miru-agent --features test --all-targets 2>&1 | tail -20

   Expected: zero errors.
3. Commit:

       git add agent/src/main.rs
       git commit -m "fix(main): drop spurious \`?\` on bare-Settings determine_settings call"

### Milestone 4: full preflight

1. Run preflight:

       bash scripts/preflight.sh

   Expected final lines:

       Preflight clean

   If preflight fails, read each `=== Lint ===` / `=== Tests ===` / `=== Tools Lint ===` / `=== Tools Tests ===` block to locate the failure, fix it in place, and re-run preflight. Do not proceed to publish until preflight is clean.
2. There is no separate commit for this milestone — preflight is a validation gate, not a code change.

## Validation and Acceptance

Acceptance criteria, all of which must hold from `/home/ben/miru/workbench2/repos/agent`:

1. `git status` shows a clean working tree (all edits committed).
2. `cargo check --package miru-agent --features test --all-targets` exits 0 with no `error[E…]` lines.
3. `bash scripts/preflight.sh` exits 0 and prints `Preflight clean` as the last non-empty line. **The plan is not complete until preflight reports `clean`.**
4. The three new commits are present on `feat/harden-allowed-domains-rebase`, in milestone order.
5. Behavioural check (manual reasoning, not a runnable test): the `determine_settings` tests in both `provision.rs` and `reprovision.rs` continue to test the same behaviour they did before — backend host appends `/agent/v1`, MQTT host overrides verbatim, no-override case yields defaults — and the equality assertions now compare `.as_str()` against the expected `&str` (or compare two `BackendUrl` / `MqttHost` values directly, which already works via `PartialEq`).

Test-name verification:

- `cargo test --package miru-agent --features test provisioning::provision::tests::determine_settings -- --nocapture` runs the three `provision` tests (`backend_host_appends_agent_v1_suffix`, `mqtt_broker_host_override`, `no_overrides_preserves_defaults`); all three pass after this change. They fail to compile before this change because of the `shared.rs` errors that the wrapper depends on.
- `cargo test --package miru-agent --features test provisioning::reprovision::tests::determine_reprovision_settings -- --nocapture` runs the three reprovision tests. All three pass after this change.

## Idempotence and Recovery

- Every edit in this plan is a textual replacement on a small, well-identified region. Re-reading the file and re-applying the rewrite is safe; the new code is itself idempotent (running it twice with the same overrides yields the same `Settings`).
- If a `git commit` fails (e.g. pre-commit hook), fix the reported issue and create a **new** commit. Do not amend; the policy in `/home/ben/miru/workbench2/CLAUDE.md` and the parent `.agents/` rules forbid bypassing hooks.
- If preflight fails on the agent crate, the failure is local to the edits in this plan — revert by `git restore -SW agent/src/provisioning/shared.rs agent/src/provisioning/reprovision.rs agent/src/main.rs` and reapply. Do **not** revert `agent/src/lib.rs` or `agent/src/provisioning/provision.rs` test changes — those are correct on the rebased branch.
- The plan does not push to the remote; final delivery (force-push, PR sync) is handled by the orchestrator after preflight is clean.
