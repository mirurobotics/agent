# Narrow network loopback rule to `localhost` only

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Narrows `is_loopback_host` to `localhost`, removes IPv6/bracket handling in `BackendHost`, simplifies `explicit_port`, updates affected tests. |

This plan lives in `agent/plans/` because all changes are confined to `agent/src/network/` and its tests (plus six MQTT integration-test call sites).

This work amends the existing PR #66 on branch `refactor/backend-host`. Do not create a new branch; commit on top of the current branch.

## Purpose / Big Picture

`agent/src/network/mod.rs` currently treats `localhost`, `127.0.0.1`, and `::1` as loopback. The IP-literal cases force the module to carry IPv6 bracket parsing, multi-colon port-parsing guards, and per-form authority-formatting branches — none of which serve a real operator workflow. The only loopback an operator ever uses for the agent's backend or MQTT broker is `localhost`. Narrowing the rule shrinks the type and the test surface while leaving observable behaviour for `localhost` and `mirurobotics.com` hosts identical.

After this change: `BackendHost::new("localhost")` and `BackendHost::new("api.mirurobotics.com")` continue to work; `BackendHost::new("127.0.0.1")` and `BackendHost::new("::1")` (and `[::1]:8080`) return `Err`. Same for `MqttHost`.

## Progress

- [ ] Edit `agent/src/network/mod.rs` per Plan of Work.
- [ ] Edit `agent/tests/network/mod.rs` per Plan of Work.
- [ ] Edit `agent/tests/mqtt/{options,client,errors}.rs` per Plan of Work.
- [ ] Run `cargo build`, `cargo fmt -p miru-agent -- --check`, `cargo clippy --package miru-agent --all-features -- -D warnings`, `./scripts/test.sh`.
- [ ] Commit `refactor(network): narrow loopback rule to localhost only`.
- [ ] Run `./scripts/preflight.sh`; expect clean.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: Narrow loopback to `localhost` only.
  Rationale: The only realistic local-dev case is `localhost`. `127.0.0.1` and `::1` support no operator workflow we have, yet they force IPv6 bracket handling, multi-colon port-parsing guards, and per-form authority branches. Dropping them shrinks the type and the test surface.
  Date/Author: 2026-05-08 / ben.

- Decision: Tighten `MqttHost` together with `BackendHost`.
  Rationale: Both newtypes share `is_loopback_host`; the network module's accepted-host invariant should be uniform. Six integration-test call sites (`mqtt/options.rs:68`, `mqtt/client.rs:27/66/89/106`, `mqtt/errors.rs:323`) need a mechanical `127.0.0.1` → `localhost` rename. The production `is_loopback_host` check at `agent/src/mqtt/options.rs:58` (SSL-unless-loopback) becomes stricter — an MQTT broker configured as `127.0.0.1` or `::1` would now require SSL — but in practice only `mqtt.mirurobotics.com` and `localhost` reach this code path, so the impact is nil.
  Date/Author: 2026-05-08 / ben.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Working directory for all commands: `cd /home/ben/miru/workbench2/repos/agent`.

Key files:

- `agent/src/network/mod.rs` — defines `is_loopback_host`, `is_allowed_host`, `explicit_port`, and the `BackendHost` and `MqttHost` newtypes. The accepted-host invariant for both newtypes is `is_loopback_host(host) || is_allowed_host(host)`. `is_allowed_host` (the `mirurobotics.com` rule) is unchanged by this work.
- `agent/tests/network/mod.rs` — owns the `BackendHost` and `MqttHost` unit tests.
- `agent/tests/mqtt/{options,client,errors}.rs` — six call sites construct `MqttHost::new("127.0.0.1")` purely as test fixtures.
- `agent/src/mqtt/options.rs:58` — calls `is_loopback_host(broker.as_str())` to enforce "SSL unless loopback". Behaviour change from this plan is theoretical; record as decision-log note above.
- `agent/tests/http/` and `agent/tests/mocks/` use raw socket binds with `127.0.0.1` literals — those are out of scope.

Out of scope (must not change):

- `is_allowed_host` rule (`mirurobotics.com`).
- `BackendHost`'s default value (`api.mirurobotics.com`).
- `as_url()` shape: `<scheme>://<authority>/agent/v1`, no trailing slash.
- The on-disk JSON key (`host`) and the warn-and-default fallback path.
- All call sites in `agent/src/main.rs`, `agent/src/app/`, `agent/src/storage/`, `agent/src/provisioning/` — they use `BackendHost::as_url()` and don't care about the loopback set.
- `http::Client::new(&str)` signature.

## Plan of Work

In `agent/src/network/mod.rs`:

- `is_loopback_host` (lines 10-12): replace body with `host == "localhost"`. Keep the function name (cheaper diff than rename).
- `explicit_port` (lines 23-37): collapse to `raw.rfind(':').and_then(|i| raw[i+1..].parse::<u16>().ok())`. Drop the IPv6 bracket guard at lines 24-26 and the multi-colon check at line 33.
- `BackendHost::new` (lines 70-135):
  - Remove the `::1` IPv6 special-case wrapping at lines 88-92.
  - Remove the bracket-strip on `host_str()` at line 114 (`localhost` and `mirurobotics.com` hosts never have brackets).
  - Remove the `host_is_ipv6` flag and the bracketed `formatted` branch at lines 123-128. Construct `formatted` as `host.to_string()` when `port` is `None` and `format!("{host}:{port}")` when `Some`.
- `BackendHost::as_url` (lines 157-173): scheme branch at line 158 still calls `is_loopback_host` (now narrower). Remove the IPv6 authority branches at lines 165-171; authority is `host` or `format!("{host}:{port}")`.
- `MqttHost::new` (lines 210-216): no body change; the validation `is_loopback_host(host) || is_allowed_host(host)` automatically inherits the narrowed rule.
- Doc-comments at lines 9, 64, 68-69 mentioning `127.0.0.1` / `::1`: reword to `localhost`-only.

In `agent/tests/network/mod.rs`:

Delete eight tests. In `mod backend_host_new`: `accepts_loopback_ipv4` (28-30), `accepts_loopback_ipv6` (33-35), `as_url_http_for_loopback_ipv4` (210-213), `as_url_http_for_loopback_ipv6` (216-219), `accepts_loopback_ipv6_with_port` (234-237), `as_url_http_loopback_ipv6_with_port` (240-243). In `mod mqtt_host_new`: `accepts_loopback_ipv4` (274-276), `accepts_loopback_ipv6` (279-281).

Add five tests:

- `rejects_loopback_ipv4`: `BackendHost::new("127.0.0.1")` returns `Err`.
- `rejects_loopback_ipv6`: `BackendHost::new("::1")` returns `Err`.
- `rejects_bracketed_ipv6_loopback`: `BackendHost::new("[::1]:8080")` returns `Err`.
- `mqtt_rejects_loopback_ipv4`: `MqttHost::new("127.0.0.1")` returns `Err`.
- `mqtt_rejects_loopback_ipv6`: `MqttHost::new("::1")` returns `Err`.

Tests that must remain passing: `accepts_loopback_localhost` (23-25), `accepts_loopback_with_port` (44-47), `as_url_http_for_localhost` (204-207), `as_url_http_loopback_with_port` (228-231), `preserves_explicit_port_80_for_loopback` (246-250); plus `mqtt_host_new::accepts_localhost` (269-271).

In `agent/tests/mqtt/{options,client,errors}.rs`:

Mechanical `MqttHost::new("127.0.0.1")` → `MqttHost::new("localhost")` at six call sites:

- `agent/tests/mqtt/options.rs:68`
- `agent/tests/mqtt/client.rs:27, 66, 89, 106`
- `agent/tests/mqtt/errors.rs:323`

## Concrete Steps

One milestone, one commit. From `cd /home/ben/miru/workbench2/repos/agent`:

1. Apply edits in `agent/src/network/mod.rs` per Plan of Work.
2. Apply edits in `agent/tests/network/mod.rs` per Plan of Work (delete 8 tests, add 5).
3. Apply the six `127.0.0.1` → `localhost` substitutions in `agent/tests/mqtt/{options,client,errors}.rs`.
4. Run gates and observe a clean run:
   - `cargo build`
   - `cargo fmt -p miru-agent -- --check`
   - `cargo clippy --package miru-agent --all-features -- -D warnings`
   - `./scripts/test.sh`
5. Commit on the existing `refactor/backend-host` branch:

       git add agent/src/network/mod.rs agent/tests/network/mod.rs \
         agent/tests/mqtt/options.rs agent/tests/mqtt/client.rs agent/tests/mqtt/errors.rs
       git commit -m "refactor(network): narrow loopback rule to localhost only"

6. Run `./scripts/preflight.sh`; expect a clean run.

## Validation and Acceptance

- `cargo build` succeeds.
- `cargo fmt -p miru-agent -- --check` produces no diff.
- `cargo clippy --package miru-agent --all-features -- -D warnings` reports no warnings.
- `./scripts/test.sh` passes; the five new `rejects_*` tests pass after the change and would have failed before (they assert `Err` for inputs the old rule accepted).
- `./scripts/preflight.sh` runs clean.

The narrowed surface is locked in by the new `rejects_*` tests: any future regression that re-admits `127.0.0.1` or `::1` will fail them.

## Idempotence and Recovery

Edits are deterministic — re-running them yields the same tree. If anything goes wrong after the commit lands on `refactor/backend-host`, roll back with `git revert <commit>` on that branch and force-push if PR #66 has not yet merged.
