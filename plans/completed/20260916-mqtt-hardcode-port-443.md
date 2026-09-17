# Hardcode MQTT broker port 443

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Change the agent's MQTT broker port from `8883` to `443` in two source files and update one test assertion. |
| `infra/` | read-only | Context only: the TLS-passthrough proxy that serves MQTT-over-TLS on `443`, and the per-environment `mqtt_dns_target` DNS cutover this change depends on. No changes here. |

This plan lives in `agent/plans/backlog/` because all code changes are made in the agent repo. Working branch is `feat/mqtt-hardcode-port-443` (already checked out, base `main`).

## Purpose / Big Picture

Miru devices reach the MQTT broker through the infra TLS-passthrough proxy, which serves MQTT-over-TLS on port **443**. This change makes `443` the agent's standard, hardcoded MQTT broker port (replacing `8883`). It is a deliberate product decision: the port is a fixed constant, **not** a configurable setting. After this change, a device that builds its MQTT connect address from settings (host only) will target `443` over TLS instead of `8883`.

Observable behavior: `ConnectAddress::default()` reports `port() == 443`, and `build_app_options` produces a broker address on port `443`. The Display form of the default address becomes `ssl://mqtt.mirurobotics.com:443`.

## Release precondition / rollout gate

**This is the primary risk of the change. It must also be reproduced in the PR body.**

Hardcoding `443` is a **breaking change** for any device that upgrades to this agent version in an environment whose DNS has **not** yet been cut over to the TLS-passthrough proxy. As of today (2026-09-16):

- **Staging** is cut over (`mqtt.mirurobotics.com` resolves to the proxy, which serves MQTT-over-TLS on `443`).
- **uat** and **production** are **not** cut over: `mqtt.mirurobotics.com` still resolves directly to EMQX Cloud, which listens on `8883` only, **not** `443`.

A device running this version in uat or production would therefore **fail to establish its MQTT connection** until that environment's DNS is cut over (infra `mqtt_dns_target = "proxy"`).

**Rollout gate — mandatory ordering:** this agent version must **not** be rolled out to devices in an environment until that environment's DNS is cut over to the proxy. The safe order is **per-environment DNS cutover first, then agent upgrade** in that environment.

Mitigating fact (does not remove the gate): the agent's fallback poller (12-hour interval) still runs, so an affected device would not lose deployments permanently — only real-time MQTT delivery is lost until the environment is cut over or the device is rolled back.

## Progress

- [ ] (2026-09-16) Milestone 1: change the two source constants and the one default-path test assertion; classify the remaining `8883` occurrences as leave-as-explicit.
- [ ] (2026-09-16) Milestone 1: run `scripts/lint.sh`, `scripts/test.sh`, `scripts/covgate.sh`; commit.
- [ ] (2026-09-16) Milestone 2: open/refresh the PR (draft) with the rollout-gate section in the body; run preflight; leave draft only once CI is green.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: hardcode `443` as a constant rather than adding a `port` field to settings.
  Rationale: explicit product decision — `443` is the single standard MQTT port for all devices via the proxy; a configurable port would invite drift and is not wanted.
  Date/Author: 2026-09-16, plan author.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The agent is a Rust binary (`miru-agent`, package root `agent/`). MQTT connection targeting lives in `agent/src/mqtt/options.rs`, which defines the `ConnectAddress` type: a validated `{ protocol, broker, port }` triple. Key facts about `ConnectAddress`:

- `ConnectAddress::new(broker, protocol, port)` is the only checked constructor; it enforces the SSL-unless-loopback rule (a non-loopback broker must use `Protocol::SSL`). This rule is **not** changed by this plan.
- `ConnectAddress::new_or(broker, protocol, port, fallback)` returns the constructed address on success, else logs and returns `fallback`.
- `impl Default for ConnectAddress` currently yields `{ protocol: SSL, broker: mqtt.mirurobotics.com, port: 8883 }`. This default is the canonical value and is also passed as the `fallback` in `build_app_options`.

`build_app_options` (private fn in `agent/src/main.rs`, the binary crate) constructs the runtime broker address:

    let broker_address = ConnectAddress::new_or(
        settings.mqtt_broker.host,
        Protocol::SSL,
        8883,                      // <- becomes 443
        ConnectAddress::default(), // fallback; its port also becomes 443
    );

Because `build_app_options` is private to the binary crate, it is **not** reachable from the integration test suite in `agent/tests/`, and no test currently asserts its output. Adding a dedicated assertion for it is impractical (would require exposing the fn or the binary's internals) and is **out of scope**; the default-path unit test below is the effective coverage.

Callers of `ConnectAddress::default()` whose port silently moves from `8883` to `443` (all intended and consistent — verified by grep):

- `agent/src/main.rs:242` — the `new_or` fallback. Intended: fallback should be the new standard.
- `agent/src/mqtt/options.rs:168` — `Options::new` sets `connect_address: ConnectAddress::default()`. Consistent.
- `agent/src/workers/mqtt.rs:45` — worker `broker_address: ConnectAddress::default()`. Consistent; no test asserts `8883` here.
- Test callers `agent/tests/mqtt/options.rs:105, 120, 210, 224` compare a default against a default (fallback-equality and `Options` struct-equality). They stay valid regardless of the port value — no change needed.

Test/lint conventions (from `agent/AGENTS.md`; there is no `agent/CLAUDE.md`): unit tests live in inline `#[cfg(test)]` modules; integration tests live under `agent/tests/`. Coverage is gated per-module by `.covgate` files (e.g. `agent/src/mqtt/.covgate` = `96.88`), checked by `scripts/covgate.sh`. A `.covgate` value is only ever changed via `scripts/update-covgates.sh`, never hand-edited, and must not be ratcheted for a module with credentialed integration tests. This change flips constants only — it adds no new branches — so coverage is expected to be unaffected and no `.covgate` should need touching.

## Plan of Work

Milestone 1 — code and test change (three edits):

1. `agent/src/main.rs`, in `build_app_options` (around line 241): change the literal `8883` to `443`. `Protocol::SSL` stays unchanged.
2. `agent/src/mqtt/options.rs`, in `impl Default for ConnectAddress` (around line 46): change `port: 8883` to `port: 443`.
3. `agent/tests/mqtt/options.rs`, in the `connect_address::default` test (around line 33): change `assert_eq!(addr.port(), 8883);` to `assert_eq!(addr.port(), 443);`.

Do **not** add a `port` field to settings or any configurable path. Do **not** change `Protocol::SSL`, the SSL-unless-loopback rule, host handling, or the poller.

### Classification of every `8883` occurrence

From `grep -rn 8883 agent/src agent/tests --include=*.rs` (excluding `18883`), five occurrences plus the source constant:

- `agent/src/main.rs:241` (`build_app_options` `new_or` port) — **CHANGE to 443.** This is the production broker port.
- `agent/src/mqtt/options.rs:46` (`impl Default` `port: 8883`) — **CHANGE to 443.** Canonical default and `new_or` fallback; must match the new standard.
- `agent/tests/mqtt/options.rs:33` (`default` test, `assert_eq!(addr.port(), 8883)`) — **CHANGE to 443.** Asserts the default/production path directly.
- `agent/tests/mqtt/options.rs:71` (`accepts_loopback_ssl`, `new(127.0.0.1, SSL, 8883)`) — **LEAVE.** Explicit port testing that loopback + SSL is accepted; the port value is arbitrary and unrelated to the default.
- `agent/tests/mqtt/options.rs:79` (`accepts_allowed_host_ssl`, `new(mqtt.mirurobotics.com, SSL, 8883)`) — **LEAVE.** Explicit port testing that the constructor accepts an allowed host with SSL; exercises constructor validation, not the default path.
- `agent/tests/mqtt/options.rs:146/149` (`renders_ssl_allowed_domain` Display test, `"ssl://mqtt.mirurobotics.com:8883"`) — **LEAVE.** Tests Display formatting of an explicitly-constructed address; `8883` is the value being formatted, not the default.

Milestone 2 — validation and PR (no code changes beyond fixups CI demands).

## Concrete Steps

All commands run from the repo root `agent/` (i.e. `/home/ben/miru/workbench1/repos/agent`) unless stated. Confirm branch first:

    git -C /home/ben/miru/workbench1/repos/agent branch --show-current
    # expect: feat/mqtt-hardcode-port-443

Milestone 1:

1. Apply the three edits described in Plan of Work (main.rs `8883`→`443`; options.rs `port: 8883`→`port: 443`; tests/mqtt/options.rs line ~33 assertion `8883`→`443`).

2. Confirm exactly the intended `8883` occurrences remain (the three LEAVE entries):

       grep -rn 8883 agent/src agent/tests --include=*.rs | grep -v 18883
       # expect 3 lines: tests/mqtt/options.rs:71, :79, :146 (:149 string)
       # expect NO lines in agent/src

3. Lint (auto-fixes; re-check status afterward):

       ./scripts/lint.sh
       git status --short   # confirm lint introduced no unexpected changes

4. Test:

       ./scripts/test.sh
       # expect: all tests pass, including mqtt::options default test now asserting 443

5. Coverage gate:

       ./scripts/covgate.sh
       # expect: PASS with no .covgate change (constant flip adds no branches)
       # ONLY if it fails: run ./scripts/update-covgates.sh (never hand-edit a .covgate),
       # and do NOT ratchet a module that has credentialed integration tests.

6. Commit (one commit for this milestone):

       git add agent/src/main.rs agent/src/mqtt/options.rs agent/tests/mqtt/options.rs
       git commit
       # message: "feat(mqtt): hardcode broker port 443 (proxy TLS passthrough)"

Milestone 2 — PR and preflight:

7. Run the preflight loop (drives CI):

       ./scripts/preflight.sh

8. Open (or refresh) the PR as a **draft**. The PR body must include the "Release precondition / rollout gate" content from this plan verbatim in substance: 443 requires per-environment DNS cutover to the proxy; staging is cut over, uat/production are not; roll out per environment cutover-first; fallback poller (12h) limits blast radius to real-time MQTT only.

9. Leave the PR in draft until preflight reports **CLEAN** — CI green on the pushed head (agent CI workflow is `.github/workflows/ci.yml`). Only then mark it ready for review.

## Validation and Acceptance

- `grep -rn 8883 agent/src ... | grep -v 18883` returns **no** `agent/src` lines and exactly the three explicit-port test lines (71, 79, 146/149).
- `./scripts/test.sh` passes. The `connect_address::default` test in `agent/tests/mqtt/options.rs` asserts `addr.port() == 443`; it fails before edit 2 (default still `8883`) and passes after. The three LEAVE tests (loopback-SSL accept, allowed-host-SSL accept, Display render) continue to pass unchanged.
- `./scripts/lint.sh` reports clean and introduces no unexpected working-tree changes (re-check `git status`).
- `./scripts/covgate.sh` passes with no `.covgate` modified.
- Behavioral acceptance: `ConnectAddress::default().port()` is `443`; its Display is `ssl://mqtt.mirurobotics.com:443`; `build_app_options` yields a broker address on port `443` (verified indirectly via the default-path test, since the fn is private to the binary crate).
- Preflight/CI: preflight reports **CLEAN** (CI green on the pushed head, `.github/workflows/ci.yml`) before the PR leaves draft.

## Idempotence and Recovery

All edits are literal constant swaps and are safe to re-apply or inspect repeatedly. The grep in step 2 is the idempotence check: it deterministically confirms the intended end state regardless of how many times steps run. If the wrong occurrence was changed, revert with `git checkout -- <file>` and re-apply per the classification table. `scripts/lint.sh`, `scripts/test.sh`, and `scripts/covgate.sh` are read-only w.r.t. source (lint may auto-format) and may be run repeatedly. If `covgate.sh` fails, recover only via `scripts/update-covgates.sh`; never hand-edit a `.covgate`. No migrations, no destructive operations.
