# Release carries its upload-rule references via expansion

## Scope

Read-write target: the agent repo at `/home/ben/miru/workbench4/repos/agent`. The
plan file itself lives at
`/home/ben/miru/workbench4/repos/agent/plans/backlog/20260630-release-carry-upload-rule-ids.md`.
All code changes are made inside the agent repo and committed from inside that
repo's own git context (never from the workbench root).

Read-only reference: the openapi repo at `/home/ben/miru/workbench4/repos/openapi`
(on `main`) is the source of truth that the agent's vendored spec is mirrored
FROM. Do not modify it.

This plan delivers a single standalone PR off `main` on the existing branch
`feat/release-carry-upload-rule-ids` (already checked out — do NOT create or
switch branches, do NOT `git stash`). This PR is intended to merge BEFORE the
in-progress uploads PR #93.

Explicitly OUT OF SCOPE (these belong to PR #93, not here — never add them):

- `agent/src/sync/upload_rules.rs` (the `active_upload_rules` traversal) and the
  `pub mod upload_rules;` line in `agent/src/sync/mod.rs`.
- `agent/src/sync/syncer.rs` changes that add an `uploader` field to `SyncerArgs`
  or call `uploader.update_rules(...)`.
- The entire `agent/src/upload/` subsystem and `agent/src/workers/uploads.rs`,
  plus their tests (`agent/tests/upload/`, `agent/tests/workers/uploads.rs`,
  `agent/tests/sync/upload_rules.rs`, `agent/tests/sync/syncer.rs` changes that
  reference the uploader).

The design decision that full upload-rule bodies continue to live in the existing
separate append-only `upload_rules` store is FINAL — do not revisit it. This plan
only adds the lightweight `upload_rule_ids: Vec<UploadRuleID>` references onto the
domain `Release`.

## Purpose / Big Picture

Today the agent's domain `Release` model (`agent/src/models/release.rs`) knows
nothing about which upload rules belong to a release. The full rule bodies are
stored separately in an append-only `upload_rules` store, but a `Release` has no
way to enumerate "my rules". PR #93 (uploads) needs to walk a release's rules to
decide what to upload; without a reference list it would have to re-derive that
mapping. This change gives `Release` a first-class `upload_rule_ids` field, the
same way `Deployment` already carries `config_instance_ids`.

The ids are populated from the backend in two places:

1. The get-release service (fallback fetch path) asks the backend to expand the
   rules via an OpenAPI `expand=upload_rules` query parameter, then projects the
   returned rule bodies down to their ids.
2. Deployment sync, which already receives expanded releases, links the ids onto
   the stored `Release` while continuing to write the full bodies into the
   `upload_rules` store.

For (1) to be possible, the vendored OpenAPI spec must declare the `expand` query
parameter on `getRelease`; that is milestone 1's re-vendor + regenerate step.

When this is done, any consumer of a domain `Release` can read
`release.upload_rule_ids` to enumerate its rules, and PR #93 can build its
uploader on top of that contract without re-deriving the mapping.

## Progress

- [ ] M1 Re-vendor `getRelease` `expand` param + regenerate models
- [ ] M2 Add `upload_rule_ids` to domain `Release`
- [ ] M3 Get-release service requests `expand=upload_rules` and populates ids
- [ ] M4 Deployment sync links ids onto stored `Release`
- [ ] Final validation pass (check / test / lint / preflight / scope grep)

(Living document — update this list as milestones complete.)

## Surprises & Discoveries

(None yet — fill in during implementation.)

## Decision Log

- The store-bodies-separately decision is final; see Scope. (No further decisions
  yet — add them here as they arise during implementation.)

## Outcomes & Retrospective

(To be completed after the work lands.)

## Context and Orientation

Repo layout. The agent workspace root is
`/home/ben/miru/workbench4/repos/agent`. The agent binary crate is nested under
`agent/` (so source is `agent/src/...`, tests are `agent/tests/...`). The
generated API client libraries are at `libs/backend-api/` and `libs/device-api/`.
The spec + codegen tooling lives under `api/`. The workspace `Cargo.toml`
includes the libs as members, so `cargo check --workspace` covers them.

Key files this plan touches:

- `api/specs/backend/v04.yaml` — vendored backend spec (hand-edited in M1).
- `api/regen.sh`, `api/Makefile`, `api/openapitools.json` — regen tooling.
- `libs/backend-api/src/models/release_expansion.rs` — NEW generated file (M1).
- `libs/backend-api/src/models/mod.rs` — gains two lines for the new model (M1).
- `agent/src/models/release.rs` (83 lines) — domain `Release` (M2).
- `agent/src/services/backend.rs` — `fetch_release` (M3).
- `agent/src/services/release/get.rs` (38 lines) — get-release service (M3).
- `agent/src/sync/deployments.rs` — `store_expanded_release` (M4).
- Tests: `agent/tests/models/release.rs`, `agent/tests/services/backend.rs`,
  `agent/tests/services/release/get.rs`, `agent/tests/sync/deployments.rs`,
  `agent/tests/sync/helpers.rs`.

Reference branch. `feat/uploads-file-discovery` already contains working
versions of parts 2, 3 (the `get.rs` extraction) and 4, mixed in with the
out-of-scope uploader code. It does NOT contain the part-1 re-vendor, and does
NOT contain the `fetch_release` `&[]` -> `&["upload_rules"]` change — both are
net-new here. First confirm the branch is present: run `git rev-parse --verify
feat/uploads-file-discovery` (if it errors, run `git fetch origin
feat/uploads-file-discovery:feat/uploads-file-discovery` — the `:local`
refspec is required so the `git diff ..feat/uploads-file-discovery` commands
below resolve a local ref rather than only the remote-tracking
`origin/feat/uploads-file-discovery`). The per-step prose below fully specifies each test
body, so this branch is a cross-check aid, not the sole source — if it is
unavailable, follow the prose. Inspect the proven source diffs (read-only) with,
from `/home/ben/miru/workbench4/repos/agent`:

    git diff origin/main..feat/uploads-file-discovery -- \
      agent/src/models/release.rs \
      agent/src/services/release/get.rs \
      agent/src/sync/deployments.rs

and the proven test bodies with:

    git diff origin/main..feat/uploads-file-discovery -- agent/tests/

When copying test bodies, exclude the out-of-scope files listed in Scope
(`agent/tests/upload/`, `agent/tests/workers/uploads.rs`,
`agent/tests/sync/upload_rules.rs`, and any `agent/tests/sync/syncer.rs` changes
that reference the uploader).

Existing precedent to mirror. The `Deployment` resource already has the exact
shape of every change here:

- A `DeploymentExpansion` schema and a deployment `expand` query parameter
  already exist in the SAME vendored `v04.yaml` — M1 mirrors their style and
  placement for `ReleaseExpansion` / `release_expansions`.
- `Deployment` already carries `config_instance_ids` — M2/M4 mirror that for
  `upload_rule_ids`.
- `fetch_deployment` in `agent/src/services/backend.rs` already passes
  `&["config_instances"]` — M3 mirrors that for `fetch_release`.

A note to dispel confusion: there is NO `poll_interval` work in this plan. No
such field exists in either spec; the unrelated `poll_interval_secs` removal was
already handled by a prior completed plan. Do not add a milestone for it.

Validation tooling (all run from `/home/ben/miru/workbench4/repos/agent`):

- `./scripts/test.sh` — sets `CARGO_FEATURES=--features test` (mandatory; mocks
  are gated behind the `test` feature), `RUST_LOG=off`, package `miru-agent`.
- `cargo check --workspace` — compiles the agent crate plus both libs.
- `./scripts/update-deps.sh` — `cargo update --verbose` at repo root; run BEFORE
  lint to refresh `Cargo.lock`.
- `./scripts/lint.sh` — custom import-order linter + field-by-field-assert
  detector + `cargo fmt` + `cargo machete` + rustsec audit + `cargo clippy
  --all-features -D warnings`. Per the environment note, run this separately
  after `./scripts/update-deps.sh`.
- `./scripts/preflight.sh` — runs `scripts/lint.sh`, `scripts/covgate.sh`,
  `tools/lint/scripts/lint.sh`, `tools/lint/scripts/covgate.sh` in parallel;
  clean = all exit 0.

Conventions (from `agent/AGENTS.md`):

- Strict import grouping: `// standard crates` / `// internal crates` /
  `// external crates`.
- Generated `libs/` code must NOT be hand-edited; regenerate via `api/regen.sh`.
  Clippy warnings inside generated code are expected and tolerated.
- FIELD-BY-FIELD-ASSERT lint: 4 or more `assert_eq!` calls on fields of the same
  variable in one test triggers a finding. Avoid by building an `expected` struct
  and asserting `assert_eq!(actual, expected)` (the reference `from_backend` test
  does this), or annotate with `// lint:allow(field-by-field-assert)`.

Commit style: Conventional Commits, one commit per milestone, each ending with
the trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`. Suggested
subjects:

- M1 `chore(api): vendor getRelease expand param and regenerate models`
- M2 `feat(models): add upload_rule_ids to Release`
- M3 `feat(services): request expand=upload_rules and populate ids`
- M4 `feat(sync): link upload_rule_ids onto stored release`

## Plan of Work

Four milestones, one commit each, in strict order (each depends on the previous
compiling):

- M1 Re-vendor the spec + regenerate models. Hand-edit `api/specs/backend/v04.yaml`
  to add three blocks (the `getRelease` parameter ref, the `ReleaseExpansion`
  schema, the `release_expansions` parameter), run `api/regen.sh`, validate the
  `libs/` delta is exactly one new file plus one modified `mod.rs`, commit.
- M2 Data model. Add `upload_rule_ids` to the `Release` struct, replace the
  `From` impl with a `from_backend` constructor, thread the field through the
  custom `Deserialize`, fix the two model tests. `cargo check`, commit.
- M3 Get-release service. Change `fetch_release` to request `&["upload_rules"]`,
  rework `get.rs` to project rule ids and call `from_backend`, update the two
  affected service tests and add the new "links ids" service test.
  `./scripts/test.sh`, commit.
- M4 Deployment sync. Rework `store_expanded_release` to compute ids from the
  already-extracted rules and call `from_backend`, add the sync test plus the
  `read_release` test helper. `./scripts/test.sh`, commit.

Then a final whole-repo validation pass plus the out-of-scope grep guard.

Why this order: M2 introduces `from_backend`, which M3 and M4 both call, so it
must land first. M3 depends on M1's regenerated `ReleaseExpansion`/expand wiring
being present in the spec (although the http layer passes plain string slices, M1
keeps the spec and generated models consistent). M4 reuses the `from_backend`
constructor from M2.

Note: M2 leaves temporary `from_backend(..., vec![])` stubs at the two call sites
so its commit compiles; M3 and M4 replace those exact stubs with real id
projection. This is intentional — each milestone commit builds on its own.

## Concrete Steps

All commands below run from `/home/ben/miru/workbench4/repos/agent` unless stated
otherwise.

### M1 — Re-vendor + regenerate

Step 1.1 — Confirm the starting state. The vendored `Release` schema in
`v04.yaml` ALREADY has the `upload_rules` expansion field (around lines 982-986),
and the generated `libs/backend-api/src/models/release.rs` already has
`pub upload_rules: Option<Vec<models::BaseUploadRule>>`. The ONLY missing pieces
are the three `expand`-related blocks on the request side. Verify:

    grep -n "release_expansions\|ReleaseExpansion" api/specs/backend/v04.yaml

Expected output: nothing (these do not exist yet). Also confirm the precedents
exist:

    grep -n "DeploymentExpansion\|getRelease" api/specs/backend/v04.yaml

Expected: matches for `DeploymentExpansion` (a schema + a deployment expand
param) and the `getRelease` operation.

Step 1.2 — Hand-edit `api/specs/backend/v04.yaml`, mirroring the openapi bundle
`apis/apps/backend-server/agent/openapi.gen.yaml` exactly. Make three additions:

(a) In the `getRelease` operation's `parameters:` list (around lines 133-151),
add the parameter ref alongside the existing parameters:

        - $ref: '#/components/parameters/release_expansions'

(b) In `components/schemas`, placed near the existing `DeploymentExpansion`
schema and matching its style, add:

        ReleaseExpansion:
          type: string
          enum:
            - upload_rules
          x-enum-varnames:
            - RELEASE_EXPAND_UPLOAD_RULES

(c) In `components/parameters`, placed near the existing deployment expand
parameter and matching its style, add:

        release_expansions:
          name: expand
          in: query
          required: false
          description: Fields to expand on the release resource.
          schema:
            type: array
            items:
              $ref: '#/components/schemas/ReleaseExpansion'
            example:
              - upload_rules

Step 1.3 — Confirm the spec delta is exactly those three blocks (nothing else):

    git diff api/specs/backend/v04.yaml

Expected: three added hunks corresponding to (a), (b), (c); no deletions, no
unrelated reformatting.

Step 1.4 — Regenerate. This needs Node/npx and a Java JRE (the generator is
pinned to 7.12.0 in `api/openapitools.json`). `api/regen.sh` resolves the git
root, `cd`s into `api/`, runs `make gen` (clean + gen-backend + gen-device), then
for backend does `rm -rf libs/backend-api/src/models/*` followed by `cp -r
api/codegen/backend/src/models/* libs/backend-api/src/models` (and the device
analog). Run:

    ./api/regen.sh

Expected: the generator runs for backend and device, then the copy step
completes with no errors.

Step 1.5 — Validate the `libs/` delta is exactly what was predicted:

    git status --short libs/

Expected output (and nothing else):

    A  libs/backend-api/src/models/release_expansion.rs
     M libs/backend-api/src/models/mod.rs

`libs/device-api` MUST be untouched. The generated
`libs/backend-api/src/models/release.rs` MUST be unchanged (the `upload_rules`
field was already present). Inspect the new file and the mod change:

    git diff libs/backend-api/src/models/mod.rs
    cat libs/backend-api/src/models/release_expansion.rs

Expected `mod.rs` diff: one added `pub mod release_expansion;` and one added
`pub use self::release_expansion::ReleaseExpansion;` (placement follows the
generator's alphabetical ordering). Expected new file: a `ReleaseExpansion` enum
mirroring the generated `DeploymentExpansion` — a `RELEASE_EXPAND_UPLOAD_RULES`
variant with `#[serde(rename = "upload_rules")]`, an `Unknown` catch-all, plus
`Display` and `Default` impls.

Fallback if regen cannot run (no Node/Java). PREFER `api/regen.sh`. Only if it
genuinely cannot run, hand-write `libs/backend-api/src/models/release_expansion.rs`
by copying the generated `deployment_expansion.rs` and substituting the type name
(`DeploymentExpansion` -> `ReleaseExpansion`) and the single variant
(`RELEASE_EXPAND_UPLOAD_RULES` with `#[serde(rename = "upload_rules")]`), then add
the two `mod.rs` lines. It must be byte-equivalent to the generated output. The
enum is unused (Part 3 passes plain `&["upload_rules"]` slices); that is expected,
exactly as for `DeploymentExpansion`.

Step 1.6 — Commit M1 (from inside the agent repo):

    git add api/specs/backend/v04.yaml \
            libs/backend-api/src/models/release_expansion.rs \
            libs/backend-api/src/models/mod.rs
    git commit -m "chore(api): vendor getRelease expand param and regenerate models

    Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M2 — Data model

Reference the proven diff first:

    git diff origin/main..feat/uploads-file-discovery -- agent/src/models/release.rs

Step 2.1 — Edit `agent/src/models/release.rs`:

- Add the import (internal crates group): `use crate::models::UploadRuleID;`
  (`UploadRuleID` is `pub type UploadRuleID = String;` in
  `agent/src/models/upload_rule.rs:75`, re-exported from
  `agent/src/models/mod.rs:29`).
- Add the field to the struct (currently lines 14-21, derives
  `Clone, Debug, PartialEq, Serialize`):

        pub upload_rule_ids: Vec<UploadRuleID>,

- REPLACE the `impl From<backend_client::Release> for Release` block (lines
  35-51) with an inherent constructor that keeps the exact same date parsing
  (`.parse::<DateTime<Utc>>().unwrap_or(UNIX_EPOCH)` for `created_at` /
  `updated_at`) and sets the new field from the argument:

        pub fn from_backend(
            release: backend_client::Release,
            upload_rule_ids: Vec<UploadRuleID>,
        ) -> Release {
            // ... same id / version / git_commit_id / date parsing as before ...
            Release {
                // ... existing fields ...
                upload_rule_ids,
            }
        }

  Put `from_backend` in an `impl Release { ... }` block.

- In the custom `impl<'de> Deserialize<'de> for Release` (lines 53-82), add to
  the inner `DeserializeRelease` struct:

        #[serde(default)]
        upload_rule_ids: Vec<UploadRuleID>,

  and map it through into the constructed `Release` (alongside the existing
  fields, preserving the `deserialize_error!` fallback).

Step 2.2 — Update the model tests in `agent/tests/models/release.rs`. The
`from_backend()` test (~line 83) and `from_backend_invalid_dates()` test (~line
99) currently construct via `.into()`; rewrite them to call
`Release::from_backend(backend_release, vec![...])`. Copy the proven bodies from:

    git diff origin/main..feat/uploads-file-discovery -- agent/tests/models/release.rs

The proven `from_backend` test builds an `expected` `Release` struct and asserts
`assert_eq!(actual, expected)` to satisfy the field-by-field-assert lint.

Step 2.3 — Keep the tree compiling with temporary call-site stubs. Replacing the
`From` impl breaks the two existing call sites, which still use `.into()` /
`from(...)`. M3 and M4 are the proper homes for the real id-projection logic, so
do NOT write it here. Instead make each call site compile with an empty-ids stub:

- `agent/src/services/release/get.rs:20` — `models::Release::from_backend(backend_rls, vec![])`
- `agent/src/sync/deployments.rs:258` — `models::Release::from_backend(backend_release, vec![])`

These `vec![]` stubs are deliberately temporary; M3 and M4 replace them with real
id extraction. Then compile-check (from `/home/ben/miru/workbench4/repos/agent`):

    cargo check --workspace

Expected: clean.

Step 2.4 — Commit M2:

    git add agent/src/models/release.rs agent/tests/models/release.rs \
            agent/src/services/release/get.rs agent/src/sync/deployments.rs
    git commit -m "feat(models): add upload_rule_ids to Release

    Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M3 — Get-release service

Step 3.1 — `agent/src/services/backend.rs`, `fetch_release` (lines 55-60).
Change the empty expansions slice to request the rules. Current:

        http::with_retry(|| async {
            http::releases::get(self.client, id, &[], &token.token).await
        })

Change `&[]` to `&["upload_rules"]`. Compare `fetch_deployment` (~line 49) which
passes `&["config_instances"]`. The http layer
`http::releases::get(client, id, expansions: &[&str], token)` builds
`QueryParams::new().expand(expansions)`, and `expand` pushes one
`("expand", value)` pair per item — so `&["upload_rules"]` produces exactly one
query pair `("expand", "upload_rules")`.

Step 3.2 — `agent/src/services/release/get.rs` (lines 19-22). Replace the
temporary `models::Release::from_backend(backend_rls, vec![])` stub left by M2
with the real id projection feeding `from_backend`.
Use `unwrap_or_default()` (tolerant) because this is the fallback fetch path:

        let upload_rule_ids: Vec<models::UploadRuleID> = backend_rls
            .upload_rules
            .as_ref()
            .map(|rules| rules.iter().map(|r| r.id.clone()).collect())
            .unwrap_or_default();
        let storage_rls = models::Release::from_backend(backend_rls, upload_rule_ids);
        cache_release(releases, storage_rls.clone()).await;
        Ok(storage_rls)

(The generated backend `Release` has `pub upload_rules:
Option<Vec<models::BaseUploadRule>>`; `BaseUploadRule` has `pub id: String`,
derives `Default`, so `BaseUploadRule { id, ..Default::default() }` compiles in
tests.)

Step 3.3 — Tests.

`agent/tests/services/backend.rs`: the existing
`fetch_release_constructs_url_no_expand` test (~lines 57-75) asserts a
`CapturedRequest` with `query: vec![]`. Rename it (e.g.
`fetch_release_constructs_url_and_expand_param`) and change the assertion to:

        query: vec![("expand".to_string(), "upload_rules".to_string())],

The deployment analog (~lines 16-34) shows the exact pattern with
`("expand".to_string(), "config_instances".to_string())`. The test uses
`MockClient` + `StubTokenManager`, `Call::GetRelease`, path `/releases/rls_1`.

`agent/tests/services/release/get.rs`: add a service test (reference name
`cache_miss_backend_release_with_upload_rules_links_ids`). Using the existing
`StubBackend::new().with_release(Ok(...))` and the `setup(name)` helper that
returns `(filesys::Dir, Releases)`, return a backend release whose
`upload_rules` is `Some(vec![BaseUploadRule { id: "upl_rule_1".into(),
..Default::default() }, BaseUploadRule { id: "upl_rule_2".into(),
..Default::default() }])`, run get-release, and assert the returned (and cached)
domain `Release.upload_rule_ids == vec!["upl_rule_1", "upl_rule_2"]`.

Copy proven bodies for the `get.rs` test from:

    git diff origin/main..feat/uploads-file-discovery -- agent/tests/services/release/get.rs

(The `backend.rs` expand-param test is net-new here — write it by mirroring the
deployment analog already in `agent/tests/services/backend.rs`.)

Step 3.4 — Run the targeted tests, then commit:

    ./scripts/test.sh

Expected: the renamed backend test and the new get-release test pass; the whole
suite is green.

    git add agent/src/services/backend.rs agent/src/services/release/get.rs \
            agent/tests/services/backend.rs agent/tests/services/release/get.rs
    git commit -m "feat(services): request expand=upload_rules and populate ids

    Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### M4 — Deployment sync

Reference the proven diff:

    git diff origin/main..feat/uploads-file-discovery -- agent/src/sync/deployments.rs

Step 4.1 — `agent/src/sync/deployments.rs`, `store_expanded_release` (lines
250-291). After M2's stub, the body builds the release via the temporary
`models::Release::from_backend(backend_release, vec![])` call, writes it, then
extracts `backend_release.upload_rules` with an `ok_or_else(UploadRulesNotExpanded
{ deployment_id })`, then loops writing full rule bodies. Rework so the rules are
extracted FIRST (reusing the same strict `ok_or_else(UploadRulesNotExpanded)` —
this is the strict path that MUST error if expansion is missing), then compute the
ids, then build the release via `from_backend`, then write the release, then run
the SAME for-loop writing full bodies into `storage.upload_rules`. Rename the loop
var `rules` -> `backend_rules`:

        // Link the rule ids onto the stored Release (mirrors config_instance_ids
        // on Deployment); full rule bodies are stored separately below.
        let backend_rules = backend_release.upload_rules.clone().ok_or_else(|| {
            SyncErr::UploadRulesNotExpanded(UploadRulesNotExpandedErr {
                deployment_id: backend_dpl.id.clone(),
            })
        })?;
        let upload_rule_ids: Vec<models::UploadRuleID> =
            backend_rules.iter().map(|r| r.id.clone()).collect();
        let release = models::Release::from_backend(backend_release.clone(), upload_rule_ids);
        let release_id = release.id.clone();
        storage.releases.write_if_absent(release_id, release, |_, _| false).await?;
        for backend_rule in backend_rules {
            let rule: models::UploadRule = backend_rule.into();
            let id = rule.id.clone();
            storage.upload_rules.write_if_absent(id, rule, |_, _| false).await?;
        }
        // ... git_commit write unchanged ...

`UploadRulesNotExpandedErr` lives in `agent/src/sync/errors.rs:75` (variant
~line 108) and is imported via `use crate::sync::errors::*;`.

Step 4.2 — Tests.

`agent/tests/sync/helpers.rs`: add a `read_release(release_stor, id)` helper
mirroring the existing `read_deployment` helper, using
`read_optional(...).await.unwrap().expect("release should be stored")`.

`agent/tests/sync/deployments.rs`: in `mod pull_success`, add
`populates_release_upload_rule_ids`. Build a deployment using the existing helper
`make_deployment_with_release_upload_rules("dpl_1", cfg_inst_args,
&["upl_rule_1", "upl_rule_2"])` (verify its exact name/signature when
implementing — it is already referenced by sibling tests in this file, alongside
`assert_upload_rule_stored`, `read_deployment`, `assert_deployment_stored`), run
`f.sync()`, then `read_release(&f.release_stor, "dpl_1_rel")` and assert
`release.upload_rule_ids == vec!["upl_rule_1", "upl_rule_2"]`.

Copy proven bodies (excluding out-of-scope test files) from:

    git diff origin/main..feat/uploads-file-discovery -- agent/tests/sync/deployments.rs agent/tests/sync/helpers.rs

Step 4.3 — Run tests and commit:

    ./scripts/test.sh

Expected: `populates_release_upload_rule_ids` passes; suite green.

    git add agent/src/sync/deployments.rs agent/tests/sync/deployments.rs \
            agent/tests/sync/helpers.rs
    git commit -m "feat(sync): link upload_rule_ids onto stored release

    Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"

### Final validation pass

From `/home/ben/miru/workbench4/repos/agent`:

    cargo check --workspace
    ./scripts/test.sh
    ./scripts/update-deps.sh
    ./scripts/lint.sh
    ./scripts/preflight.sh

Expected: `cargo check` clean; `test.sh` green; `update-deps.sh` refreshes
`Cargo.lock`; `lint.sh` reports no findings; `preflight.sh` reports all gates
exit 0.

If `update-deps.sh` changed `Cargo.lock`, amend it into the M4 commit before
pushing: `git add Cargo.lock && git commit --amend --no-edit`.

Out-of-scope grep guard — confirm NONE of PR #93's symbols leaked in:

    git diff origin/main..HEAD | grep -nE "active_upload_rules|pub mod upload_rules|SyncerArgs" ; \
    test -f agent/src/sync/upload_rules.rs && echo "LEAK: sync/upload_rules.rs" ; \
    test -d agent/src/upload && echo "LEAK: src/upload/" ; \
    test -f agent/src/workers/uploads.rs && echo "LEAK: workers/uploads.rs" ; \
    echo "grep guard done"

Expected: no matches for `active_upload_rules`, `pub mod upload_rules`, or an
`uploader` added to `SyncerArgs`; no LEAK lines; just `grep guard done`. (A bare
`SyncerArgs` mention may match unrelated context — manually confirm no
`uploader` field was added.)

## Validation and Acceptance

Acceptance is phrased as observable test behavior — each new/changed test fails
on `main` and passes after its milestone.

- M1: `git status --short libs/` shows EXACTLY
  `A libs/backend-api/src/models/release_expansion.rs` and
  `M libs/backend-api/src/models/mod.rs`, with `libs/device-api` and the
  generated `release.rs` untouched. `git diff api/specs/backend/v04.yaml` shows
  exactly the three added blocks. `cargo check --workspace` compiles.

- M2: `agent/tests/models/release.rs` — `from_backend()` and
  `from_backend_invalid_dates()` fail to compile on `main` once rewritten to
  call `Release::from_backend(...)` (because the constructor does not yet exist),
  and pass after M2. `cargo check --workspace` is clean.

- M3: `agent/tests/services/backend.rs` —
  `fetch_release_constructs_url_and_expand_param` asserts
  `query == vec![("expand".into(), "upload_rules".into())]`; this fails before
  the `&[]` -> `&["upload_rules"]` change and passes after.
  `agent/tests/services/release/get.rs` —
  `cache_miss_backend_release_with_upload_rules_links_ids` asserts the cached
  domain `Release.upload_rule_ids == vec!["upl_rule_1", "upl_rule_2"]`; fails
  before (ids never populated) and passes after. `./scripts/test.sh` green.

- M4: `agent/tests/sync/deployments.rs` — `populates_release_upload_rule_ids`
  reads the stored release after `f.sync()` and asserts
  `upload_rule_ids == vec!["upl_rule_1", "upl_rule_2"]`; fails before (sync never
  links ids) and passes after. The existing rule-body-store assertions
  (`assert_upload_rule_stored`) continue to pass, confirming full bodies still
  land in the separate `upload_rules` store. `./scripts/test.sh` green.

- Whole-repo: `cargo check --workspace`, `./scripts/test.sh`, `./scripts/lint.sh`
  (after `./scripts/update-deps.sh`), and `./scripts/preflight.sh` all clean.

- Scope guard: the out-of-scope grep finds none of `active_upload_rules`,
  `pub mod upload_rules`, an `uploader` on `SyncerArgs`, and none of the
  out-of-scope files/dirs exist.

## Idempotence and Recovery

- The spec edit is idempotent: re-applying the three blocks when they already
  exist is a no-op (or a trivially detectable duplicate). Always confirm with
  `git diff api/specs/backend/v04.yaml` that only the intended hunks are present.

- `api/regen.sh` is safe to re-run: it does `rm -rf libs/backend-api/src/models/*`
  then re-copies from freshly generated codegen, so the result is deterministic
  for a given spec. Re-running after a clean spec edit reproduces the same
  `libs/` delta.

- Recovering from a noisy regen. If `git status --short libs/` shows more than the
  one new file + `mod.rs` change (e.g. the toolchain reformatted unrelated
  generated files, or device models changed), reset the generated tree and
  re-apply only the surgical spec edit:

      git checkout -- libs/
      # confirm the spec still has exactly the three intended blocks:
      git diff api/specs/backend/v04.yaml
      ./api/regen.sh
      git status --short libs/

  Do NOT run the openapi repo's python release-build toolchain to produce the
  spec — it generates noisy diffs and version regressions. Hand-edit `v04.yaml`
  to mirror the openapi bundle exactly, as in M1.

- If regen produces a `release.rs` change in `libs/backend-api`, that is a
  signal the spec edit accidentally touched the `Release` schema (it must not —
  the `upload_rules` field is already present). Reset `libs/`, re-inspect the
  spec diff, and re-run.

- Per-milestone commits make recovery cheap: if a later milestone goes wrong, the
  earlier commits stand on their own (each compiles). Re-running `./scripts/test.sh`
  and `cargo check --workspace` is always safe and side-effect-free.

- `./scripts/update-deps.sh` runs in the final validation pass. If it dirties
  `Cargo.lock`, amend that change into the M4 commit (the last milestone) before
  pushing — that is its single committed home. Re-running update-deps.sh is
  idempotent once dependencies are settled.
