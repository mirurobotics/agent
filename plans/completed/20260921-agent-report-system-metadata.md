# Report device system metadata on provision and reprovision

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Revendor the backend OpenAPI spec to agent/v0.5.1, regenerate `libs/backend-api`, and populate the new system-metadata fields on the provision and reprovision requests. |
| `mirurobotics/openapi` | read-only (release asset) | Source of truth for the vendored spec. The plan only downloads the published `agent/v0.5.1` release asset; no code is written there. |

This plan lives in this repo's `plans/backlog/` because every code change (spec vendor, regeneration, and agent source) happens here.

## Purpose / Big Picture

Today the Miru agent registers a device (provision) or rebinds an existing device to new hardware (reprovision) by POSTing a request that carries only `public_key_pem`, `agent_version`, and (provision only) `name`. The backend's agent/v0.5.1 API now also accepts five optional device-metadata fields — `os`, `arch`, `hostname`, `os_version`, `kernel_version` — and persists them on the device record. Because the agent sends none of them, those columns are always null.

After this change, a freshly provisioned or reprovisioned device reports its own OS family, CPU architecture, hostname, human-readable OS version, and kernel version. An operator inspecting the device in the Miru platform (or `GET /device`) sees, for example, `os: linux`, `arch: x86_64`, `hostname: robot-1.local`, `os_version: Ubuntu 22.04`, `kernel_version: 5.15.0-91-generic` instead of nulls. Metadata that cannot be determined on the running host is omitted from the request (never sent as an empty string), and an unrecognized OS/architecture simply omits that field rather than failing provisioning.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) M1: Revendor `api/specs/backend/v05.yaml` to the agent/v0.5.1 asset and regenerate `libs/backend-api`.
- [ ] M2: Add the system-metadata helper, populate both requests, add unit tests.
- [ ] Validation: `cargo test -p miru-agent` green locally; CI (`ci.yml`) green on the pushed branch head (preflight reports CLEAN).

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

(Add entries as you go.)

- Decision: …
  Rationale: …
  Date/Author: …

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Read this section as if you have never seen this repo. Everything you need is here.

### What the agent is

The agent is a Rust binary (`miru-agent`, package in `agent/`) that runs on devices and talks to the Miru backend over HTTP. It has a **provision** mode (register a brand-new device) and a **reprovision** flow (rebind an existing device record to new hardware). Both flows generate an RSA keypair and POST a JSON request to the backend.

### Generated backend client — never hand-edit

`libs/backend-api/` is a Rust crate **auto-generated** from an OpenAPI spec. Its request/response types live in `libs/backend-api/src/models/*.rs`. Per `ARCHITECTURE.md` and `AGENTS.md`, these files are **never hand-edited** — they are deleted and rewritten on regeneration. To change them you change the spec and regenerate.

- The vendored spec is `api/specs/backend/v05.yaml`. Its filename is fixed by `api/Makefile` (`BACKEND_FILE := specs/backend/v05.yaml`); the generator reads exactly that path.
- The spec's canonical source is the `mirurobotics/openapi` repo, published as a GitHub release. The release for this work is tag `agent/v0.5.1`, asset **`agent.yaml`**. The in-repo filename (`v05.yaml`) differs from the asset name (`agent.yaml`); only the *bytes* must match.
- Regeneration is driven by `api/regen.sh`, which runs `make gen` in `api/` and then copies the generated models into `libs/backend-api/src/models/` and `libs/device-api/src/models/` (it `rm -rf`s the target `models/*` first, so every model file is rewritten; unchanged files produce no git diff).
- `make gen` invokes `npx --yes @openapitools/openapi-generator-cli generate ... -g rust -t templates/rust`. The generator version is pinned to **7.12.0** by `api/openapitools.json`. There is **no `package.json` / `node_modules`** committed in `api/`, so **no `npm ci` is required** — `npx --yes` fetches the wrapper on demand. Prerequisites: **Node.js** (for `npx`) and a **Java runtime** (openapi-generator is a Java tool). Both are present in the dev environment used to author this plan (`node v24`, `java` on PATH); regeneration was dry-run there successfully.
- The Rust codegen uses custom Mustache templates in `api/templates/rust/` (notably `model.mustache`), which add a forward-compatible `#[serde(other)]` catch-all variant to every generated string enum.

### What agent/v0.5.1 adds to the spec

Compared to the currently vendored spec (`x-release-version: v0.5.0`), the agent/v0.5.1 asset (`x-release-version: v0.5.1`) adds two shared string-enum schemas and five optional properties to each of `ProvisionDeviceRequest`, `ReprovisionDeviceRequest`, and `UpdateDeviceFromAgentRequest` (plus `BaseDevice`/`Device` responses). All added request fields are **optional** and **non-nullable** (`os`/`arch` are bare `$ref`s, the three strings are plain `type: string`), so agents that omit them are unaffected.

- Enum `OS`: wire values `linux`, `windows`, with `x-enum-varnames` `OS_LINUX`, `OS_WINDOWS`.
- Enum `Arch`: wire values `x86_64`, `aarch64`, with `x-enum-varnames` `ARCH_X86_64`, `ARCH_AARCH64`.

### Exact generated Rust names (verified by dry-running the pinned generator 7.12.0 on the v0.5.1 asset)

The `OS` schema generates Rust type **`Os`** (file `libs/backend-api/src/models/os.rs`); the `Arch` schema generates type **`Arch`** (`arch.rs`). Both are re-exported from `libs/backend-api/src/models/mod.rs` as `pub use self::os::Os;` and `pub use self::arch::Arch;`. Variants (verbatim from `x-enum-varnames`, plus the template's catch-all):

    pub enum Os   { OS_LINUX, OS_WINDOWS, OsUnknown }        // #[serde(other)] OsUnknown, Default = OS_LINUX
    pub enum Arch { ARCH_X86_64, ARCH_AARCH64, ArchUnknown } // #[serde(other)] ArchUnknown, Default = ARCH_X86_64

Both enums derive `Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize`.

The regenerated request structs (in `provision_device_request.rs` and `reprovision_device_request.rs`) gain these fields (all with `#[serde(..., skip_serializing_if = "Option::is_none")]`, so `None` is omitted from the JSON body):

    pub os: Option<models::Os>,
    pub hostname: Option<String>,
    pub arch: Option<models::Arch>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,

`ProvisionDeviceRequest` keeps `public_key_pem`, `agent_version`, `name` (all required); `ReprovisionDeviceRequest` keeps `public_key_pem`, `agent_version`. The structs derive `Clone, Default, Debug, PartialEq, Serialize, Deserialize`, so any field left unset stays `None`.

### Where the requests are built (the sites to edit)

- `agent/src/provisioning/provision.rs`, function `provision_with_backend` (~line 82): builds `backend_client::ProvisionDeviceRequest { public_key_pem, agent_version, name }`. `backend_client` is the alias `use backend_api::models as backend_client;`.
- `agent/src/provisioning/reprovision.rs`, function `reprovision_with_backend` (~line 54): builds `backend_client::ReprovisionDeviceRequest { public_key_pem, agent_version }`.
- The HTTP transport (`agent/src/http/devices.rs`) serializes whatever the request struct holds; it needs **no** change.
- **Out of scope:** the `update` path (`UpdateDeviceFromAgentRequest`, used by `services/device` via `http::devices::update`) also gains these fields at regen time but is intentionally **not** populated here — this task is provision + reprovision only. The unset fields stay `None` and are omitted, so leaving `update` alone changes nothing on the wire. Note it as a possible follow-up; do not touch it.

### System-info facilities already in the repo

- `agent/src/telemetry/mod.rs` exposes `SystemInfo` with associated (static) functions backed by the `sysinfo` crate (workspace dep, locked at `0.38.4`; already a dependency of the `agent` crate — **no new dependency is added**):
  - `SystemInfo::host_name() -> String` = `sysinfo::System::host_name().unwrap_or_default()`
  - `SystemInfo::os() -> String` = `sysinfo::System::long_os_version().unwrap_or_default()` (this is the human-readable **os_version**)
  - `SystemInfo::arch() -> String` = `sysinfo::System::cpu_arch()`
  - There is **no** `kernel_version()` yet; `sysinfo::System::kernel_version() -> Option<String>` exists in 0.38 and must be surfaced.
- `telemetry` currently depends only on `sysinfo` (it does not import `backend_api`); keep it that way. The mapping from host strings to backend enums belongs in the provisioning layer, which already imports `backend_api`.
- `std::env::consts::OS` yields `"linux"`, `"windows"`, `"macos"`, etc.; `std::env::consts::ARCH` yields `"x86_64"`, `"aarch64"`, `"arm"`, etc. The task specifies using these compile-time constants for the machine-readable `os`/`arch` enums (not sysinfo), and sysinfo for the three human-readable strings.

### Repo conventions you must follow

- **Import ordering** (enforced by a custom linter over `agent/src` and `agent/tests`): three groups separated by a blank line and a `// standard crates` / `// internal crates` / `// external crates` comment. `backend_api` is treated as an **internal** crate (see the existing block in `provision.rs`, where `use backend_api::models as backend_client;` is the last line of the internal group).
- **Function length** (linter): production functions/closures are capped at 50 non-blank, non-comment body lines. Keep helpers small; the new functions are well under.
- **Field-by-field assertions** (linter): 4+ `assert_eq!` on fields of the *same* variable in one test triggers a finding. Prefer asserting the whole struct at once (derive `PartialEq, Debug` and compare to an expected value). This also matches the repo's "assert whole structs" convention.
- **Unit tests** live in inline `#[cfg(test)] mod tests` at the bottom of the source file they test (`shared.rs` already has one). **Integration tests** live under `agent/tests/<module>/mod.rs` and use the public API via `miru_agent::...` (`agent/tests/telemetry/mod.rs` already exists).
- **Portability:** the `windows-check` CI job runs `cargo test --package miru-agent --locked` on Windows. All new tests must pass on both Linux and Windows — use explicit input strings for the mapping tests (not the host) and derive host-dependent assertions from `std::env::consts` so they hold on either OS. Do **not** gate the new tests with `#[cfg(unix)]`.
- **Coverage gates:** `agent/src/telemetry/.covgate` = `100`, `agent/src/provisioning/.covgate` = `96.57`. New code must be exercised by tests so these gates still pass in the CI `test` job.
- **No dependency/lockfile churn:** this change adds no crate, so `Cargo.toml` and `Cargo.lock` must be untouched (keeps `--locked` CI jobs green). Do not run `cargo update`.

## Plan of Work

Two milestones, one commit each.

### Milestone 1 — Revendor spec and regenerate the client (generated-only commit)

1. Download the agent/v0.5.1 `agent.yaml` release asset and place its bytes at `api/specs/backend/v05.yaml`, replacing the v0.5.0 content. Verify byte-for-byte equality (sha256/`cmp`) against the downloaded asset. The expected sha256 of the asset (observed during authoring, 85466 bytes) is:

       9a21745dedaedf86362f26d924db4351cf2753f8f97911a5f3b11474c31663e6

   If a re-cut asset has a different digest, trust the freshly downloaded asset and record the new digest in the Decision Log; the requirement is that the vendored file equals *the asset you downloaded*, verified by `cmp`.
2. Run `api/regen.sh` to regenerate `libs/backend-api`. Expect new files `libs/backend-api/src/models/os.rs` and `arch.rs`; modified `mod.rs` (adds `pub mod os; pub use self::os::Os;` and the `arch` pair), `provision_device_request.rs`, `reprovision_device_request.rs`, `update_device_from_agent_request.rs`, `base_device.rs`, and `device.rs`. `libs/device-api` should show **no** diff (its spec `v02.yaml` is unchanged). The `api/codegen/` scratch output is gitignored (`**/codegen/`), so it will not appear in `git status`.
3. Confirm the workspace still compiles: `cargo build -p backend-api` (fast) and confirm `Cargo.lock` is unchanged.
4. Commit as a `chore(api)` change containing exactly the spec file and the regenerated `libs/backend-api/src/models/` files.

### Milestone 2 — Report system metadata (feature commit)

1. **Extend telemetry** (`agent/src/telemetry/mod.rs`): add an associated function `pub fn kernel_version() -> String` returning `System::kernel_version().unwrap_or_default()`, mirroring `host_name()`/`os()`. (No struct field is required; keep the change minimal.)
2. **Add the helper** in `agent/src/provisioning/shared.rs`:
   - Add to the internal import group: `use crate::telemetry;` and `use backend_api::models as backend_client;` (both in the internal group, following `provision.rs`'s ordering).
   - Define a small value type and builder:

         #[derive(Debug, Default, PartialEq)]
         pub(super) struct SystemMetadata {
             pub os: Option<backend_client::Os>,
             pub arch: Option<backend_client::Arch>,
             pub hostname: Option<String>,
             pub os_version: Option<String>,
             pub kernel_version: Option<String>,
         }

   - Pure mapping functions (unit-testable, no I/O):

         fn map_os(os: &str) -> Option<backend_client::Os> {
             match os {
                 "linux" => Some(backend_client::Os::OS_LINUX),
                 "windows" => Some(backend_client::Os::OS_WINDOWS),
                 _ => None,
             }
         }
         fn map_arch(arch: &str) -> Option<backend_client::Arch> {
             match arch {
                 "x86_64" => Some(backend_client::Arch::ARCH_X86_64),
                 "aarch64" => Some(backend_client::Arch::ARCH_AARCH64),
                 _ => None,
             }
         }
         fn non_empty(s: String) -> Option<String> {
             if s.trim().is_empty() { None } else { Some(s) }
         }

   - A pure builder that takes all inputs (so tests can drive it with known values) plus a thin public wrapper that reads the real host values:

         fn build_system_metadata(
             os: &str, arch: &str,
             hostname: String, os_version: String, kernel_version: String,
         ) -> SystemMetadata {
             SystemMetadata {
                 os: map_os(os),
                 arch: map_arch(arch),
                 hostname: non_empty(hostname),
                 os_version: non_empty(os_version),
                 kernel_version: non_empty(kernel_version),
             }
         }

         pub(super) fn system_metadata() -> SystemMetadata {
             build_system_metadata(
                 std::env::consts::OS,
                 std::env::consts::ARCH,
                 telemetry::SystemInfo::host_name(),
                 telemetry::SystemInfo::os(),           // long_os_version → os_version
                 telemetry::SystemInfo::kernel_version(),
             )
         }

     Rationale for placement: `shared.rs` is inside the module that constructs both requests and already the natural home for `provisioning` helpers; putting the backend-enum mapping here keeps `telemetry` free of any `backend_api` coupling.
3. **Populate `ProvisionDeviceRequest`** in `provision.rs::provision_with_backend`: build the metadata once and spread its fields (keep the existing `name` behavior):

         let meta = shared::system_metadata();
         let payload = backend_client::ProvisionDeviceRequest {
             public_key_pem,
             agent_version: version::VERSION.to_string(),
             os: meta.os,
             hostname: meta.hostname,
             arch: meta.arch,
             os_version: meta.os_version,
             kernel_version: meta.kernel_version,
             name: device_name.unwrap_or_else(telemetry::SystemInfo::host_name),
         };

4. **Populate `ReprovisionDeviceRequest`** in `reprovision.rs::reprovision_with_backend` the same way (no `name` field).
5. **Tests** (see Validation for commands):
   - Inline `#[cfg(test)] mod tests` in `shared.rs` (extend the existing module): a `mod system_metadata` covering `map_os` (linux/windows → `Some`, e.g. `macos`/`freebsd` → `None`), `map_arch` (`x86_64`/`aarch64` → `Some`, e.g. `arm`/`x86` → `None`), `build_system_metadata` with fully-populated known inputs (assert the **whole** `SystemMetadata` struct equals the expected value — single `assert_eq!`), `build_system_metadata` with empty strings + an unsupported os/arch (assert it equals `SystemMetadata::default()`, i.e. all `None`), and one `system_metadata()` smoke test asserting `.os == map_os(std::env::consts::OS)` and `.arch == map_arch(std::env::consts::ARCH)` (portable and deterministic on Linux and Windows; also covers the `system_metadata()` wrapper for the coverage gate).
   - In `agent/tests/telemetry/mod.rs`: exercise `SystemInfo::kernel_version()` so the telemetry 100% gate still passes (call it and assert it returns a `String`; do not assert non-empty, since kernel version can be blank on some hosts).
6. Commit as a `feat(provisioning)` change.

## Concrete Steps

All commands run from the repo root `agent/` checkout (this worktree) unless stated. Replace `<repo>` with the worktree path.

### Milestone 1

Download the asset to a scratch path, vendor it, and verify bytes:

    gh release download agent/v0.5.1 -R mirurobotics/openapi -p agent.yaml -O /tmp/agent-v051.yaml
    cp /tmp/agent-v051.yaml api/specs/backend/v05.yaml
    cmp /tmp/agent-v051.yaml api/specs/backend/v05.yaml && echo "BYTE-IDENTICAL"
    sha256sum api/specs/backend/v05.yaml

Expected: `cmp` prints nothing and the `&&` echoes `BYTE-IDENTICAL`; the sha256 matches the digest recorded in Plan of Work (or the freshly downloaded asset if re-cut).

Regenerate and inspect the diff:

    ./api/regen.sh
    git status --short libs/backend-api/src/models

Expected (order may vary): new `?? libs/backend-api/src/models/arch.rs`, `?? .../os.rs`; modified ` M` entries for `mod.rs`, `provision_device_request.rs`, `reprovision_device_request.rs`, `update_device_from_agent_request.rs`, `base_device.rs`, `device.rs`. `git status` for `libs/device-api` shows no changes, and `git diff --stat Cargo.lock` is empty.

Confirm it compiles and commit the generated changes:

    cargo build -p backend-api
    git add api/specs/backend/v05.yaml libs/backend-api/src/models
    git commit -m "chore(api): revendor backend spec agent/v0.5.1 and regenerate backend-api"

Commit body should note the source (agent/v0.5.1 asset agent.yaml) and that only generated models changed.

### Milestone 2

Make the source edits described in Plan of Work, then run the fast checks:

    cargo fmt -p miru-agent
    cargo test -p miru-agent provisioning::shared
    cargo test -p miru-agent --test mod telemetry
    cargo clippy -p miru-agent --all-features -- -D warnings

Expected: `cargo test -p miru-agent provisioning::shared` runs the new inline unit tests and reports `test result: ok. N passed; 0 failed`; the telemetry integration filter passes; clippy is clean. For a full local pass you may also run `cargo test -p miru-agent` (or `./scripts/test.sh`, which sets `RUST_LOG=off`).

Commit the feature:

    git add agent/src/telemetry/mod.rs agent/src/provisioning/shared.rs \
            agent/src/provisioning/provision.rs agent/src/provisioning/reprovision.rs \
            agent/tests/telemetry/mod.rs
    git commit -m "feat(provisioning): report device system metadata on provision and reprovision"

### Push and drive CI to green (before the PR leaves draft)

    git push -u origin feat/agent-report-system-metadata

Then open the PR **as a draft** and watch the `ci.yml` run on the pushed branch head (see Validation). Do not mark the PR ready — and do not report this task complete — until CI is green (preflight reports CLEAN).

## Validation and Acceptance

### Unit / integration tests (local, fast feedback)

- New behavior under test: OS/architecture mapping (including unsupported → `None`) and the metadata builder's field population (including empty/unsupported → `None`).
- `cargo test -p miru-agent provisioning::shared` — expect the `system_metadata` test group to pass. Each mapping/builder test **fails before** the helper exists (it will not compile / the module is absent) and **passes after** the change.
- `cargo test -p miru-agent --test mod telemetry` — expect the telemetry tests, including the new `kernel_version()` exercise, to pass.
- Full suite: `cargo test -p miru-agent` (or `./scripts/test.sh`) — expect `test result: ok` with `0 failed`.

### Observable end-to-end behavior

The request body is what changes on the wire. Acceptance is that a provision request now includes the populated optional fields and omits any that are unavailable. Concretely, in `build_system_metadata`:

- Input `os = "linux"`, `arch = "x86_64"`, `hostname = "robot-1.local"`, `os_version = "Ubuntu 22.04"`, `kernel_version = "5.15.0-91-generic"` yields `SystemMetadata { os: Some(OS_LINUX), arch: Some(ARCH_X86_64), hostname: Some("robot-1.local"), os_version: Some("Ubuntu 22.04"), kernel_version: Some("5.15.0-91-generic") }`.
- Input `os = "macos"`, `arch = "arm"`, and empty strings yields `SystemMetadata::default()` (every field `None`), so serialization (with `skip_serializing_if`) omits all five keys — the request is byte-compatible with today's and provisioning does not fail.

Because the fields use `skip_serializing_if = "Option::is_none"`, an unset field never appears as an empty string in the JSON body.

### CI / preflight (authoritative gate)

Heavy validation runs in GitHub Actions via `ci.yml`, not locally. The relevant jobs are `lint` (custom import/funclen/assert linter, `cargo fmt --check`, `cargo machete`, `cargo-audit`, `cargo clippy -D warnings`), `test` (runs `./scripts/covgate.sh` — tests plus per-module coverage gates), and `windows-check` (`cargo test --package miru-agent --locked` on Windows).

**Validation requirement:** preflight must report **CLEAN** — i.e. the `ci.yml` workflow must be **green on the pushed branch head** — before this PR is moved out of draft and before the task is reported complete. If any CI job is red, fix it from the job logs and push again; re-check until green. A local pass is necessary but not sufficient; the pushed-head CI result is the gate.

## Idempotence and Recovery

- **Vendor step** is idempotent: re-downloading and re-copying the asset produces the same bytes; `cmp` re-verifies. If the digest differs on a re-cut asset, use the freshly downloaded bytes and record the new digest in the Decision Log.
- **Regeneration** is idempotent and destructive-by-design: `api/regen.sh` deletes and rewrites `libs/backend-api/src/models/*`. To recover from a bad regen, `git checkout -- libs/backend-api` (and `api/specs/backend/v05.yaml`) restores the tree, then re-run. The `api/codegen/` scratch dir is gitignored and safe to delete (`rm -rf api/codegen`).
- **Source edits** are ordinary, reversible Rust changes; `git checkout -- <file>` reverts any single file. No migrations, no on-disk state, no destructive runtime behavior.
- **No dependency changes:** if `Cargo.lock` shows a diff after these steps, something unrelated ran (e.g. `cargo update`); revert it with `git checkout -- Cargo.lock` so `--locked` CI jobs stay green.
- Each milestone is a single commit, so the branch can be bisected and any milestone rolled back independently.
