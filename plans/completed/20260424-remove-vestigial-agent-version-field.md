# Remove vestigial `agent_version` field from local `models::Device`

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Remove the `agent_version` field from the local `models::Device` struct and its `Updates` patch type, drop the now-unused `Updates::set_agent_version` constructor, simplify `setup::bootstrap` to take `version: &str` directly, drop the redundant assignment in `app::upgrade::ensure`, stop stamping the field in `provision::entry::provision`, and update tests that asserted on or constructed the field. |
| `agent/libs/backend-api/` | read-only | The generated `backend_api::models::Device` and `backend_api::models::UpdateDeviceFromAgentRequest` keep their `agent_version` field — that is the wire contract with the backend and is unchanged by this plan. |

This plan lives in `agent/plans/backlog/` because all code changes are inside the `agent/` repo.

## Purpose / Big Picture

After the idempotent-upgrade-rebootstrap work landed earlier on this branch, the agent's local source of truth for "what version last bootstrapped this device" is the plain-text marker file at `Layout::agent_version()` (i.e. `/var/lib/miru/agent_version` on a deployed device). The local `models::Device` struct still carries an `agent_version: String` field, but it is now written in four places (deserialize default, `Default::default`, `From<&backend_api::models::Device>`, the `Patch` impl) and read in exactly one — `storage::setup::bootstrap` forwards `device.agent_version` into `storage::setup::reset` so reset can write the marker. That is a redundant round-trip: every caller of `bootstrap` already knows the running version (it is the compile-time constant `version::VERSION`), so it can pass it explicitly.

After this change a maintainer reading `models::Device` sees only fields the agent actually consults. There is no behavior change visible to operators: the wire format with the backend (`UpdateDeviceFromAgentRequest { agent_version: Some(version::VERSION.to_string()) }`, `ProvisionDeviceRequest::agent_version`) is unchanged, and the marker file still records what last bootstrapped. Acceptance is "the field is gone, the agent compiles, all tests pass, and `scripts/preflight.sh` prints `Preflight clean`."

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Read this plan end-to-end before starting.
- [ ] Edit `agent/src/models/device.rs`: remove the `agent_version` field, the `Default` initializer line, the `DeserializeAgent` field + the unwrap_or_else block that builds it, the line in `From<&backend_api::models::Device>`, the `Patch` impl arm, the `Updates.agent_version` field, the `Updates::empty` initializer line, and the `Updates::set_agent_version` function. Also drop the `assert_eq!(device.agent_version, …)` line from the in-file unit test `from_openapi_device_maps_fields`.
- [ ] Edit `agent/src/storage/setup.rs`: change `bootstrap`'s signature to take `version: &str` and replace the `&device.agent_version` argument to `reset` with that new parameter.
- [ ] Edit `agent/src/provision/entry.rs`: pass `version::VERSION` (already imported) into `bootstrap`, and stop relying on `(&device).into()` to populate the field. The `From<&backend_api::models::Device>` impl is being changed in the same edit so the conversion no longer mentions the field; `provision_with_backend` itself does not need to change because the `ProvisionDeviceRequest` wire type still has its own `agent_version: String` field stamped from `version::VERSION`.
- [ ] Edit `agent/src/app/upgrade.rs`: delete the line `device_model.agent_version = version.to_string();` (it becomes a compile error after the field is removed, and the PATCH at line 113 already sources from `version` directly). The line `let mut device_model …` becomes `let device_model …` — the `mut` is no longer needed.
- [ ] Edit `agent/src/storage/mod.rs` if needed: the `Storage::init` site at line 93 constructs a `models::Device` with `..models::Device::default()` and only sets `id`, `activated`, `status` — it does not name the removed field, so this site should compile unchanged. Verify after editing the model that no further changes are needed here.
- [ ] Edit `agent/tests/models/device.rs`:
  - Remove the `OptionalField { key: "agent_version", … }` entry from the `optional_fields()` vec (around line 42–46).
  - Remove the `agent_version: "placeholder".to_string(),` line from the `defaults` test struct literal (line 86).
  - Remove the `agent_version: "v1.0.0".to_string(),` lines from the `merge_empty` (line 174) and `merge_all` (line 194) test struct literals.
  - Remove the `agent_version: Some("v1.0.1".to_string()),` line from the `merge_all` `Updates { … }` literal (line 205) and the `agent_version: updates.agent_version.clone().unwrap(),` line from the expected literal (line 215).
  - Remove the `agent_version: None,` line from the `updates_empty` test (line 234).
  - Delete the entire `updates_set_agent_version` test (lines 279–287).
- [ ] Edit `agent/tests/storage/setup.rs`: the existing `assert_marker(&layout, expected_version)` helper (defined inside `pub mod reset`) is the new way to verify post-bootstrap version state. The `bootstrap` tests in this file currently call `validate_storage(&layout)` which only checks that `device.json` deserializes to `Device::default()`. Since `Device::default()` no longer carries `agent_version`, those assertions still hold by virtue of struct equality. Add an explicit marker check to `bootstrap::clean_install` at minimum: after `storage::setup::bootstrap(&layout, &device, &settings, &private_key_file, &public_key_file, "v0.0.0").await.unwrap();`, assert that `storage::agent_version::read(&layout.agent_version()).await.unwrap() == Some("v0.0.0".into())`. Update every other `bootstrap` test in this file to pass the new `version: &str` argument (currently nine call sites: `src_public_key_file_doesnt_exist`, `src_private_key_file_doesnt_exist`, `clean_install`, `device_file_already_exists`, `auth_directory_already_exists`, `private_key_file_already_exists`, `public_key_file_already_exists`, `storage_directory_already_exists`, `events_directory_already_exists`). Use `"v0.0.0"` for tests that don't otherwise care about the value.
- [ ] Edit `agent/tests/provision/entry.rs`: there are no current assertions on `device.agent_version` (the test inspects `device_json["device_id"]` and `device_json["name"]`); the file should compile unchanged. Verify this is still the case after editing the model.
- [ ] Edit `agent/tests/app/upgrade.rs`:
  - Delete line 134 (`assert_eq!(on_disk_device.agent_version, "v0.9.0");`) inside `ensure_rebootstraps_when_marker_missing`. The marker check on lines 119–122 already proves the version was recorded.
  - Leave line 75 alone (`agent_version: Some("v0.0.0".to_string())` is on the `backend_client::Device`, the wire type, which retains the field).
- [ ] Edit `agent/tests/server/response.rs`: remove the `agent_version: "1.0.0".into(),` line at line 24 and `agent_version: "2.0.0".into(),` at line 53 — both literals construct local `models::Device` values whose field no longer exists. The expected `openapi::Device` literals on lines 32–40 and 61–69 already do not mention `agent_version` (the OpenAPI device-server type has no such field), so no further test edit is needed.
- [ ] Edit `agent/tests/services/device/get.rs`: remove the `agent_version: "1.2.3".to_string(),` line at line 72 inside the `returns_custom_device_data` test — same reason (constructs a local `models::Device`).
- [ ] Run `scripts/preflight.sh` from the `agent` repo root and verify it prints `Preflight clean` (lint, format, all tests, all covgate modules at threshold). Do not publish until this is clean.
- [ ] Sanity-check covgate: the `models` module currently sits at 100% coverage. Removing both the field and its tests removes lines from numerator and denominator together. If any covgate module ends up below threshold after this change, the right action is to delete the now-redundant tests rather than add filler — see the warning in the Notes section.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

(Add entries as you go.)

- Decision: Drop `agent_version` from `models::Device` and `models::device::Updates`; keep it on the wire types.
  Rationale: After `sync::agent_version::push` was removed earlier on this branch, the field on `device.json` is written in four places and read in one — `setup::bootstrap` forwards it into `reset` to write the marker. That is a redundant round-trip. The marker file is now the single local source of truth and the backend PATCH already sources from `version::VERSION` directly, so the local field has no remaining consumers. Date/Author: 2026-04-24 / ben@miruml.com.

- Decision: `setup::bootstrap` takes `version: &str` explicitly rather than threading it through `models::Device`.
  Rationale: `setup::reset` already takes the version separately; the only caller of `bootstrap` (`provision::entry`) already has `version::VERSION` in scope. An explicit parameter makes the dependency obvious and avoids re-introducing a "device-as-config" anti-pattern. Date/Author: 2026-04-24 / ben@miruml.com.

- Decision: Wire format (`UpdateDeviceFromAgentRequest`, `ProvisionDeviceRequest`, `backend_api::models::Device`) is unchanged.
  Rationale: The backend's `Device` resource still has `agent_version` and the agent still PATCHes it during rebootstrap (`agent/src/app/upgrade.rs` line 112). Removing the local mirror does not change the contract. Date/Author: 2026-04-24 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The agent runs on a customer device and persists state under `/var/lib/miru/`. The relevant files for this task:

- `agent/src/models/device.rs` — defines `pub struct Device` (the in-memory + on-disk representation that the agent actually uses) and its `Updates` patch type. Contains a custom `Deserialize` impl that tolerates older `device.json` files missing newer fields, and a `From<&backend_api::models::Device>` conversion that promotes the wire-type response from the backend into the local model. The `Updates::set_agent_version` constructor was the only code path that set the field after install, and it is no longer called anywhere after `sync::agent_version::push` was removed.
- `agent/src/storage/setup.rs` — `bootstrap` is the installer entry point (called by `provision::entry::provision`), `reset` is the shared wipe-and-write body called by both `bootstrap` and `app::upgrade::ensure`. `reset` already takes `agent_version: &str` and writes the marker file at the end; `bootstrap` currently extracts it from `device.agent_version` instead of taking it as a parameter.
- `agent/src/storage/agent_version.rs` — read/write helpers for the plain-text marker file at `Layout::agent_version()` (`/var/lib/miru/agent_version`). This is the new local source of truth for "what version last bootstrapped this device."
- `agent/src/app/upgrade.rs` — `ensure` is the boot-time idempotent upgrade gate. It checks the marker, and if it differs from the running `version::VERSION` it calls `setup::reset` with the new version and then PATCHes the backend with `agent_version: Some(version.to_string())`. Currently also writes `device_model.agent_version = version.to_string();` before calling reset; that line becomes dead the moment the field is removed.
- `agent/src/provision/entry.rs` — `provision` is the install-time entry point. It generates a keypair, calls `POST /devices/provision` (wire payload includes its own `agent_version: version::VERSION.to_string()` field on `ProvisionDeviceRequest`, unchanged), converts the response into a local `models::Device`, and then calls `setup::bootstrap`. The conversion currently picks up `agent_version` via the `From` impl that stamps `version::VERSION`; after the field is removed, the conversion stops mentioning it and the bootstrap call gains an explicit `version::VERSION` argument.
- `agent/src/storage/mod.rs` — `Storage::init` constructs a default `models::Device` at line 93 with `..models::Device::default()` and overrides `id`, `activated`, `status`. It does not name the removed field, so it should compile unchanged.
- `agent/src/server/response.rs` — `From<&models::Device> for device_api::models::Device` (the device-side OpenAPI type sent over the local control-plane gRPC) does not currently use `agent_version` and does not need to change. The matching test `agent/tests/server/response.rs` does mention the field in literals and needs editing.

Key term: **marker file**. `/var/lib/miru/agent_version` is a plain-text file containing the version string of the agent that last completed a successful bootstrap or rebootstrap. Its presence + matching contents is what `app::upgrade::ensure` checks at boot to decide whether to re-enter the rebootstrap flow.

## Plan of Work

1. **`agent/src/models/device.rs`** — Remove every reference to `agent_version` inside the file. Specifically:
   - Delete the `pub agent_version: String,` field from `pub struct Device` (line 40).
   - Delete the `agent_version: "placeholder".to_string(),` line from the `Default` impl (line 54).
   - In the custom `Deserialize` impl: delete the `agent_version: Option<String>,` line from `DeserializeAgent` (line 76) and the `agent_version: result.agent_version.unwrap_or_else(...)` block (lines 103–105) from the constructed `Device { … }` literal.
   - In `From<&backend_api::models::Device> for Device`: delete the `agent_version: crate::version::VERSION.to_string(),` line (line 132).
   - In `Patch<Updates> for Device`: delete the `if let Some(agent_version) = patch.agent_version { … }` block (lines 150–152).
   - In `pub struct Updates`: delete the `pub agent_version: Option<String>,` field (line 175).
   - In `Updates::empty`: delete the `agent_version: None,` initializer (line 188).
   - Delete the entire `pub fn set_agent_version(version: String) -> Self { … }` function (lines 213–218).
   - In the in-file `mod tests` block: delete the `assert_eq!(device.agent_version, crate::version::VERSION);` line (line 239) from `from_openapi_device_maps_fields`.

2. **`agent/src/storage/setup.rs`** — Change `bootstrap`'s signature from
   `pub async fn bootstrap(layout: &Layout, device: &models::Device, settings: &Settings, private_key_file: &filesys::File, public_key_file: &filesys::File) -> Result<(), StorageErr>`
   to add `version: &str` as the last parameter. Change the final line from `reset(layout, device, settings, &device.agent_version).await` to `reset(layout, device, settings, version).await`. Update the rustdoc on `bootstrap` to mention that the caller supplies the version explicitly.

3. **`agent/src/provision/entry.rs`** — In `provision`, change the `storage::setup::bootstrap(...)` call to pass `version::VERSION` as the new last argument. The `version` module is already imported. No other change.

4. **`agent/src/app/upgrade.rs`** — Delete line 94 (`device_model.agent_version = version.to_string();`). Change the binding two lines above from `let mut device_model: models::Device = (&device).into();` to `let device_model: models::Device = (&device).into();`. The `setup::reset(layout, &device_model, &settings, version)` call already passes `version` and does not change.

5. **`agent/src/storage/mod.rs`** — No code change required. After step 1, re-confirm that `Storage::init`'s `models::Device { id: …, activated: true, status: …, ..models::Device::default() }` still type-checks (it does not name the removed field).

6. **Tests** — see Progress for the per-file checklist. The pattern is: every test that constructs a local `models::Device` literal with an `agent_version: …` line, and every test that exercises `Updates::set_agent_version` or asserts on `device.agent_version`, needs to drop those lines. Tests that exercise the wire types (`backend_api::models::Device.agent_version`, `UpdateDeviceFromAgentRequest.agent_version`, `ProvisionDeviceRequest.agent_version`) are unchanged.

7. **Add a marker assertion to `bootstrap::clean_install`** in `agent/tests/storage/setup.rs`: after the bootstrap call, assert the marker file matches the version string passed in. This converts what was previously an implicit assertion (via `device.agent_version == "placeholder"` round-tripping through `device.json`) into an explicit one (the marker file is the new source of truth).

8. **Validate** by running `scripts/preflight.sh` from the agent repo root. Iterate until it prints `Preflight clean`.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent/` (the agent repo root) unless noted.

1. Read the current files to remind yourself of the layout. The Bash tool is fine; commands like

       grep -n agent_version agent/src/models/device.rs

   will quickly enumerate the field's appearances inside a single file.

2. Apply the source edits in order: `agent/src/models/device.rs`, `agent/src/storage/setup.rs`, `agent/src/provision/entry.rs`, `agent/src/app/upgrade.rs`. Do not try to fix tests yet.

3. Run a quick sanity build to confirm src compiles:

       cargo check -p miru-agent

   Expected: clean compile of the library; tests will still fail to build at this stage because the test files still mention the removed field.

4. Apply the test edits enumerated in Progress and Plan of Work. Then:

       cargo test -p miru-agent --no-run

   Expected: all test crates build with no `no field `agent_version` on type` errors.

5. Run the test suite to confirm behavior:

       cargo test -p miru-agent

   Expected: all tests pass. The new marker assertion in `bootstrap::clean_install` should pass on the first try; if it fails, the most likely cause is forgetting to thread `version` through one of the nine `bootstrap` test call sites.

6. Run the full preflight gate:

       scripts/preflight.sh

   Expected final line: `Preflight clean`. If any covgate module is below threshold, see the Notes / Warnings section before reaching for filler tests.

7. Stage and commit when preflight is clean. The `task` workflow handles the actual branch + PR mechanics; do not push from inside the implement step.

## Validation and Acceptance

Acceptance is `scripts/preflight.sh` printing `Preflight clean` on the `feat/idempotent-upgrade-reset` branch with the changes applied. Specific behaviors to observe along the way:

- `cargo check -p miru-agent` succeeds after the source edits and before the test edits — confirming the source-side change is internally consistent.
- `cargo test -p miru-agent --no-run` succeeds after the test edits — confirming all test crates compile.
- `cargo test -p miru-agent` reports the same number of passing tests as before, minus the deleted `updates_set_agent_version` test, and the new explicit marker-file assertion in `bootstrap::clean_install` passes.
- `scripts/preflight.sh` prints `Preflight clean` on its final line. The script runs lint, format, tests, and per-module covgate thresholds in parallel and prints `Preflight FAILED (lint=… tests=… tools_lint=… tools_tests=…)` on any failure.

Behavioral acceptance for an operator: there is none — this is a refactor with no user-visible change. The wire format with the backend is unchanged; the on-disk marker file is unchanged; the rebootstrap flow's observable side effects are identical.

## Notes / Warnings

- **Coverage**: removing code without removing the corresponding tests-of-removed-code is the right shape — coverage should stay steady or improve. If a covgate module drops below threshold, the right action is to delete the now-redundant tests rather than add filler tests just to pad numerator. Specifically, the `Updates::set_agent_version` constructor is gone, so its test (`updates_set_agent_version`) goes with it; do not invent a replacement test.
- **`agent/tests/server/response.rs`**: do not be tempted to add an assertion that the OpenAPI conversion drops the version — the OpenAPI type has no such field and the conversion is already symmetrically silent on the topic.
- **Wire model is intentional**: the `agent_version` field on `backend_api::models::Device`, `backend_api::models::UpdateDeviceFromAgentRequest`, and `backend_api::models::ProvisionDeviceRequest` stays. Test references such as `agent/tests/app/upgrade.rs:75` (sets it on the `backend_client::Device` mock response) and `agent/tests/http/devices.rs:143` (sets it on `UpdateDeviceFromAgentRequest`) are correct as-is.
- **`mut` keyword**: after deleting the `device_model.agent_version = version.to_string();` line in `app::upgrade::ensure`, the `let mut device_model` binding is no longer mutated. Drop the `mut` to keep clippy happy.
- **`Updates::empty` symmetry**: when removing the `agent_version: None,` line from `Updates::empty`, double-check that no existing call site of `Updates { agent_version: …, ..Updates::empty() }` still names the field. A repo-wide grep for `agent_version` after the source edits should surface anything missed.

## Idempotence and Recovery

All steps are local and re-runnable. If the build fails partway through:

- Source edits are atomic per file; re-running `cargo check -p miru-agent` will tell you what is still wrong.
- Test edits are mechanical; re-run `cargo test -p miru-agent --no-run` and grep the error output for `agent_version` to find the next site to fix.
- `git restore <path>` is the recovery path for any individual file you want to back out.
- `scripts/preflight.sh` is idempotent: running it twice in a row produces the same result.

There is nothing destructive in this plan. No data migrations, no schema changes on disk (the marker file is already the source of truth and was written by earlier commits on this branch), no wire-format changes.
