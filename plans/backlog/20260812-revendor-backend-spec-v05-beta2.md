# Re-vendor the backend API spec to `agent/v0.5.0-beta.2` and flip the agent to the file-rules vocabulary

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.


## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `/home/ben/miru/workbench1/repos/agent` | read-write | The Miru agent (Rust). All edits in this plan happen here. Branch `feat/revendor-spec-v05-beta2` is already created and checked out at `main`'s head. |
| `/home/ben/miru/workbench1/repos/openapi` | read-only reference | Source of truth for the backend API spec. The vendored spec artifact is downloaded from its GitHub releases, not copied from this checkout. |

This plan lives in `agent/plans/` because every file changed is inside the agent repo; the openapi repo is consulted only as a reference and is never modified.


## Purpose / Big Picture

The agent talks to the Miru backend over a versioned HTTP API. The API's shape is described by an OpenAPI specification (a YAML document describing endpoints and JSON schemas). The agent repo *vendors* (checks in a copy of) that spec at `api/specs/backend/v04.yaml`, and runs a code generator over it to produce Rust structs under `libs/backend-api/src/models/`. Those generated structs are called the **wire models**. Hand-written agent code converts wire models into **domain models** (the agent's own types, e.g. `agent/src/models/file_rule.rs`).

Backend API `v0.5` renames and reshapes the concept formerly called an "upload rule" into a **file rule**. A file rule tells the agent which files on disk to watch (`source`), optionally where to upload them (`upload`), and optionally how long to keep them (`retention`).

A prior change (referred to below as "PR 1") already introduced the *domain* types `FileRule`, `FileRuleSource`, `FileRuleUpload`, `FileRuleRetention` in `agent/src/models/file_rule.rs`, plus a temporary **adapter**: `impl From<backend_client::BaseUploadRule> for FileRule`. The adapter fakes the new shape out of the old v0.4 wire models — it invents `name` from `upload_collection_name`, always sets `upload: Some(..)`, and synthesizes `retention` from the old `UploadDeletePolicy` enum.

This plan is "PR 4 — Wire flip" of the umbrella plan `agent/plans/active/20260809-adopt-file-rules-spec-v0.5.md`. After it, the vendored spec is v0.5.0-beta.2, the wire models natively have `BaseFileRule`, and the adapter becomes a direct near-1:1 field copy. Nothing about agent behavior changes for the user other than that the agent now speaks v0.5 to the backend.

Observable outcome: `rg 'upload_rule|UploadRule|upload_rules' agent/src agent/tests` returns no matches for wire-level vocabulary, `api/specs/backend/v05.yaml` exists with `x-release-version: v0.5.0-beta.2`, and `./scripts/preflight.sh` reports `CLEAN`.

**Merge coordination (important):** this is a coordinated breaking change. The agent cannot ship v0.5 until the backend actually serves v0.5, because old-shape and new-shape file rules are not mutually deserializable. The pull request for this work stays a **draft** until the backend is serving v0.5.


## Progress

- [ ] Milestone 1 — Vendor the v0.5.0-beta.2 spec artifact; delete `v04.yaml`; point `api/Makefile` at it. Commit.
- [ ] Milestone 2 — Run `api/regen.sh`; review the regenerated wire models; confirm `libs/device-api` is unchanged. Commit.
- [ ] Milestone 3 — Replace the PR-1 adapter in `agent/src/models/file_rule.rs` with the direct `BaseFileRule → FileRule` mapping. Commit.
- [ ] Milestone 4 — Flip call sites: expansion string literals, `UploadRulesNotExpanded` → `FileRulesNotExpanded` (two error enums), `upload_rule_id` → `file_rule_id`. Commit.
- [ ] Milestone 5 — Update test fixtures (incl. new required `slot_key`), add adapter unit tests, run full validation. Commit.


## Surprises & Discoveries

(Add entries as you go.)


## Decision Log

- **2026-08-12 — Vendor the stamped release asset, not the raw bundle (deviation from the umbrella plan).** The umbrella plan instructs copying `apis/apps/backend-server/agent/openapi.gen.yaml` from the openapi repo and hand-stamping `info.version`, `info.x-release-version`, `info.x-git-commit`, and substituting `$API_VERSION$` placeholders. That recipe is stale: the currently vendored `api/specs/backend/v04.yaml` is the *release artifact* produced by the openapi release workflow, not the raw bundle. It carries `info.x-git-commit.{sha,url,message,author,branch,dirty}` and `info.x-build.built_at`, and uses a different YAML dumper style. Following the umbrella recipe literally would produce a ~2764-line pure-reformatting diff and drop the build/author metadata. Instead we download the `agent.yaml` asset attached to the GitHub release for tag `agent/v0.5.0-beta.2`, which is already fully stamped and contains zero `$API_VERSION$` placeholders. See Milestone 1.

(Add further entries as work proceeds.)


## Outcomes & Retrospective

(Summarize at completion or major milestones.)


## Context and Orientation

All commands below run from `/home/ben/miru/workbench1/repos/agent` unless stated otherwise. Confirm your branch first:

    cd /home/ben/miru/workbench1/repos/agent
    git branch --show-current   # expect: feat/revendor-spec-v05-beta2

### How code generation works here

`api/Makefile` drives the generator. Only three variables matter:

    BACKEND_FILE := specs/backend/v04.yaml     # the one line this plan changes
    DEVICE_FILE  := specs/device/v02.yaml
    RUST_TEMPLATES_DIR := templates/rust

`make gen` cleans `api/codegen/` and then runs, for each of backend and device:

    npx --yes @openapitools/openapi-generator-cli generate -i <FILE> -g rust \
      -t templates/rust -o codegen/<backend|device> \
      --additional-properties=packageName=<backend|device>-api

`api/regen.sh` runs `make gen` inside `api/`, then `rm -rf`s and wholesale-replaces **both** `libs/backend-api/src/models/` and `libs/device-api/src/models/` from the generated output. Prerequisites: Node/`npx` and a Java JRE on `PATH`. The generator version is pinned to 7.12.0 in `api/openapitools.json`; the Rust toolchain is pinned by `rust-toolchain.toml`.

Generated code under `libs/backend-api/` and `libs/device-api/` is **never hand-edited**. Clippy warnings originating inside generated code are expected and ignored.

A repo-wide grep for `v04` / `specs/backend` shows the filename appears in exactly one functional place — `api/Makefile` line 1. Everything else is historical prose in `plans/completed/*.md` and `plans/active/20260809-adopt-file-rules-spec-v0.5.md`. No CI workflow, script, doc, or Rust source references the spec filename.

There is **no CI drift gate** between the spec and the generated models. The regen diff must be eyeballed by a human in review; Milestone 2 tells you exactly what to look for.

### What actually changes in the spec (v0.4 vendored → v0.5.0-beta.2)

Roughly 1212 semantic diff lines. The complete inventory:

Schemas removed (4): `BaseUploadRule`, `UploadRuleSource`, `UploadRuleDestination`, `UploadDeletePolicy`.

Schemas added (4): `BaseFileRule`, `FileRuleSource`, `FileRuleUpload`, `FileRuleRetention`.

Schemas changed (16):

- `Release.upload_rules` → `Release.file_rules`, items now `BaseFileRule`.
- `ReleaseExpansion` enum member `upload_rules` → `file_rules` (generated Rust variant name `RELEASE_EXPAND_UPLOAD_RULES` → `RELEASE_EXPAND_FILE_RULES`), and the `release_expansions` query-parameter example.
- `BaseUpload`, `Upload`, `UploadWithCredentials`, `CreateUploadRequest`: field `upload_rule_id` → `file_rule_id`.
- `BaseConfigInstance` and `ConfigInstance` (which inherits via `allOf`): **new required field** `slot_key` (`{type: string, example: controller_2}`, "The key of the config schema slot this instance is bound to"). `Deployment` examples gain `slot_key: default`.
- `UploadSource.file_path`: description only.
- `APIVersion` enum `v0.4` → `v0.5`; `ReleaseVersion` `v0.4.1` → `v0.5.0-beta.2`; `APIGitCommit`; `MiruVersion` parameter example.
- `InstanceFormat` gains enum members `xml` and `text` (generated variants `INSTANCE_FORMAT_XML`, `INSTANCE_FORMAT_TEXT`). Verified inert: no hand-written code outside `libs/` references `InstanceFormat` or `INSTANCE_FORMAT_*`. It will still show up in the regen diff — that is expected, not a problem.

The `APIVersion` change is what makes the agent send `Miru-Version: v0.5`; it is picked up automatically from the generated constant, with no hand-written change.

The API base path `/agent/v1` is version-agnostic and does **not** change. The device API is untouched.

### Domain type vs new spec type

`agent/src/models/file_rule.rs` (around line 50) defines the domain `FileRule`. Against spec `BaseFileRule`:

| Domain `FileRule` | Spec `BaseFileRule` |
| --- | --- |
| `id: FileRuleID` (= `String`) | `id: string`, required |
| `name: String` | `name: string`, required |
| `digest: String` | `digest: string`, required |
| `source: FileRuleSource` | `$ref FileRuleSource`, required |
| `upload: Option<FileRuleUpload>` | `allOf FileRuleUpload`, optional |
| `retention: Option<FileRuleRetention>` | `allOf FileRuleRetention`, optional |
| `created_at: DateTime<Utc>` | `string`/`date-time`, required |
| `updated_at: DateTime<Utc>` | `string`/`date-time`, required |
| — | `object: enum[file_rule]`, required — ignored by the domain type |

`FileRuleSource` — domain `{glob: String, stability_window_secs: i64}`, spec identical, both fields required. Exact match.

`FileRuleUpload` — domain `{upload_collection_id, upload_collection_name, bucket_id, bucket_name, path}`, all `String`; spec identical, all five required. Exact match.

`FileRuleRetention` — domain `{require_upload: bool, ttl_secs: u64}`. **Two mismatches**:

1. The spec marks only `ttl_secs` required; `require_upload` is optional ("Present exactly when the rule has an `upload` block"). The generated field is therefore `Option<bool>`. Map with `.unwrap_or(false)` — semantically correct, since no upload block means there is nothing to wait on.
2. The spec's `ttl_secs` is `int64`, generated as `i64`; the domain uses `u64`. Clamp with `ttl_secs.max(0) as u64`.

Note that after regen the spec type is also named `FileRuleSource`, colliding by name with the domain type. The file already aliases `use backend_api::models as backend_client;`, which resolves it — keep the alias and add a short comment.

### Repo conventions (from `AGENTS.md`)

- Tests: `./scripts/test.sh` (equals `RUST_LOG=off cargo test --features test`). The `--features test` flag is mandatory; plain `cargo test` will not build.
- Lint: run `./scripts/update-deps.sh` first (refreshes `Cargo.lock`), then `./scripts/lint.sh` (import linter, `cargo fmt`, machete/diet, audit, clippy).
- Coverage: `./scripts/covgate.sh`, with per-directory threshold files named `.covgate`.
- `./scripts/preflight.sh` runs four jobs in parallel: `scripts/lint.sh`, `scripts/covgate.sh`, `tools/lint/scripts/lint.sh` (with `LINT_FIX=0`), `tools/lint/scripts/covgate.sh`.
- Import ordering: three blank-line-separated groups commented `// standard crates`, `// internal crates`, `// external crates`.
- Errors use `thiserror::Error` plus `crate::errors::Error` plus the `impl_error!` macro.
- A custom linter rule flags four or more `assert_eq!` calls on fields of the same variable inside one test function; suppress with a `// lint:allow(field-by-field-assert)` comment on the test.
- CI is `.github/workflows/ci.yml`, workflow name `CI`, jobs `lint`, `test`, `tools`.


## Plan of Work

Five milestones, each ending in a commit from the agent repo root. The work is mechanical: vendor a new spec, regenerate, then rename and reshape at every hand-written call site. The only genuinely new logic is the two-line mismatch handling in `FileRuleRetention`, and the only genuinely new risk is coverage: `agent/src/models/.covgate` is `100`, and the simplified adapter has fewer branches than the old one, so each remaining branch needs an explicit test.

Order matters. Milestones 2 through 4 leave the tree **not compiling** in between (regen renames types that hand-written code still uses). That is expected; do not try to keep every intermediate commit green. The tree must compile and pass at the end of Milestone 5.


## Concrete Steps

### Milestone 1 — Vendor the spec and point the Makefile at it

Working directory: `/home/ben/miru/workbench1/repos/agent`.

Download the release asset. A recorded environment constraint applies: `gh release download` and `gh pr edit` are broken here (they fail through GraphQL, and `gh pr edit` fails *silently*, dropping the write). Use `gh api` REST throughout.

    ASSET_ID=$(gh api repos/mirurobotics/openapi/releases/tags/agent%2Fv0.5.0-beta.2 \
      --jq '.assets[] | select(.name=="agent.yaml") | .id')
    echo "$ASSET_ID"   # expect a non-empty numeric id
    gh api -H "Accept: application/octet-stream" \
      repos/mirurobotics/openapi/releases/assets/$ASSET_ID > api/specs/backend/v05.yaml

Sanity-check the downloaded file. It is already fully stamped; **no hand-editing of `info:` is needed**.

    wc -l api/specs/backend/v05.yaml                     # expect ~2400
    grep -c '\$API_VERSION\$' api/specs/backend/v05.yaml  # expect 0
    rg 'x-release-version' api/specs/backend/v05.yaml     # expect v0.5.0-beta.2
    rg -n 'version: v0\.5' api/specs/backend/v05.yaml     # info.version: v0.5

Expected embedded values: `info.version: v0.5`, `info.x-release-version: v0.5.0-beta.2`, `info.x-git-commit.sha: 76d489f858e2076150e2ced2b9923cbd3c043622`, `schemas.APIVersion.enum: [v0.5]`, `schemas.ReleaseVersion.enum: [v0.5.0-beta.2]`, `schemas.APIGitCommit.enum: [76d489f8...]`, `parameters.MiruVersion.schema.example: v0.5`.

If `grep -c '$API_VERSION$'` returns non-zero or the file is HTML/JSON rather than YAML, the download failed — delete the file and retry the two `gh api` commands.

Remove the old spec and repoint the Makefile:

    git rm api/specs/backend/v04.yaml

Edit `api/Makefile` line 1, changing `specs/backend/v04.yaml` to `specs/backend/v05.yaml`. That is the only line in the repo that must change for the filename.

Verify nothing else references the old filename:

    rg -n 'v04\.yaml' --glob '!plans/**'   # expect no matches

Commit:

    git add api/specs/backend/v05.yaml api/Makefile
    git commit -m "chore(api): vendor backend spec agent/v0.5.0-beta.2"

### Milestone 2 — Regenerate the wire models

Working directory: `/home/ben/miru/workbench1/repos/agent`.

    ./api/regen.sh

This takes a couple of minutes (it downloads the generator via `npx` on first run).

Then inspect what changed:

    git status --short libs/
    git diff --stat libs/

Expected and required:

- `git status libs/device-api` is **clean**. The device spec was not touched, so its regenerated output must be byte-identical. If it is dirty, stop and investigate before committing.
- `libs/backend-api/src/models/` shows: new files for `base_file_rule.rs`, `file_rule_source.rs`, `file_rule_upload.rs`, `file_rule_retention.rs`; deleted files for `base_upload_rule.rs`, `upload_rule_source.rs`, `upload_rule_destination.rs`, `upload_delete_policy.rs`; and edits to release, upload, config-instance, and version models.

Eyeball the diff against the spec-delta inventory in Context and Orientation. Specifically confirm:

    rg -n 'file_rules' libs/backend-api/src/models/release.rs
    rg -n 'RELEASE_EXPAND_FILE_RULES' libs/backend-api/src/models/
    rg -n 'file_rule_id' libs/backend-api/src/models/
    rg -n 'slot_key' libs/backend-api/src/models/base_config_instance.rs
    rg -n 'v0\.5' libs/backend-api/src/models/api_version.rs
    rg -n 'INSTANCE_FORMAT_XML|INSTANCE_FORMAT_TEXT' libs/backend-api/src/models/

Also confirm the generated `require_upload` is `Option<bool>` and `ttl_secs` is `i64` in `libs/backend-api/src/models/file_rule_retention.rs`, and note the exact parameter order of the generated `BaseConfigInstance::new(..)` / `ConfigInstance::new(..)` — Milestone 5 needs it.

Do **not** attempt to build yet; hand-written code still uses the removed types and will not compile until Milestone 4.

If device generation fails: a completed plan (`plans/completed/20260515-forward-compatible-enums-via-generator.md`, line 38) records that device generation once failed under generator 7.12.0 on a YAML folded-scalar parse bug. `regen.sh` uses `set -e`, so it aborts *after* the backend models were already replaced, leaving a half-applied tree. Recover with `git checkout -- libs/` and re-run.

Commit:

    git add libs/
    git commit -m "chore(api): regenerate backend wire models for v0.5.0-beta.2"

### Milestone 3 — Flip the adapter in `agent/src/models/file_rule.rs`

Working directory: `/home/ben/miru/workbench1/repos/agent`. File: `agent/src/models/file_rule.rs`.

Three edits. Leave `impl<'de> Deserialize<'de> for FileRule` (the on-disk snapshot format) completely alone — it is unaffected by the wire change.

**(a)** Change `impl From<backend_client::UploadRuleSource> for FileRuleSource` (around line 19) to take `backend_client::FileRuleSource`. Body is unchanged (both fields map straight across). Add a one-line comment noting that `backend_client::FileRuleSource` is the wire type and `FileRuleSource` (unqualified) is the domain type of the same name, disambiguated by the `backend_client` alias.

**(b)** Add a `From<backend_client::FileRuleUpload> for FileRuleUpload` conversion (all five `String` fields copy straight across), or inline the construction in (c) — either is fine; prefer the standalone `From` impl for symmetry and testability.

**(c)** Replace `impl From<backend_client::BaseUploadRule> for FileRule` (around line 76) with `impl From<backend_client::BaseFileRule> for FileRule`. Delete the entire `UploadDeletePolicy` match, the `*rule.destination` unwrap, and the `name: rule.upload_collection_name.clone()` placeholder. The new body:

    impl From<backend_client::BaseFileRule> for FileRule {
        fn from(rule: backend_client::BaseFileRule) -> FileRule {
            FileRule {
                id: rule.id,
                name: rule.name,
                digest: rule.digest,
                source: (*rule.source).into(),
                upload: rule.upload.map(|u| (*u).into()),
                retention: rule.retention.map(|r| FileRuleRetention {
                    // the spec marks require_upload optional: it is present exactly
                    // when the rule has an upload block. absent => nothing to wait on.
                    require_upload: r.require_upload.unwrap_or(false),
                    // the spec types ttl_secs as int64; the domain uses u64.
                    ttl_secs: r.ttl_secs.max(0) as u64,
                }),
                created_at: /* unchanged parse-with-fallback block */,
                updated_at: /* unchanged parse-with-fallback block */,
            }
        }
    }

Keep the existing `created_at` / `updated_at` parse-with-`error!`-and-fall-back-to-`UNIX_EPOCH` blocks verbatim. Adjust the `Box`/`Option<Box<..>>` dereferences to match whatever the generator actually emitted for `source`, `upload`, and `retention` — check `libs/backend-api/src/models/base_file_rule.rs` and follow it exactly rather than assuming.

Commit (the tree still does not compile — that is expected at this point):

    git add agent/src/models/file_rule.rs
    git commit -m "refactor(models): map BaseFileRule directly to the FileRule domain type"

### Milestone 4 — Flip the remaining call sites

Working directory: `/home/ben/miru/workbench1/repos/agent`.

**Expansion string literals.** The agent asks the backend to inline related objects via an `expand[]` query parameter. Change:

| File | Current | Becomes |
| --- | --- | --- |
| `agent/src/services/backend.rs:58` | `&["upload_rules"]` | `&["file_rules"]` |
| `agent/src/sync/deployments.rs:134` | `"release.upload_rules"` | `"release.file_rules"` |
| `agent/src/sync/deployments.rs:249-250` | doc comment mentioning `expand=release.upload_rules` and an `upload_rules` array | reword to `file_rules` |

The `expand[]` list is assembled generically at `agent/src/http/releases.rs:13` via `QueryParams::new().expand(expansions)` — there is no literal there to change.

**Generated field rename `Release.upload_rules` → `Release.file_rules`.** Update the field accesses at `agent/src/services/release/get.rs:23` and `agent/src/sync/deployments.rs:271`.

**Error variants — there are two, in two different enums.**

In `agent/src/services/errors.rs`: rename `UploadRulesNotExpandedErr` (struct, ~line 12) to `FileRulesNotExpandedErr`, and the enum variant `ServiceErr::UploadRulesNotExpanded` (~line 35) to `ServiceErr::FileRulesNotExpanded`, updating the `From` impl (~line 80) and the `impl_error!` entry (~line 94). Change the message (~line 11) to:

    release '{release_id}' did not have file_rules expansion (backend did not expand file_rules)

In `agent/src/sync/errors.rs`: rename `UploadRulesNotExpandedErr` (struct with `deployment_id`, ~line 75) to `FileRulesNotExpandedErr`, and `SyncErr::UploadRulesNotExpanded` (~line 108) to `SyncErr::FileRulesNotExpanded`, updating the `From` impl (~line 153) and the `impl_error!` entry (~line 172). Change the message (~line 74) to:

    deployment '{deployment_id}' release did not have file_rules expansion (backend did not expand release.file_rules)

Neither error defines a custom `code()`, so there are **no wire error-code constants to change**.

Non-test call sites to update: `agent/src/services/release/get.rs:7,24` and `agent/src/sync/deployments.rs:272`.

**Generated field rename `upload_rule_id` → `file_rule_id`.** `agent/src/data_uploads/upload/executor.rs:142` builds a generated `CreateUploadRequest` with `upload_rule_id: job.file_rule_id.clone()`. The struct field must become `file_rule_id:` or it will not compile. (The right-hand side already reads `job.file_rule_id` — that is a domain field and is already correct.)

Optional and explicitly **out of scope** (PR 5 territory): the test-helper functions named `upload_rule()` in `agent/src/data_uploads/scan/scanner.rs` (~30 call sites) and `agent/tests/data_uploads/upload/sink.rs`. These are cosmetic local names, not wire vocabulary. Leave them.

Commit:

    git add agent/src
    git commit -m "refactor(agent): flip expansion literals, error variants, and file_rule_id to v0.5"

### Milestone 5 — Tests, fixtures, and validation

Working directory: `/home/ben/miru/workbench1/repos/agent`.

**Expansion literal in tests.** `agent/tests/services/backend.rs:70` asserts the query pair `("expand", "upload_rules")` → change to `"file_rules"`.

**`Release.file_rules` field accesses in tests.** Update: `agent/tests/services/release/get.rs:87,112,220,239`; `agent/tests/services/release/current.rs:200`; `agent/tests/models/release.rs:80,111`; `agent/tests/sync/helpers.rs:74,99`; `agent/tests/sync/deployments.rs:506`.

**Error-variant renames in tests.** Update: `agent/tests/services/errors.rs:11,54,55,100-102`; `agent/tests/sync/errors.rs:14,105-110`; `agent/tests/services/release/get.rs:10,228-229`; `agent/tests/sync/deployments.rs:502-518`. Where a test asserts on the error's display string, update the expected text to the new `file_rules` wording.

**`upload_rule_id` in tests.** `agent/tests/http/uploads.rs:22` sets `upload_rule_id: "uplr_1"` on a generated struct — rename the field to `file_rule_id`.

**New required `slot_key` on generated config instances.** Only fixtures that build the generated `backend_client::ConfigInstance` with a *full struct literal* (no `..Default::default()`) break:

- `agent/tests/models/config_instance.rs:80` — add `slot_key`.
- `agent/tests/models/config_instance.rs:105` — add `slot_key`.

Use a realistic value such as `"default".to_string()` (the spec's `Deployment` examples use `slot_key: default`). Fixtures that use `..Default::default()` need no change: `agent/tests/services/deployment/get.rs:93`, `agent/tests/models/deployment.rs:653,658`. `agent/src/models/config_instance.rs:38` (`From<backend_client::ConfigInstance>`) simply ignores the new field — no change. Files that construct the *domain* config-instance type (`agent/tests/sync/{syncer,deployments,helpers}.rs`, `agent/tests/deploy/{filesys,apply}.rs`, `agent/src/deploy/filesys.rs`) are unaffected.

**New adapter unit tests.** `agent/src/models/.covgate` is `100`, so every branch of the new `From<backend_client::BaseFileRule>` must be exercised. Add tests (in the existing `agent/tests/models/file_rule.rs` if present, otherwise alongside the other model tests, following the file's existing style) covering, at minimum:

1. **Full rule** — `upload: Some(..)` and `retention: Some(..)` both present. Assert every domain field maps across, including all five `upload` fields and both `source` fields.
2. **Bare rule** — `upload: None` and `retention: None`. Assert both domain fields are `None` and the rest still map.
3. **`require_upload` absent** — `retention: Some(..)` with `require_upload: None`. Assert the domain value is `false`.
4. **`ttl_secs` clamp** — `retention: Some(..)` with a negative `ttl_secs` (e.g. `-1`). Assert the domain value is `0`. Also cover a normal positive value.

Additionally keep or add a test for the timestamp fallback path (an unparseable `created_at` / `updated_at` yields `DateTime::<Utc>::UNIX_EPOCH`) if the previous adapter tests covered it — the branch still exists and still counts toward the 100% gate.

If a test ends up with four or more `assert_eq!` calls on fields of the same variable, add `// lint:allow(field-by-field-assert)` above the test function, per the repo's custom linter rule.

Now run validation (see the next section for expected output and how to read failures):

    ./scripts/update-deps.sh
    ./scripts/lint.sh
    ./scripts/test.sh
    ./scripts/covgate.sh
    ./scripts/preflight.sh

Commit:

    git add -A
    git commit -m "test(agent): update fixtures for v0.5 spec and cover the BaseFileRule adapter"

Then push and open the pull request **as a draft**:

    git push -u origin feat/revendor-spec-v05-beta2
    gh pr create --draft --title "Re-vendor backend spec agent/v0.5.0-beta.2 and flip to file rules" --body "<summary>"

The PR body should state that this is a coordinated breaking change (`v0.5.0-beta.x` agent release) that cannot merge or release until the backend serves v0.5, and should flag that there is no CI drift gate between spec and generated models so the regen diff needs a human eyeball.


## Validation and Acceptance

**The acceptance bar: `./scripts/preflight.sh` must report `CLEAN`, meaning CI is green on the pushed branch head, before the pull request leaves draft or this task is reported complete.** No partial-green result is acceptable, with the one documented exception below.

Individual commands, in order, all from `/home/ben/miru/workbench1/repos/agent`:

`./scripts/update-deps.sh` — refreshes `Cargo.lock`. Must be run *before* `lint.sh`, otherwise lint fails on a stale lockfile. Expected: exits 0; `Cargo.lock` may or may not change. If it changed, include it in the Milestone 5 commit.

`./scripts/lint.sh` — runs the import linter, `cargo fmt`, machete/diet, audit, and clippy. Expected: exits 0 with no findings. Clippy warnings pointing into `libs/backend-api/` or `libs/device-api/` are generated code and are ignorable. Note that `lint.sh` **auto-fixes** formatting, so re-check `git status` after running it and fold any changes into the commit.

`./scripts/test.sh` — equals `RUST_LOG=off cargo test --features test`. Expected: all tests pass, `0 failed`. Plain `cargo test` without `--features test` will not compile; that is a usage error, not a real failure.

`./scripts/covgate.sh` — per-directory coverage gates. Relevant thresholds for the directories this plan touches:

| Directory | `.covgate` |
| --- | --- |
| `agent/src/models/` | 100 (highest risk — the new adapter must be fully covered) |
| `agent/src/services/` | 95.01 |
| `agent/src/services/release/` | 91.66 |
| `agent/src/sync/` | 93.63 |
| `agent/src/data_uploads/upload/` | 96.00 |
| `agent/src/data_uploads/scan/` | 98.83 |
| `agent/src/data_uploads/retention/` | 98.39 |

**Known-ignorable local failure:** `agent/src/workers/.covgate` fails locally on every branch regardless of what is changed — it is a pre-existing local-versus-CI gap and it passes in CI. Do **not** try to fix it and do **not** lower the threshold. Any *other* covgate failure is real.

`./scripts/preflight.sh` — runs `scripts/lint.sh`, `scripts/covgate.sh`, `tools/lint/scripts/lint.sh` (with `LINT_FIX=0`), and `tools/lint/scripts/covgate.sh` in parallel. Expected final report: `CLEAN`.

Behavioral acceptance checks:

    rg -n 'upload_rules|UploadRule|UploadDeletePolicy|upload_rule_id' agent/src agent/tests
    # expect: no matches (the local helper fns named `upload_rule()` in
    # data_uploads/scan/scanner.rs and tests/data_uploads/upload/sink.rs are
    # deliberately out of scope and may still match `upload_rule` — nothing else should)

    rg -n 'x-release-version' api/specs/backend/v05.yaml   # v0.5.0-beta.2
    test ! -f api/specs/backend/v04.yaml && echo "old spec removed"
    git status --short libs/device-api                     # expect: empty


## Idempotence and Recovery

Every step is safe to re-run.

**Re-downloading the spec** overwrites `api/specs/backend/v05.yaml` in place; re-run the two `gh api` commands any number of times. If the file looks wrong (HTML error page, JSON, wrong line count, non-zero `$API_VERSION$` count), delete it and download again.

**Re-running `./api/regen.sh`** is fully idempotent: it cleans `api/codegen/` and wholesale-replaces both model directories from scratch. Running it twice produces the same tree.

**Half-applied regen.** `regen.sh` uses `set -e`. If device generation fails, backend models have already been replaced and the tree is inconsistent. Recover with:

    git checkout -- libs/

then fix the underlying cause and re-run `./api/regen.sh`. The known historical cause is a generator 7.12.0 YAML folded-scalar parse bug on the device spec (`plans/completed/20260515-forward-compatible-enums-via-generator.md:38`).

**Reverting a milestone.** Each milestone is a single commit, so `git revert <sha>` or `git reset --hard HEAD~1` (before pushing) backs out exactly one step. To abandon everything and start over:

    git reset --hard origin/main

This is destructive to uncommitted work — check `git status` first, and prefer `git stash` if unsure.

**Nothing here is destructive outside the agent repo.** The openapi repo is never written to. The only deletion is `git rm api/specs/backend/v04.yaml`, recoverable at any time with `git checkout main -- api/specs/backend/v04.yaml`.

**Intermediate commits do not compile.** Milestones 2 through 4 leave the tree in a non-building state by design (regen renames types before the hand-written call sites are flipped). If you need a compiling tree mid-flight, complete Milestone 4 before running any `cargo` command. Do not bisect within this branch expecting green intermediate commits.
