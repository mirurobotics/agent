# Make the domain status enums version-neutral

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Drop the Device API twin from `impl_status_enum!`, write the v0.2 enum conversions in `server/v02/response.rs`, move the twin tests to the v0.2 test layer, docs |

This plan lives in `agent/plans/` because every change is in the agent repo. Paths are relative to the agent repo root (the directory holding the workspace `Cargo.toml` and `ARCHITECTURE.md`); the crate is in `agent/`. Branch: `refactor/version-neutral-status-enums`, stacked on `refactor/domain-deployment-events` (PR 1, plan `plans/backlog/20261114-domain-deployment-events.md`), which is based on `main` at `4ca00ca2`. Open the PR against PR 1's branch; retarget it to `main` once PR 1 merges.

## Purpose / Big Picture

The agent serves a local HTTP API, the Device API, to programs on the same device. Since agent#485 each Device API version is one generated crate (`libs/device-api-v02`, imported as `device_api_v02`) plus one module (`agent/src/server/v02/`). A later PR adds `/v0.3` as a full copy of `server/v02/` over its own crate `device_api_v03`.

PR 1 moved the event store off the v0.2 structs. The last place outside `server/v02/` that depends on the v0.2 crate is the domain status enums in `agent/src/models/`: each `impl_status_enum!` call names one v0.2 enum with an `agent_type` clause, and the macro generates the conversion to it. A macro clause can only name one version, so `/v0.3` would have to edit `models/`.

After this change nothing in `agent/src/` outside `server/v02/` imports `device_api_v02`, and `/v0.3` adds its own conversions in its own `response.rs` without touching `models/` or `events/`. Nothing a client sees changes: every `/v0.2` response body is the same.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `assert_converts_each` (private generic test helper in `agent/tests/server/v02/response.rs`) | `assert_wire_conversions`; `assert_agent_conversions` (the harness name it replaces) | Reads like its sibling test `converts_each_code`; "agent" in the old name meant the v0.2 server crate, which is now the module the helper sits in. |

The seven `From<&Domain> for device_server::<Enum>` impls add no names.

### Precedent

No new files. New code in existing files mirrors these siblings:

| New code | Mirrors |
|----------|---------|
| Seven `impl From<&models::X> for device_server::Y` in `agent/src/server/v02/response.rs` | `impl From<SyncCode> for device_server::SyncDeviceResult` in the same file (a `match` with `Self::` arms) |
| Module `status_enums` in `agent/tests/server/v02/response.rs` | `sync_device_response::converts_each_code` in the same file (a hand-written table of domain value and wire value) |

### Left out

- A macro clause that takes one Device API type per version (e.g. `device_types: [v02 => ..., v03 => ...]`): each version module is a full copy that writes its own `From` impls, and the tables are 2 to 8 arms.
- Request-direction conversions (v0.2 enum to domain): none exist today; the agent only sends these enums.
- Removing `device_api_v02::models::ApiVersion` from `agent/tests/server/tcp.rs` and `agent/tests/app/run.rs`: transport tests need a served path, and v0.2 is the only version. `agent/tests/events/model.rs` keeps the v0.2 structs on purpose (PR 1).
- `/v0.3` itself: the follow-up PR.
- Pattern First, Then Fan Out: the seven enums are one mechanical change to an existing pattern (the `SyncCode` conversion), not a new pattern across resources, and the macro's agent arms can only go once all seven drop `agent_type`. All seven are done here.

### Open questions

- Add a clause to `ARCHITECTURE.md` stating that nothing outside a version module imports its generated crate? Default: yes, one clause on the existing `server` sentence, so the `/v0.3` author keeps the rule.

## Progress

- [x] M1: source (macro, call sites, v0.2 conversions).
- [ ] M2: tests (harness, fixtures, v0.2 conversion tests).
- [x] M3: docs (`AGENTS.md`, `ARCHITECTURE.md`).
- [ ] CI green on the pushed head (`$preflight` reports `CLEAN`).
- [x] Source written

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Open question taken at its default: `ARCHITECTURE.md`'s `server` paragraph now says nothing outside a version module imports its generated crate.
- M3 (docs) ran in the source stage with M1, so the design review sees the documented rule; M2 (tests) is left for the tests stage.
- The four deployment conversions in `server/v02/response.rs` follow the order `From<&models::Deployment>` calls them (status, activity, error, target), per "order functions by control flow".
- In `deployment.rs` and `file_rule.rs` each mapping keeps `Variant => "wire" =>` and the `backend_client::` path on two lines, as before, since one line would pass 100 columns; `os.rs` and `device.rs` fit on one line.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

`impl_status_enum!` (`agent/src/models/status.rs`) is a `macro_rules!` macro that, from one mapping table, generates a status enum's custom `Deserialize` (an unknown string becomes the default with a log), `variants()`, `as_str()`, optional `Display`, inline serde tests, and conversions to "twins": the same enum in a generated crate. It has four public arms: local (no twin), agent (`agent_type` only), backend+agent (`agent_type`, `backend_type`, `unknown_backend`), backend-only (`backend_type`, `unknown_backend`). The private `@agent` arm generates `impl From<&Domain> for AgentType`; `@backend` generates both directions with `backend_api` (the control-plane client crate) plus tests. The header comment calls backend-only "no current users", which is stale: `InstanceFormat` (`agent/src/models/instance_format.rs`) uses it. `LogLevel` (`agent/src/logs/mod.rs`) uses the local arm.

Seven call sites pass `agent_type: agent_server::...` with `use device_api_v02::models as agent_server;`:

- `agent/src/models/deployment.rs`: `DplTarget` → `DeploymentTargetStatus`, `DplActivity` → `DeploymentActivityStatus`, `DplErrStatus` → `DeploymentErrorStatus`, `DplStatus` → `DeploymentStatus`, each also with `backend_type`.
- `agent/src/models/os.rs`: `Os` → `Os`, with `backend_type`.
- `agent/src/models/file_rule.rs`: `UploadTimezone` → `FileRuleUploadTimezone`, with `backend_type`.
- `agent/src/models/device.rs`: `DeviceStatus` → `DeviceStatus`, agent only.

After PR 1 the generated conversions have one caller, `agent/src/server/v02/response.rs` (imports `device_api_v02::models as device_server`), which uses `(&x).into()` to fill `Device.status`, `Deployment.{status, activity_status, error_status, target_status}`, `Release.os`, `BaseFileRule.os` and `FileRuleUpload.timezone`. Its existing enum conversion `impl From<SyncCode> for device_server::SyncDeviceResult` matches each variant to a `Self::` value.

Wire strings. Each domain enum derives `Serialize` with `#[serde(rename_all = "snake_case")]`; each v0.2 enum renames every variant to the same lowercase string and adds an `#[serde(other)]` `...Unknown` variant used only when parsing.

Tests. `agent/tests/models/harnesses.rs` defines `StatusCase<T>` (`variant`, `wire`, `backend: T::Backend`, `agent: T::Agent`), trait `StatusEnumFixture` (`type Backend; type Agent; fn cases()`), `assert_agent_conversions`, `assert_agent_wire_strings`, and `status_enum_tests!(Type, backend, agent)` whose `@agent` arm runs those two. Fixtures: `agent/tests/models/deployment.rs` (four), `os.rs`, `file_rule.rs` (`UploadTimezone`), `device.rs` (`DeviceStatus`, `Backend = ()`), and, with `Agent = ()`, `instance_format.rs` and `agent/tests/logs/mod.rs` (`LogLevel`). The four model test files import `device_api_v02::models as agent_server` only for the fixtures. `agent/tests/server/v02/response.rs` (imports `device_api_v02::models as openapi`) has one module per response type; `sync_device_response::converts_each_code` loops over a `(SyncCode, openapi::SyncDeviceResult)` table.

Coverage. `scripts/covgate.sh` checks every `.covgate` under `agent/src`; each counts every file under its directory. Touched gates: `agent/src/models/.covgate` (100), `agent/src/server/.covgate` (87), `agent/src/server/v02/.covgate` (87). Never lower one.

CI. `.github/workflows/ci.yml` runs on pull requests: `lint` (import-order linter, which also limits production functions to 50 lines and flags 4+ `assert_eq!` on fields of one variable in a test; `cargo fmt --check`; clippy `-D warnings`; machete; audit), `test` (the suite plus coverage gates), `windows-check` (`cargo test --package miru-agent --locked`), `windows-agent-build`.

## Plan of Work

M1, source. In `agent/src/models/status.rs`, delete the "backend + agent" and "agent only" public arms and the `@agent` arm. Rewrite the header comment: the macro generates the backend `From` conversions (not "the agent-server and/or backend"); it has two public forms, local (neither clause, e.g. `LogLevel`, `DeviceStatus`) and backend (`backend_type` + `unknown_backend`, e.g. the `Dpl*` enums, `Os`, `UploadTimezone`, `InstanceFormat`); Device API conversions are written by hand in each version's `server/vNN/response.rs`. Keep the pointer to `AGENTS.md`.

In `agent/src/models/deployment.rs`, `os.rs`, `file_rule.rs`, delete the `agent_type:` line and the `agent_server::... =>` part of each mapping, leaving `Variant => "wire" => backend_client::...` (the form `InstanceFormat` uses), and delete `use device_api_v02::models as agent_server;`. In `agent/src/models/device.rs`, delete `agent_type:` and the import and write the mappings as `Online => "online", Offline => "offline"`.

In `agent/src/server/v02/response.rs`, add seven impls, each placed right after the first impl that calls it (`DeviceStatus` after `From<&models::Device>`, the four deployment enums after `From<&models::Deployment>`, `Os` after `From<&models::Release>`, `UploadTimezone` after `From<&models::FileRuleUpload>`), in the style of `From<SyncCode>`:

    impl From<&models::DplTarget> for device_server::DeploymentTargetStatus {
        fn from(status: &models::DplTarget) -> Self {
            match status {
                models::DplTarget::Staged => Self::DEPLOYMENT_TARGET_STATUS_STAGED,
                models::DplTarget::Deployed => Self::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
                models::DplTarget::Archived => Self::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
            }
        }
    }

and likewise `DplActivity` → `DeploymentActivityStatus` (drifted, staged, queued, deployed, removing, archived), `DplErrStatus` → `DeploymentErrorStatus` (none, failed, retrying), `DplStatus` → `DeploymentStatus` (drifted, staged, queued, deployed, removing, archived, failed, retrying), `DeviceStatus` → `DeviceStatus` (online, offline), `Os` → `Os` (`OS_LINUX`, `OS_WINDOWS`), `UploadTimezone` → `FileRuleUploadTimezone` (`FILE_RULE_UPLOAD_TIMEZONE_UTC`, `..._DEVICE`). Copy each arm from the mapping table being deleted. Take `&` so the existing `(&x).into()` call sites stay as they are.

M2, tests. In `agent/tests/models/harnesses.rs`, remove `StatusCase::agent`, `StatusEnumFixture::Agent`, `assert_agent_conversions`, `assert_agent_wire_strings` and the `(@agent ...)` arm; make the `StatusCase` doc say "and its backend twin", and the usage example `status_enum_tests!(DplTarget, backend);` and `status_enum_tests!(LogLevel);`. In each fixture remove `type Agent` and every `agent:` field: `agent/tests/models/{deployment,os,file_rule,device,instance_format}.rs` and `agent/tests/logs/mod.rs`. Change `status_enum_tests!(X, backend, agent)` to `status_enum_tests!(X, backend)` for the six backend enums and `status_enum_tests!(DeviceStatus, agent)` to `status_enum_tests!(DeviceStatus)`. Delete the now-unused `use device_api_v02::models as agent_server;` in the four model test files.

In `agent/tests/server/v02/response.rs`, add `pub mod status_enums` after the last conversion module (`git_commit_response`) with a private helper and seven tests:

    fn assert_converts_each<D, W>(cases: Vec<(D, W)>, variants: Vec<D>)
    where
        D: Serialize + PartialEq + Debug,
        W: for<'a> From<&'a D> + Serialize + PartialEq + Debug,
    {
        let tested: Vec<&D> = cases.iter().map(|(d, _)| d).collect();
        assert_eq!(tested, variants.iter().collect::<Vec<_>>());
        for (domain, wire) in cases {
            assert_eq!(W::from(&domain), wire, "{domain:?}");
            assert_eq!(serde_json::to_value(&wire).unwrap(), serde_json::to_value(&domain).unwrap());
        }
    }

The first assert makes a new domain variant without a row fail; the second catches a wrong arm; the third proves the v0.2 twin's wire string equals the domain's. Tests `converts_each_deployment_target_status`, `converts_each_deployment_activity_status`, `converts_each_deployment_error_status`, `converts_each_deployment_status`, `converts_each_device_status`, `converts_each_os`, `converts_each_upload_timezone` each pass a hand-written table of every variant in declaration order with its `openapi::` value, and `X::variants()`. Add `DplStatus` and `Os` to the `miru_agent::models` import, plus `serde::Serialize` and `std::fmt::Debug`.

M3, docs. In `AGENTS.md` (`CLAUDE.md` is a symlink to it) "Enum conventions": the first bullet becomes "Backend-mirrored status enum (has a `backend_api` twin) → `impl_status_enum!` with `backend_type` + `unknown_backend`", generating the backend conversions; the second becomes "local form (omit `backend_type`)", without "agent-only"; add one line that Device API conversions are hand-written `From` impls in each version's `server/vNN/response.rs`, tested there. In `ARCHITECTURE.md`'s `server` paragraph, extend "holding its route table, handlers and wire conversions over one generated crate" with ", which nothing outside the module imports".

## Concrete Steps

All commands run from the agent repo root, `/home/user/agent`, on `refactor/version-neutral-status-enums`.

M1: edit `agent/src/models/{status,deployment,os,file_rule,device}.rs` and `agent/src/server/v02/response.rs`, then check that only the v0.2 module imports the crate:

    grep -rn device_api_v02 agent/src | grep -v '^agent/src/server/v02/'

Expected: no output.

    git add agent/src/models/status.rs agent/src/models/deployment.rs agent/src/models/os.rs agent/src/models/file_rule.rs agent/src/models/device.rs agent/src/server/v02/response.rs
    git commit -m "refactor(models): convert status enums to v0.2 in the v0.2 module"

M2: edit the test files named in Plan of Work, then check what still imports the crate outside `server/v02`:

    grep -rln device_api_v02 agent/tests | grep -v '^agent/tests/server/v02/'

Expected exactly `agent/tests/server/tcp.rs`, `agent/tests/app/run.rs`, `agent/tests/events/model.rs` (reasons in Decisions, Left out).

    git add agent/tests/models/harnesses.rs agent/tests/models/deployment.rs agent/tests/models/os.rs agent/tests/models/file_rule.rs agent/tests/models/device.rs agent/tests/models/instance_format.rs agent/tests/logs/mod.rs agent/tests/server/v02/response.rs
    git commit -m "test(server): check status enum conversions in the v0.2 layer"

M3: edit `AGENTS.md` and `ARCHITECTURE.md`, then:

    git add AGENTS.md ARCHITECTURE.md
    git commit -m "docs: hand-write Device API enum conversions per version"

Never run cargo, `scripts/lint.sh`, `scripts/test.sh` or `scripts/covgate.sh` locally; `$preflight` pushes and CI checks.

## Validation and Acceptance

Validation runs in CI (`.github/workflows/ci.yml`) on the pushed branch head and is complete only when `$preflight` reports `CLEAN`. The PR stays in draft, and the task is not reported complete, until then.

- `lint` passes: imports sorted, no unused `agent_server` imports, clippy clean, `cargo machete` still finds `device-api-v02` used by `server/v02`.
- `test` passes with every coverage gate at or above its current value (`models` 100, `server` 87, `server/v02` 87); the seven new `From` impls are fully covered by `status_enums`.
- `windows-check` and `windows-agent-build` pass.

What the green run demonstrates: `status_enums::converts_each_*` in `agent/tests/server/v02/response.rs` (seven new tests) cover every domain variant's v0.2 value and wire string, so a wrong or missing arm fails. The existing `deployment_response`, `device_response`, `release_response` and `file_rule_response` tests pass unchanged, so `/v0.2` response bodies are unchanged. `status_enum_tests!` still covers each enum's wire strings and backend twin in `agent/tests/models/`, and the macro's inline tests still run. `agent/tests/events/model.rs` passes unchanged from PR 1.

Static acceptance, run from the repo root: `grep -rn device_api_v02 agent/src | grep -v '^agent/src/server/v02/'` prints nothing.

## Idempotence and Recovery

Every step is a file edit plus a commit; re-applying an edit changes nothing. If CI fails, fix from `gh run view --log-failed` and push again. If a coverage gate drops, add tests for the uncovered lines; never lower a `.covgate`. If PR 1 changes after this branch is cut, rebase this branch onto it. No on-disk format changes.
