# Make OpenAPI client enums forward-compatible via the generator pipeline (not hand-edits)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/user/agent` (root repo, single git repo — `git rev-parse --show-toplevel` == `/home/user/agent`) | read-write | Change the OpenAPI generator pipeline (`api/templates/rust/model.mustache`, `api/specs/device/v02.yaml`, `api/Makefile`/PATH), regenerate `libs/backend-api/src/models/*` and `libs/device-api/src/models/*`, revert hand-edits, and add forward-compat tests in the `miru-agent` crate (`agent/src/models/`). |

This plan lives in `/home/user/agent/plans/backlog/` because all code changes are in this single repository. (The repo uses `plans/{backlog,active,completed}/`, not the policy's default `.agents/exec-plans/`; follow the repo convention.) It supersedes the rejected plan `plans/completed/20260515-backend-api-forward-compatible-enums.md`, whose hand-edit approach a reviewer flagged as a P1 `AGENTS.md` violation.

## Purpose / Big Picture

Today, a fleet agent that receives a `Device` or `Deployment` JSON payload from the backend containing an enum value the agent's generated client does not know about (e.g. a new `DeviceStatus` like `"rebooting"` shipped by a newer backend) fails to deserialize the *entire* payload, because the OpenAPI-generated Rust enums under `libs/backend-api/src/models/` and `libs/device-api/src/models/` use a plain `#[derive(Deserialize)]` that rejects unknown strings. This makes the agent brittle against backend rollouts.

After this change, every string enum the generator emits for both API specs carries a `#[serde(other)]` catch-all variant, so an unrecognized status string deserializes into that catch-all instead of erroring; the agent's existing domain mapping (`impl_status_enum!` in `agent/src/models/status.rs`) then maps that catch-all to a safe domain default with a log line. Crucially, the catch-all is produced by the *generator template*, so it survives `api/regen.sh` and does not violate `AGENTS.md:76` ("`libs/backend-api/` and `libs/device-api/` are auto-generated ... Do not edit by hand").

Observable outcome: `cargo test --package miru-agent --features test` passes, including new tests proving a `Deployment`/`Device` payload with an unknown status string still deserializes `Ok`, and `agent/src/models/.covgate` (100%) passes. `scripts/preflight.sh` reports clean except for the two pre-existing, unrelated `deploy/filesys.rs` root-vs-chmod failures.

## Progress

- [ ] (2026-05-15) Plan authored (backlog).
- [ ] Milestone 1 — Revert hand-edits so `libs/` and agent test code are exactly generator output / clean.
- [ ] Milestone 2 — Fix `api/specs/device/v02.yaml` YAML so the device spec parses under openapi-generator 7.12.0.
- [ ] Milestone 3 — Add the `#[serde(other)]` catch-all to `api/templates/rust/model.mustache`.
- [ ] Milestone 4 — Make `make gen` runnable (openapi-generator-cli on PATH) and run `api/regen.sh`; verify the `libs/` diff contains ONLY intended catch-all additions.
- [ ] Milestone 5 — Update agent code so all 5 `impl_status_enum!` instantiations compile against regenerated enums (new catch-all variant names).
- [ ] Milestone 6 — Add agent-crate forward-compat tests (5 unknown→default, known→exact, and Deployment/Device payload deserialization) so `agent/src/models/.covgate` 100% passes.
- [ ] Milestone 7 — Validate with `scripts/preflight.sh` / `scripts/covgate.sh`; confirm only the 2 known unrelated `deploy/filesys.rs` failures remain.
- [ ] Milestone 8 — Rebase branch `claude/hunt-agent-repo-190Ta`, reworking old commits; force-with-lease push to update existing PR #75 (do NOT open a new PR).

## Surprises & Discoveries

- Observation: `--additional-properties=enumUnknownDefaultCase=true` on the openapi-generator 7.12.0 Rust generator does NOT produce a serde-tolerant catch-all. It adds a normal named variant `UnknownDefaultOpenApi` annotated `#[serde(rename = "unknown_default_open_api")]`. An unknown string still fails to deserialize because nothing maps arbitrary unknown strings into it.
  Evidence: generated `device_status.rs` with this option contained `#[serde(rename = "unknown_default_open_api")] UnknownDefaultOpenApi,` — a plain rename, no `#[serde(other)]`. Therefore option (a) is rejected; option (b) (template override) is used.

- Observation: `api/specs/device/v02.yaml` currently FAILS to generate under openapi-generator 7.12.0 (independent of our enum work). Line 269 contains a YAML *folded* description with a literal `` `: heartbeat` `` token; SnakeYAML reads the `: ` as a mapping indicator, parsing aborts, and the generator then throws an NPE in `DefaultCodegen.specVersionGreaterThanOrEqualTo310`. This was introduced by commit `f9b0f02` ("send immediate heartbeat on SSE connection open (#74)"). The backend spec `api/specs/backend/v04.yaml` generates fine.
  Evidence: generator log `MarkedYAMLException: mapping values are not allowed here ... in 'reader', line 269, column 50`; no `out/src/models/` produced. Re-running with a `>-` block scalar and the colon quoted as `": heartbeat"` generated all device models successfully.

- Observation: `api/Makefile` invokes the bare command `openapi-generator-cli`, which is the npm wrapper binary. It is NOT on PATH in this environment; only `npx @openapitools/openapi-generator-cli` works (it downloads and pins the 7.12.0 jar; network IS available here). `make gen` fails with `openapi-generator-cli: No such file or directory` until the wrapper is on PATH.
  Evidence: `make gen-device` → `make: openapi-generator-cli: No such file or directory`. `npx @openapitools/openapi-generator-cli version` → `7.12.0`.

- Observation: `scripts/covgate.sh` measures region coverage only for files under `agent/src` (`SRC_DIR="agent/src"`, `find agent/src -name .covgate`). It does NOT measure `libs/`. The 100% gate at `agent/src/models/.covgate` therefore constrains `agent/src/models/*` (including the `impl_status_enum!` macro expansions), not the generated `libs/` enums. Tests for forward-compat must live in the agent crate to satisfy the covgate; tests inside `libs/` neither help nor are allowed (hand-edit of generated files).
  Evidence: `scripts/covgate.sh` exports `SRC_DIR="agent/src"`; `scripts/lib/covgate.sh` discovers via `find "$SRC_DIR" -name '.covgate'`.

- Observation: The generator-produced catch-all variant is named `{ClassName}UnknownValue` (e.g. `DeviceStatusUnknownValue`, `DeploymentActivityStatusUnknownValue`), NOT the hand-edit's `DEVICE_STATUS_UNKNOWN_VALUE`. Existing agent test `agent/src/models/deployment.rs:396` references the old hand-edit name `DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_UNKNOWN_VALUE` and must be updated to the new generated name.
  Evidence: regenerated `git_repository_type.rs` contained `#[serde(other)] GitRepositoryTypeUnknownValue,`.

## Decision Log

- Decision: Use option (b), a custom mustache override `api/templates/rust/model.mustache`, NOT option (a) `enumUnknownDefaultCase`.
  Rationale: empirically verified option (a) emits a plain named variant without `#[serde(other)]`, which does not make deserialization tolerant; option (b) emits a true `#[serde(other)]` catch-all for every string enum in both specs and was proven to deserialize unknown strings (including nested struct fields) successfully in a standalone `serde_json` test.
  Date/Author: 2026-05-15 / Claude (plan author).

- Decision: Fix `api/specs/device/v02.yaml` line 268-269 by converting the response description to a `>-` block scalar and quoting the literal colon token as `": heartbeat"`.
  Rationale: the device spec is unparseable by the pinned generator otherwise, so regeneration of `libs/device-api/` is impossible without this. This is a content-preserving doc edit (the rendered description text is unchanged). It is in scope because forward-compat for `device-api` enums (used by `agent/src/models/device.rs`) cannot be delivered without regenerating that crate.
  Date/Author: 2026-05-15 / Claude (plan author).

- Decision: Make `openapi-generator-cli` resolvable for `make gen` by prepending the repo's npm bin to PATH (`api/node_modules/.bin`) — installing the wrapper locally if absent — rather than editing the `api/Makefile` command. If a Makefile change is preferred by the implementer, change the two `openapi-generator-cli` invocations to `npx --yes @openapitools/openapi-generator-cli`; either is acceptable as long as `api/regen.sh` runs end-to-end and `openapitools.json`'s 7.12.0 pin is honored.
  Rationale: keep the pipeline reproducible; do not silently fall back to hand-edits.
  Date/Author: 2026-05-15 / Claude (plan author).

- Decision: Keep the `impl_status_enum!` wildcard arms added in commit `650a780` to `agent/src/models/status.rs` (the `status =>` arm in the `@base` Deserialize impl and the `other =>` arm in `From<&$backend_type>`); they are correct and required for covgate. Only the *generated-file* hand-edits and *libs-resident tests* are reverted.
  Rationale: the prompt explicitly says the wildcard arm is correct; covgate needs every macro expansion's wildcard arm exercised by agent tests.
  Date/Author: 2026-05-15 / Claude (plan author).

## Outcomes & Retrospective

(To be completed at implementation.)

## Context and Orientation

Read this section assuming no prior knowledge of the repo.

### What the generator pipeline is

- `api/openapitools.json` pins `openapi-generator-cli` to version `7.12.0`.
- `api/Makefile` has targets `gen-backend` and `gen-device`. Each runs:
  `openapi-generator-cli generate -i <spec> -g rust -t templates/rust -o codegen/<name> --additional-properties=packageName=<name>-api`
  Specs: `api/specs/backend/v04.yaml` (backend) and `api/specs/device/v02.yaml` (device).
- `api/templates/rust/` is the custom template dir. It currently contains only `partial_header.mustache`; every other Rust template falls back to the generator's built-in templates. Adding `model.mustache` there overrides the built-in model template (which emits all enums and model structs).
- `api/regen.sh` runs `make gen` (from `api/`), then deletes and replaces `libs/backend-api/src/models/*` with `api/codegen/backend/src/models/*` and `libs/device-api/src/models/*` with `api/codegen/device/src/models/*`. So whatever the generator emits *is* the content of `libs/.../models/`.

### What the affected enums are

Generated string enums needing the catch-all. In `libs/backend-api/src/models/`: `device_status.rs` (`DeviceStatus`), `deployment_status.rs` (`DeploymentStatus`), `deployment_activity_status.rs` (`DeploymentActivityStatus`), `deployment_error_status.rs` (`DeploymentErrorStatus`), `deployment_target_status.rs` (`DeploymentTargetStatus`), `instance_format.rs` (`InstanceFormat`), `git_repository_type.rs` (`GitRepositoryType`). In `libs/device-api/src/models/`: `device_status.rs`, `deployment_status.rs`, `deployment_activity_status.rs`, `deployment_error_status.rs`, `deployment_target_status.rs` (its own copies). The template change covers ALL string enums in both specs automatically — no per-enum work.

### How the agent consumes them

`agent/src/models/status.rs` defines `macro_rules! impl_status_enum!`. It has three arms:
- The public 5-tuple arm (with `backend_type`) — used by the 4 deployment enums.
- The public 4-tuple arm (no `backend_type`) — used by `DeviceStatus`.
- An internal `@base` arm that generates: a hand-written `Deserialize` impl (with a `status =>` wildcard arm that logs + returns the domain default), `variants()`, `as_str()`, and `From<&$name> for $agent_type`.
The 5-tuple arm additionally generates `From<&$name> for $backend_type` and `From<&$backend_type> for $name` (the latter has an `other =>` wildcard arm that logs + returns the domain default).

Instantiations (5 total):
- `agent/src/models/deployment.rs`: `DplTarget`, `DplActivity`, `DplErrStatus`, `DplStatus` — each `backend_type: backend_client::Deployment*Status` (alias `backend_client = backend_api::models`), `agent_type: agent_server::Deployment*Status` (alias `agent_server = device_api::models`).
- `agent/src/models/device.rs`: `DeviceStatus` — 4-tuple form, `agent_type: agent_server::DeviceStatus` (device-api), no `backend_type`.

Key data flow that exposes the bug: `agent/src/models/deployment.rs::Deployment::from_backend` takes a `backend_client::Deployment` (the generated struct) whose fields are generated enums with `#[derive(Deserialize)]`. When the agent deserializes a backend `Deployment` JSON, serde must first deserialize those generated enum fields; an unknown string there fails the *whole* `backend_client::Deployment`. The agent's own `impl_status_enum!` Deserialize only applies to its domain enums (`DplActivity` etc.), not to the generated `backend_client::*` enums. Hence the catch-all must be on the *generated* enums. Same for `device_api::models::DeviceStatus` consumed via `agent/src/models/device.rs` and `agent/src/models/deployment.rs`'s `agent_server::*` server-side payloads.

### Covgate

`scripts/covgate.sh` → `scripts/lib/covgate.sh` runs `cargo llvm-cov --json --package miru-agent --features test`, then for each `agent/src/**/.covgate` checks region coverage ≥ the threshold on line 1. `agent/src/models/.covgate` contains `100`. Every macro expansion line in `agent/src/models/*` (including each instantiation's wildcard arms) must be covered. `libs/` is NOT measured.

### Pre-existing unrelated failures (out of scope)

Two `agent/src/deploy/filesys.rs` tests fail in this environment because they depend on running as non-root vs `chmod` semantics. They are unrelated to this change and must NOT be fixed here; just note them when reporting validation.

### Current branch / PR state

Branch: `claude/hunt-agent-repo-190Ta`. Commits to be reworked: `650a780` (feat: hand-edit generated enums + add macro wildcard arm), `1cf1def` (test: tests inside `libs/` + agent), `3a60033` (style: rustfmt), plus plan-lifecycle commits `045e76a`, `e6dde4c`. PR #75 already exists for this branch; this work updates it via force-with-lease after rebase. Do NOT open a new PR.

## Plan of Work

The work is a clean replacement of the hand-edit approach with a generator-pipeline approach. Sequence:

1. Revert hand-edits and libs-resident tests.
   - From the working tree, restore every file under `libs/backend-api/src/models/` and `libs/device-api/src/models/` to the state at the merge-base with `main` (commit `f9b0f02` is the last pre-branch commit on `main`'s history; use it as the pristine generated baseline). Concretely: `git checkout f9b0f02 -- libs/backend-api/src/models libs/device-api/src/models`. This wipes the catch-all hand-edits AND the `#[cfg(test)] mod tests` blocks that commit `1cf1def` added inside `libs/backend-api/src/models/{deployment_activity_status.rs,deployment.rs,device.rs}`.
   - Keep `agent/src/models/status.rs` macro changes from `650a780` (they are correct). Keep the structure of `agent/src/models/deployment.rs` and `device.rs` but the test module `backend_unknown_mapping_tests` in `deployment.rs` will be rewritten in Milestone 6 (the variant name changes).

2. Fix the device spec YAML so it parses.
   - File: `api/specs/device/v02.yaml`. The current lines (1-indexed) 268-272 are a folded block:

         description: SSE event stream. Each event is delivered as an SSE frame with
           `id`, `event`, and `data` fields. A `: heartbeat` comment is sent
           immediately when the connection opens, and again every 30 seconds
           while the stream is idle. Comments carry no data and should be
           ignored by clients.

   - Replace with an explicit block scalar and a quoted colon token (rendered text unchanged):

         description: >-
           SSE event stream. Each event is delivered as an SSE frame with
           `id`, `event`, and `data` fields. A ": heartbeat" comment is sent
           immediately when the connection opens, and again every 30 seconds
           while the stream is idle. Comments carry no data and should be
           ignored by clients.

   - Preserve the exact YAML indentation of the surrounding `responses:` → `'200':` block (the `description:` key keeps its current column; continuation lines indented one level deeper under the `>-`). After editing, re-check there is no other unquoted `: ` inside a folded/plain scalar in the file (search `grep -n '`: ' api/specs/device/v02.yaml` and inspect any plain-scalar descriptions). Verified during authoring that this is the only offending location.

3. Add the catch-all to the model template.
   - Create `api/templates/rust/model.mustache` by extracting the stock 7.12.0 Rust `model.mustache` (see Concrete Steps for the exact `author template` command) and applying exactly two minimal edits, both only in the non-integer string-enum blocks (do NOT touch integer-repr enums, model structs, oneOf, or discriminator code):

   Edit A — top-level enum block. Locate this exact text:

         {{/enumVars}}{{/allowableValues}}
         }

         impl std::fmt::Display for {{{classname}}} {
             fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                 match self {
                     {{#allowableValues}}
                     {{#enumVars}}
                     Self::{{{name}}} => write!(f, "{{{value}}}"),
                     {{/enumVars}}
                     {{/allowableValues}}
                 }
             }
         }

   Replace with (insert catch-all variant before the enum `}`, and a matching Display arm before the inner closing `}`):

         {{/enumVars}}{{/allowableValues}}
             /// Catch-all for values added by the API after this client was
             /// generated. `#[serde(other)]` makes unrecognized strings
             /// deserialize here instead of failing the whole payload.
             #[serde(other)]
             {{{classname}}}UnknownValue,
         }

         impl std::fmt::Display for {{{classname}}} {
             fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                 match self {
                     {{#allowableValues}}
                     {{#enumVars}}
                     Self::{{{name}}} => write!(f, "{{{value}}}"),
                     {{/enumVars}}
                     {{/allowableValues}}
                     Self::{{{classname}}}UnknownValue => write!(f, "unknown_value"),
                 }
             }
         }

   Edit B — inline (model property) enum block. Locate this exact text (inside the `{{^isInteger}}` branch of the `{{#vars}}{{#isEnum}}` section):

         #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
         pub enum {{{enumName}}} {
         {{#allowableValues}}
         {{#enumVars}}
             #[serde(rename = "{{{value}}}")]
             {{{name}}},
         {{/enumVars}}
         {{/allowableValues}}
         }
         {{/isInteger}}

   Replace with:

         #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
         pub enum {{{enumName}}} {
         {{#allowableValues}}
         {{#enumVars}}
             #[serde(rename = "{{{value}}}")]
             {{{name}}},
         {{/enumVars}}
         {{/allowableValues}}
             /// Catch-all for values added by the API after this client was generated.
             #[serde(other)]
             {{{enumName}}}UnknownValue,
         }
         {{/isInteger}}

   Notes: this keeps `#[derive(Clone, Copy, ...)]` valid (the catch-all is a unit variant). `Default` still resolves to `enumVars.0.name` (first known variant) — unchanged. Known values round-trip byte-for-byte identically (their `#[serde(rename)]` and Display arms are untouched). Only the integer-repr enum blocks and non-enum code are left exactly as stock.

4. Make `make gen` runnable and regenerate.
   - Ensure `openapi-generator-cli` resolves on PATH for the `api/Makefile`. Preferred: install the npm wrapper locally under `api/` and prepend `api/node_modules/.bin` to PATH for the regen invocation. Alternative accepted by Decision Log: change the two `openapi-generator-cli` lines in `api/Makefile` to `npx --yes @openapitools/openapi-generator-cli`. Whichever is used, `openapitools.json`'s 7.12.0 pin must be honored (the wrapper reads it).
   - Run `api/regen.sh`. It runs `make gen` then copies generated models over `libs/{backend,device}-api/src/models/`.

5. Verify the regenerated `libs/` diff.
   - `git diff -- libs/` must show ONLY: (a) the added `#[serde(other)] {Name}UnknownValue` variant + its `Display` arm in each string-enum file in both crates, and (b) possibly a benign header line difference if the generator now emits `* The version of the OpenAPI document: ...` (the stock template includes it; the repo's current files were generated without it because of how `partial_header.mustache` interacts). If unrelated diffs appear (e.g. struct reordering), STOP and reconcile: re-extract the stock template for the *exact* 7.12.0 version and re-apply only the two edits; the diff must be limited to the catch-all (plus at most the OpenAPI-version header line, which is acceptable and consistent across all files). Document any header-line delta in Surprises & Discoveries and Decision Log.

6. Update agent code for the new variant names.
   - `agent/src/models/deployment.rs` test reference at (old) line ~396 uses `backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_UNKNOWN_VALUE`. The regenerated name is `backend_client::DeploymentActivityStatus::DeploymentActivityStatusUnknownValue`. Update all such references. The `impl_status_enum!` macro itself does NOT name the catch-all variant (its `From<&$backend_type>` uses a generic `other =>` arm), so the macro needs no change for the catch-all; it already compiles against any extra enum variant. Confirm device-api `DeviceStatus` likewise gains `DeviceStatusUnknownValue` so the `DeviceStatus` instantiation's wildcard arm is reachable/testable.

7. Add agent-crate forward-compat tests (Milestone 6 detail in Validation).

8. Validate, rebase, force-with-lease push to PR #75.

## Concrete Steps

All commands run from `/home/user/agent` unless stated. Show expected transcript fragments.

Step 1 — revert generated-file hand-edits and libs tests:

    git checkout f9b0f02 -- libs/backend-api/src/models libs/device-api/src/models
    git status --short libs/    # expect: clean (no diff vs f9b0f02)

Step 2 — fix device spec YAML (use the Edit tool, not sed). After editing:

    grep -n '`: ' api/specs/device/v02.yaml || echo "no risky colon tokens remain"

Step 3 — extract stock template and apply the two edits:

    cd /home/user/agent/api
    mkdir -p /tmp/stock-rust-tpl
    npx --yes @openapitools/openapi-generator-cli author template -g rust -o /tmp/stock-rust-tpl
    cp /tmp/stock-rust-tpl/model.mustache templates/rust/model.mustache
    # then apply Edit A and Edit B (Plan of Work step 3) with the Edit tool
    grep -n 'serde(other)' templates/rust/model.mustache
    # expect 2 functional occurrences (lines in the top-level and inline enum blocks)

Step 4 — make generator resolvable and regenerate:

    cd /home/user/agent/api
    # Option chosen per Decision Log; e.g. ensure wrapper present:
    npx --yes @openapitools/openapi-generator-cli version   # expect: 7.12.0
    cd /home/user/agent
    bash api/regen.sh
    # expect: make gen runs gen-backend then gen-device with no Java exception;
    #         api/codegen/{backend,device}/src/models populated; libs/ updated.

Step 5 — inspect the libs diff:

    git diff --stat -- libs/
    git diff -- libs/backend-api/src/models/device_status.rs
    # expect added:  #[serde(other)]  DeviceStatusUnknownValue,  + Display arm
    git diff -- libs/device-api/src/models/device_status.rs
    # expect the analogous addition in device-api

Verify EVERY affected enum got it:

    grep -rl 'serde(other)' libs/backend-api/src/models libs/device-api/src/models
    # expect all string-enum files listed (device_status, deployment_status,
    # deployment_activity_status, deployment_error_status,
    # deployment_target_status, instance_format[backend], git_repository_type[backend])

Step 6 — update agent references:

    grep -rn 'UNKNOWN_VALUE\|UnknownValue' agent/src/
    # update agent/src/models/deployment.rs test to use DeploymentActivityStatusUnknownValue
    cargo build --package miru-agent --features test
    # expect: clean build

Step 7 — run tests / covgate (see Validation).

Step 8 — rebase & push:

    git rebase ...            # rework 650a780/1cf1def/3a60033 into coherent commits
    # (squash the hand-edit + revert noise; final commits: spec fix, template, regen,
    #  agent macro arm, agent tests, plan move to completed)
    git push --force-with-lease origin claude/hunt-agent-repo-190Ta
    # Do NOT run `gh pr create`. PR #75 updates automatically.

## Validation and Acceptance

Milestone 6 — add these tests in the `miru-agent` crate (NOT in `libs/`):

In `agent/src/models/deployment.rs` (`#[cfg(test)] mod backend_unknown_mapping_tests`, rewritten):
- For EACH of `DplTarget`, `DplActivity`, `DplErrStatus`, `DplStatus`: a test feeding the corresponding `backend_client::Deployment*Status::{Name}UnknownValue` through `(&backend).into()` and asserting it equals the domain default (`DplTarget::Staged`, `DplActivity::Drifted`, `DplErrStatus::None`, `DplStatus::Drifted`). This exercises the `other =>` wildcard arm of the 5-tuple macro for all 4 deployment instantiations.
- For EACH of the 4: a known-value test asserting `(&backend::KnownVariant).into()` maps to the exact expected domain variant.
- A `Deployment` payload test: build a JSON string for `backend_client::Deployment` with `activity_status` set to an unrecognized string (e.g. `"some_future_status"`); assert `serde_json::from_str::<backend_client::Deployment>(...)` is `Ok` and that `Deployment::from_backend` yields `activity_status == DplActivity::Drifted`.

In `agent/src/models/device.rs` (`#[cfg(test)] mod tests` or a new module):
- A test feeding `agent_server::DeviceStatus::DeviceStatusUnknownValue` (device-api) through `(&backend).into()` for the `DeviceStatus` instantiation, asserting it equals `DeviceStatus::Offline` (the declared default). This exercises the 4-tuple macro's agent-side wildcard arm. Also a known-value test (`online`→`Online`, `offline`→`Offline`).
- Note: `DeviceStatus`'s `impl_status_enum!` uses `agent_type: agent_server::DeviceStatus` and the macro's `@base` `Deserialize` `status =>` arm. Add a serde test: `serde_json::from_str::<DeviceStatus>("\"rebooting\"")` returns `Ok(DeviceStatus::Offline)` (covers the `@base` wildcard arm for the `DeviceStatus` expansion).
- A `Device` payload test: a JSON object whose `status` is an unknown string still deserializes `Ok` into `agent` `Device` with `status == DeviceStatus::Offline`.

Also exercise the `@base` Deserialize wildcard arm for the 4 deployment domain enums (e.g. `serde_json::from_str::<DplActivity>("\"xyz\"")` == `Ok(DplActivity::Drifted)`), one per enum, so all 5 `@base` expansions' wildcard arms and all 4 `From<&backend>` wildcard arms are covered (covgate requires 100% region coverage of `agent/src/models/`).

Run and expected results:

    cargo test --package miru-agent --features test 2>&1 | tail -20
    # expect: all new tests pass; pre-existing suite green except the 2 known
    #         deploy/filesys.rs root/chmod failures.

    scripts/covgate.sh 2>&1 | tail -30
    # expect: "✅ models: 100% (requires 100%)" among module lines.
    # If models < 100%, the failing arm is reported by cargo-llvm-cov; add the
    # missing instantiation's unknown/known test until 100%.

    scripts/preflight.sh 2>&1 | tail -40
    # expect: "Preflight clean" OR a failure attributable ONLY to the 2 known
    #         unrelated deploy/filesys.rs tests. Any models/covgate/lint failure
    #         is in scope and must be fixed.

Acceptance (behavior a human can verify):
- Before: `serde_json::from_str::<backend_api::models::Deployment>` of a payload whose `activity_status` is `"some_future_status"` returns `Err`. After: returns `Ok`, and `agent::models::Deployment::from_backend` yields `activity_status == DplActivity::Drifted` with a warn log.
- `git diff -- libs/` after `api/regen.sh` contains only the catch-all additions (plus at most a consistent OpenAPI-version header line) — no hand-edits remain; `AGENTS.md:76` is satisfied because the change lives in `api/templates/rust/model.mustache`.
- `cargo test --package miru-agent --features test` passes; `agent/src/models/.covgate` reports 100%.

## Idempotence and Recovery

- Step 1 (`git checkout f9b0f02 -- libs/...`) is idempotent — re-running restores the same pristine baseline.
- Step 3 template extraction overwrites `/tmp/stock-rust-tpl`; safe to re-run. If the two edits were misapplied, delete `api/templates/rust/model.mustache`, re-copy the stock file, re-apply.
- Step 4 `api/regen.sh` is idempotent: it `rm -rf`s `codegen/` and the `libs/.../models/*` targets before copying, so re-running produces identical output. If `make gen` fails on the device spec, the YAML fix (Step 2) was not applied or another `: ` token exists — fix and re-run.
- Recovery for a wrong `libs/` diff: `git checkout f9b0f02 -- libs/` then re-run `api/regen.sh` after correcting the template; never hand-edit `libs/` to "fix" the diff.
- Network risk: `npx @openapitools/openapi-generator-cli` needs network to fetch the 7.12.0 jar on first use (verified available in the authoring environment). If a future run has no network and the jar is not cached, regeneration is blocked: in that case do NOT revert to hand-edits — the source of truth is `api/templates/rust/model.mustache` + the spec fix; commit those and document that `libs/` must be regenerated when network is available, leaving the pipeline change as the authoritative artifact.
- Push recovery: `--force-with-lease` aborts if the remote moved; fetch, rebase onto the updated remote branch, re-push. Never `--force` and never open a new PR.

---

Revision note (2026-05-15, initial authoring): Plan created to replace the rejected hand-edit approach (`plans/completed/20260515-backend-api-forward-compatible-enums.md`). Option (a) `enumUnknownDefaultCase` was empirically tested and rejected (emits a plain renamed variant, not `#[serde(other)]`); option (b) template override was empirically verified to produce serde-tolerant catch-alls for both specs. Discovered and scoped two pipeline blockers: the device spec YAML folded-scalar parse failure (commit `f9b0f02`) and `openapi-generator-cli` not being on PATH.
