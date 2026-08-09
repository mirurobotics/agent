# PR 1 — Internal domain restructure: UploadRule → FileRule (mechanical, behavior-preserving)

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench6/repos/agent | read-write | All changes land here (crate `miru-agent` under agent/) |
| agent (generated) | libs/backend-api | read-only | Generated wire models — never hand-edited, not regenerated in this PR |

The plan lives in this repo because every change is internal to the `miru-agent` crate. No backend, spec, or cross-repo changes. Base branch `main`; working branch `refactor/upload-rule-to-file-rule` (already exists).

## Purpose / Big Picture

The agent's internal domain model `UploadRule` mirrors the v0.4 wire shape too literally: upload destination fields live in a required `destination` block, and file-deletion behavior is a `delete_policy` enum. The upcoming file-rules spec v0.5 generalizes rules into a `FileRule` with optional `upload` and `retention` sub-objects. This PR does the internal restructure now, mechanically and behavior-preserving, while the wire protocol stays at v0.4:

- New internal model `FileRule` with `upload: Option<FileRuleUpload>` and `retention: Option<FileRuleRetention>`.
- An adapter maps the current generated `backend_client::BaseUploadRule` (v0.4 wire) into `FileRule`.
- Every internal consumer (disk cache, release model, sync, services, scanner, upload pipeline, workers) is renamed/re-threaded to `FileRule`.
- Wire-facing strings and generated types are untouched: `expand=upload_rules`, the `"release.upload_rules"` log/error string context, `CreateUploadRequest.upload_rule_id`, and everything in libs/backend-api stay exactly as they are.
- Observable behavior is identical, most importantly: a source file is deleted after upload confirmation iff the rule had `delete_policy: after_upload` — now represented as `retention == Some(FileRuleRetention { require_upload: true, ttl_secs: 0 })`.

No retention engine, no scanner re-keying, no migration code. Zero production users, so persisted-state compatibility is handled by start-fresh mechanics (detailed in Context).

## Progress

- [x] M1: FileRule model + adapter from BaseUploadRule
- [x] M2: Thread FileRule through disk, release, sync, services, scanner, upload pipeline, workers
- [x] M3: Update integration tests and fixtures
- [x] M4: Preflight clean, push, CI green, verification of start-fresh paths

## Surprises & Discoveries

- The old hand-written `Deserialize for UploadRule` used `deserialize_error!` only for the timestamp fields; missing id/digest/source were plain serde errors from the inner struct. `FileRule` keeps that exact pattern (id/name/digest/source required by the inner struct, timestamps Option + `deserialize_error!`, upload/retention `#[serde(default)]`).
- `FileRuleRetention` needs `Eq` (not just `PartialEq`): `StableFile` derives `Eq` and now carries `retention: Option<FileRuleRetention>`.
- `cargo clippy --package miru-agent --features test -- -D warnings` fails on the CLEAN branch too: two `clippy::manual_map` warnings in generated `libs/backend-api/src/models/upload_credentials.rs` abort the build under `-D warnings`. Pre-existing, unrelated to this refactor (CI's `--fix --allow-dirty` auto-applies them). miru-agent's own code lints clean with zero warnings.
- The `ModelFixture` harness (agent/tests/models/harnesses.rs) asserts every optional field is *present* in the serialized minimal instance with its `default_value`. `FileRule.upload`/`retention` have no `skip_serializing_if`, so they serialize as `null` — `default_value: json!(null)` is what the harness expects. `name` is a required field in the fixture table (the inner Deserialize struct has no default for it).
- `assert_upload_rule_stored` → `assert_file_rule_stored` and `make_backend_upload_rule` → `make_backend_file_rule` in agent/tests/sync/helpers.rs; the latter still *builds* a wire `BaseUploadRule` (only the helper name changed).
- The M1/M2 commit split is by-path, not by-compilability: neither commit compiles alone (the model swap and the threading are mutually dependent), so M1 = all of agent/src/models/ (including release.rs, which lives there), M2 = everything else — as the plan's "simplest honest split" note anticipated.

## Decision Log

- **Adapter mapping (v0.4 wire → FileRule)**: `BaseUploadRule.upload_collection_id/upload_collection_name` + all `destination` fields fold into `FileRuleUpload { upload_collection_id, upload_collection_name, bucket_id, bucket_name, path }`, always `Some` for v0.4 data (destination is required on the wire). `delete_policy: never` (and `Unknown`) → `retention: None`; `delete_policy: after_upload` → `Some(FileRuleRetention { require_upload: true, ttl_secs: 0 })`.
- **Retention semantics at the delete site**: file deleted after upload confirm iff `retention == Some(FileRuleRetention { require_upload: true, ttl_secs: 0 })` — an exact match, chosen over `matches!(retention, Some(r) if r.require_upload && r.ttl_secs == 0)` so any future nonzero-ttl value changes behavior only when a retention engine is deliberately added (out of scope here). This is bit-for-bit equivalent to the old `delete_policy == AfterUpload` check for all values the v0.4 adapter can produce.
- **`name` field placeholder**: `FileRule.name` is populated from `upload_collection_name` until the wire carries a real rule name (spec v0.5).
- **DeletePolicy removed**: the internal `models::DeletePolicy` enum, its `impl_status_enum!` block, and `UploadRuleDestination` are deleted. `StableFile.delete_policy` and `Job.delete_policy` become `retention: Option<FileRuleRetention>` with `#[serde(default)]` (None == old Never default). The generated `backend_client::UploadDeletePolicy` stays and is consumed only by the adapter.
- **Persisted-state compat = start-fresh, no migration**: stale scanner.json/upload_queue.json reset via `SingleThreadStateFile::new_with_default`; the rules cache compat comes from the upload_rules.json → file_rules.json filename change (fresh empty cache; orphan left in place — zero production users). Mechanics detailed in Context ("Persisted-state compatibility").
- **Scanner with `upload: None`**: scanner stays keyed by `upload_collection_id`. `Scanner::update_rules` skips (with a `warn!` log) any FileRule whose `upload` is None — unreachable with the v0.4 adapter, but the code path must be total. No panic, no error.
- **Wire strings frozen**: `"upload_rules"` expansion string, `"release.upload_rules"` context string, `CreateUploadRequest { upload_rule_id }` field, and test wire fixture id `"uplr_1"` all stay.

## Outcomes & Retrospective

Draft PR: https://github.com/mirurobotics/agent/pull/194 ("refactor(models)!: restructure UploadRule into FileRule domain model").

Landed as four signed commits on `refactor/upload-rule-to-file-rule`:

- `a521d3a` refactor(models): replace UploadRule with FileRule and v0.4 wire adapter
- `2c5dfb8` refactor(agent): thread FileRule through disk, sync, services, scan, upload, workers
- `73ee840` test(agent): update test surface for FileRule rename
- `bbda71c` docs(agent): refresh comments left stale by the FileRule rename

`./scripts/preflight.sh` reports "Preflight clean" (exit 0): 1528 tests pass, every `.covgate` module meets its threshold, import/assert linters clean. All four acceptance greps hold — `"upload_rules"` exactly one hit (services/backend.rs:58), `"release.upload_rules"` exactly one hit (sync/deployments.rs:134), `upload_rules.json` zero hits in agent/src, and `CreateUploadRequest.upload_rule_id` still fed from `job.file_rule_id`. Nothing under api/specs/, libs/backend-api, or libs/device-api changed.

What went to plan: the refactor was as mechanical as predicted, and the "simplest honest split" guidance for M1/M2 was the right call. What cost the most time: the test sweep (M3) touched 20 files, and the `ModelFixture` harness's optional-field contract was not obvious from the plan — worth reading `agent/tests/models/harnesses.rs` first next time a model's required/optional split changes.

## Context and Orientation

All paths are relative to the repo root /home/ben/miru/workbench6/repos/agent unless noted. Read AGENTS.md at the repo root before editing — it defines conventions the linter enforces:

- Imports in three comment-headed groups: `// standard crates`, `// internal crates`, `// external crates` (custom import linter).
- Errors via thiserror + `crate::errors::Error`; aggregate enums via `impl_error!`.
- `#[cfg(feature = "test")]` for test-only code; tests mirror src layout; `#[serial]` for shared OS resources.
- Per-source-directory `.covgate` files (a coverage threshold number, e.g. agent/src/models/.covgate) are enforced by scripts/covgate.sh (scripts/lib/covgate.sh discovers them under agent/src). This rename stays within already-gated directories (models/, disk/), so no `.covgate` changes are needed.
- fmt/clippy are scoped `--package miru-agent`. Never run `cargo fmt --all` (it dirties generated libs).
- CI lint flags 4+ `assert_eq!` on one variable's fields as a field-by-field assert; add `// lint:allow(field-by-field-assert)` where mechanical fixture updates trip it.

### Terms

- **Wire model**: generated Rust in libs/backend-api (crate `backend_client`), produced from the vendored OpenAPI spec. Never hand-edited.
- **Internal model**: hand-written types in agent/src/models/ that the agent's logic uses.
- **Adapter**: the `From<backend_client::X>` impl converting wire → internal.
- **FileCache**: agent/src/cache/file.rs — a persistent keyed JSON cache. Creates an empty file if missing (lines 36-54) but errors (does not reset) on a corrupt existing file.
- **SingleThreadStateFile**: agent/src/filesys/state_file.rs — `new_with_default` (lines 54-63) overwrites with the default when the existing file fails to parse. Used for scanner.json and upload_queue.json, wired in agent/src/app/state.rs (scanner ~lines 147-160, uploader ~lines 192-205; both fail open).

### Current model (agent/src/models/upload_rule.rs)

- `DeletePolicy` enum { Never (default), AfterUpload } via `impl_status_enum!` (backend type `backend_client::UploadDeletePolicy`, mappings never/after_upload).
- `UploadRuleSource { glob: String, stability_window_secs: i64 }` with `From<backend_client::UploadRuleSource>`.
- `UploadRuleDestination { bucket_id, bucket_name, path, delete_policy: DeletePolicy }` with `From<backend_client::UploadRuleDestination>`.
- `UploadRule { id, upload_collection_id, upload_collection_name, digest, source, destination, created_at: DateTime<Utc>, updated_at: DateTime<Utc> }` — derives Clone/Debug/PartialEq/Serialize; custom Default (id/collection = "unknown-{uuid}", timestamps UNIX_EPOCH); `From<backend_client::BaseUploadRule>` parsing timestamps with `error!` + UNIX_EPOCH fallback; hand-written Deserialize via an inner struct with `deserialize_error!` for missing required fields.
- Type aliases `UploadRuleID = String`, `UploadCollectionID = String`.
- Re-exported from agent/src/models/mod.rs (module decl line 8, re-exports lines 26-31).

### Target model (new agent/src/models/file_rule.rs)

    pub type FileRuleID = String;
    pub type UploadCollectionID = String;   // keeps existing alias; scanner stays keyed by it

    pub struct FileRuleUpload {
        pub upload_collection_id: UploadCollectionID,
        pub upload_collection_name: String,
        pub bucket_id: String,
        pub bucket_name: String,
        pub path: String,
    }

    pub struct FileRuleRetention {
        pub require_upload: bool,
        pub ttl_secs: u64,
    }

    pub struct FileRuleSource {           // rename of UploadRuleSource, same fields
        pub glob: String,
        pub stability_window_secs: i64,
    }

    pub struct FileRule {
        pub id: FileRuleID,
        pub name: String,                 // ← wire upload_collection_name (placeholder)
        pub digest: String,
        pub source: FileRuleSource,
        pub upload: Option<FileRuleUpload>,
        pub retention: Option<FileRuleRetention>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

FileRule keeps the existing pattern: derives Clone/Debug/PartialEq/Serialize; custom Default mirroring today's (id = "unknown-{uuid}", timestamps UNIX_EPOCH, upload/retention None); hand-written Deserialize via inner struct with `deserialize_error!` for missing required fields (id, digest, source; `upload` and `retention` are optional with serde-default None). `FileRuleUpload`, `FileRuleRetention`, `FileRuleSource` are plain derive structs (Clone, Debug, PartialEq, Serialize, Deserialize, and Default where useful).

### Wire model consumed by the adapter (libs/backend-api — read-only)

- base_upload_rule.rs: `BaseUploadRule { object, id, upload_collection_id, upload_collection_name, digest, source: Box<UploadRuleSource>, destination: Box<UploadRuleDestination>, created_at: String, updated_at: String }`
- upload_rule_destination.rs: `{ bucket_id, bucket_name, path, delete_policy: UploadDeletePolicy }`
- upload_delete_policy.rs: enum with serde renames "never"/"after_upload" plus `Unknown(#[serde(other)])`
- create_upload_request.rs: `CreateUploadRequest.upload_rule_id` — wire field, unchanged
- release_expansion.rs: "upload_rules" expansion — unchanged

### Delete-after-upload flow (behavior that must be preserved exactly)

Today: `UploadRule.destination.delete_policy` → `build_stable_file` (agent/src/scan/collection.rs lines 300-327) stamps it onto `StableFile.delete_policy` → `enqueue_stable_file` (agent/src/workers/scan_upload_bridge.rs line 80) copies it onto `Job.delete_policy` → `Executor::delete_source_file` (agent/src/upload/executor.rs lines 82-95, called after `confirm_upload` at line 125) deletes the file iff `job.delete_policy == DeletePolicy::AfterUpload`.

After: `FileRule.retention` → `StableFile.retention` → `Job.retention` → delete iff `job.retention == Some(FileRuleRetention { require_upload: true, ttl_secs: 0 })`. Same log lines, same delete call, same error handling.

### Full reference map of consumers to change

Models: agent/src/models/upload_rule.rs (→ file_rule.rs); agent/src/models/mod.rs; agent/src/models/release.rs (`upload_rule_ids: Vec<UploadRuleID>` field ~line 22, Default ~33, `from_backend(release, upload_rule_ids)` ~39-57, `#[serde(default)]` in Deserialize ~72-89, import line 3); agent/src/models/status.rs (doc-comment example, line 16).

Disk: agent/src/disk/upload_rules.rs (→ file_rules.rs; `pub type UploadRules = cache::FileCache<UploadRuleID, UploadRule>`, fns `upload_rules_for_deployment` / `upload_rules_for_deployed`, in-source `#[cfg(test)]` block); agent/src/disk/layout.rs lines 73-75 (`fn upload_rules()` → `resources().file("upload_rules.json")` — rename fn and literal to file_rules / "file_rules.json"); agent/src/disk/mod.rs (module decl line 15, re-export line 25, `Capacities.upload_rules` lines 42/53, `Storage.upload_rules` Arc line 85, spawn in `Storage::init` lines 143-145, shutdown lines 226-230).

Sync: agent/src/sync/deployments.rs (`Storage.upload_rules: &disk::UploadRules` line 32; `store_expanded_release` lines 263-307 reads `backend_release.upload_rules`, errors `SyncErr::UploadRulesNotExpanded` if absent, builds `upload_rule_ids`, writes rule bodies via `write_if_absent`; the wire string "release.upload_rules" at line 134 STAYS); agent/src/sync/syncer.rs line 241; agent/src/sync/errors.rs (`UploadRulesNotExpandedErr` struct + enum variant + From + `impl_error!`, lines 73-79/108/153-157/172).

Services: agent/src/services/release/get.rs (reads `backend_rls.upload_rules` → `ServiceErr::UploadRulesNotExpanded`, builds `upload_rule_ids`); agent/src/services/backend.rs line 58 (expansion string "upload_rules" STAYS); agent/src/services/errors.rs (`UploadRulesNotExpandedErr` + variant).

Workers/app: agent/src/workers/sync_scan_bridge.rs (`Storage.upload_rules` Arc line 18; `resolve_and_push` calls `disk::upload_rules_for_deployed` + `scanner.update_rules`); agent/src/workers/scan_upload_bridge.rs (`enqueue_stable_file` maps `stable.upload_rule_id` line 78 and `stable.delete_policy` line 80 into Job); agent/src/app/run.rs line 347 (`upload_rules: app_state.storage.upload_rules.clone()`).

Scan: agent/src/scan/scanner.rs (`scanners: HashMap<UploadCollectionID, CollectionScanner>`, `deployed: HashSet<UploadCollectionID>`; `update_rules(deployment, rules: Vec<UploadRule>)` keys by `rule.upload_collection_id`, rejects duplicates → `ScanErr::DuplicateCollectionID` lines 159-175; `get_rules()` test helper; large `#[cfg(test)]` module); agent/src/scan/collection.rs (`build_stable_file(..., state.rule().destination.delete_policy)` lines 300-327; `observe_file` stamps `upload_rule_id = state.cfg.rule.id` line 224; `#[cfg(test)]` fixtures `rule()`, `stable_file`, `stamps_after_upload_policy_from_rule`, `stamps_never_policy_from_default_rule`); agent/src/scan/state.rs (`Config.rule: UploadRule` line 24; `Observation.upload_rule_id` line 104; `StableFile.upload_rule_id` line 133 + `StableFile.delete_policy: DeletePolicy` `#[serde(default)]` lines 134-136; `ScannerSnapshot` keyed by UploadCollectionID lines 160-161; `set_config` compares upload_collection_id → InvalidRule; `#[cfg(test)]` fixtures incl. `without_delete_policy_defaults_to_never`); agent/src/scan/errors.rs (`InvalidRule { existing_upload_collection_id, replacement_upload_collection_id }` + `DuplicateCollectionID { collection_id }`; message strings say "upload rule"/"upload collection id" — internal, may be reworded to "file rule").

Upload: agent/src/upload/job.rs (`Job { upload_rule_id: String` line 17, `delete_policy: DeletePolicy` line 19 }); agent/src/upload/executor.rs (`new_upl_request` builds `CreateUploadRequest { upload_rule_id: job.upload_rule_id.clone() }` line 132 — wire field name stays, fed from renamed `job.file_rule_id`; `delete_source_file` lines 82-95); agent/src/upload/uploader.rs (logs `entry.job.upload_rule_id` lines 181/260/283/290/299).

Test surface (agent/tests/, mirrors src): models/upload_rule.rs (→ file_rule.rs; Required/OptionalField tables, `defaults()`, `backend_rule()` fixture with AFTER_UPLOAD, `from_backend`, `from_backend_invalid_dates`, `delete_policy_default`); models/mod.rs (module decl); models/release.rs (6 hits); disk/upload_rules.rs (→ file_rules.rs; rule builder takes DeletePolicy; round-trip + write_if_absent tests); disk/mod.rs; disk/layout.rs (asserts "/var/lib/miru/resources/upload_rules.json" line 106 → file_rules.json); disk/caches.rs; sync/helpers.rs (`make_backend_upload_rule` keeps building the BaseUploadRule wire fixture — rename helper to `make_backend_file_rule` if desired; also `make_backend_release`, `make_deployment_with_release_upload_rules`, `assert_upload_rule_stored`); sync/deployments.rs (hard-coded `dir.file("upload_rules.json")` line 62 → "file_rules.json"); sync/syncer.rs, sync/errors.rs; upload/executor.rs (delete_policy section: `after_upload_deletes_source_after_confirm` line 397, `delete_policy_setup`, Job fixtures); upload/uploader.rs, upload/queue.rs; workers/sync_scan_bridge.rs, workers/scan_upload_bridge.rs; services/release/get.rs, services/release/current.rs, services/errors.rs, services/backend.rs; mocks/scanner.rs (`UpdateRulesCalls` type, imports UploadRule); server/handlers.rs, server/response.rs (`Release { upload_rule_ids }`); http/uploads.rs (wire `upload_rule_id` value "uplr_1" — KEEP unchanged). In-source `#[cfg(test)]` fixtures in scan/scanner.rs, scan/collection.rs, scan/state.rs, disk/upload_rules.rs, scan/errors.rs.

### Persisted-state compatibility (accurate framing — no migration code)

- scanner.json / upload_queue.json: shape changes (StableFile/Job gain `retention`, lose `delete_policy`; field rename upload_rule_id → file_rule_id) are absorbed by `SingleThreadStateFile::new_with_default` — a stale file that fails to parse is overwritten with the default. Note `#[serde(default)]` on `retention` means an old scanner.json missing the field may still parse; either outcome (parse-with-None or reset) preserves the old Never behavior, so both are acceptable.
- resources/upload_rules.json (runtime path /var/lib/miru/resources/upload_rules.json): a FileCache — it does NOT reset on parse failure. Compat is the FILENAME CHANGE to file_rules.json: the new file doesn't exist on first boot after upgrade, so FileCache creates a fresh empty cache and the sync loop repopulates it; the stale upload_rules.json is orphaned (never read again). No cleanup code — zero production users.

## Plan of Work

### M1 — Model and adapter

1. Create agent/src/models/file_rule.rs with the target model above. Keep the file's structure parallel to upload_rule.rs: type aliases, structs, custom Default, `From<backend_client::UploadRuleSource> for FileRuleSource`, `From<backend_client::BaseUploadRule> for FileRule` (timestamp parsing with `error!` + UNIX_EPOCH fallback identical to today), hand-written Deserialize with `deserialize_error!` on missing id/digest/source and serde-default None for upload/retention. Adapter body:
   - `name: wire.upload_collection_name.clone()`
   - `upload: Some(FileRuleUpload { upload_collection_id, upload_collection_name, bucket_id, bucket_name, path })` from top-level + destination fields
   - `retention:` match on `wire.destination.delete_policy` — `AfterUpload` → `Some(FileRuleRetention { require_upload: true, ttl_secs: 0 })`; `Never` and `Unknown(_)` → `None`
2. Delete agent/src/models/upload_rule.rs. In agent/src/models/mod.rs replace the module decl (line 8) and re-exports (lines 26-31): export `FileRule, FileRuleID, FileRuleUpload, FileRuleRetention, FileRuleSource, UploadCollectionID`. `DeletePolicy`, `UploadRule`, `UploadRuleDestination`, `UploadRuleSource`, `UploadRuleID` disappear. Also update the doc comment at agent/src/models/status.rs:16, whose backend-only example cites the deleted `DeletePolicy` — no backend-only user remains, so reword to `backend_type` only (no current users after this refactor). Keep it a one-line comment edit.

The crate will not compile at the end of step 1+2 alone — M1's commit includes the minimal mechanical renames needed to compile (this is fine; the deep threading with behavior-shape changes is M2). Practically: do M1 and M2 edits together but commit them as two commits by staging model files first, or simply fold M1+M2 into sequential edits and commit M1 once `cargo check --package miru-agent` passes with the new model in place. Prefer the simplest honest split: M1 commit = models/ changes plus whatever call-site renames `cargo check` forces; M2 commit = the remaining structural threading (retention plumbing, filename change, scanner skip path).

### M2 — Thread FileRule through the crate

Disk:
1. Rename agent/src/disk/upload_rules.rs → agent/src/disk/file_rules.rs (`git mv`). Inside: `pub type FileRules = cache::FileCache<FileRuleID, FileRule>`; rename fns to `file_rules_for_deployment` / `file_rules_for_deployed`; update the in-source `#[cfg(test)]` block fixtures.
2. agent/src/disk/layout.rs: rename `fn upload_rules()` → `fn file_rules()` returning `resources().file("file_rules.json")`.
3. agent/src/disk/mod.rs: module decl, re-export, `Capacities.file_rules`, `Storage.file_rules` Arc, init spawn, shutdown — all renamed.

Release model:
4. agent/src/models/release.rs: `upload_rule_ids` → `file_rule_ids: Vec<FileRuleID>` (field, Default, `from_backend` param, Deserialize inner struct with `#[serde(default)]`, import).

Sync:
5. agent/src/sync/deployments.rs: `Storage.file_rules: &disk::FileRules`; in `store_expanded_release` keep reading `backend_release.upload_rules` (wire field) but convert each `BaseUploadRule` into `FileRule`, build `file_rule_ids`, write via the renamed cache. The literal context string "release.upload_rules" (line 134) stays byte-identical.
6. agent/src/sync/errors.rs and agent/src/sync/syncer.rs: the error type may keep its `UploadRulesNotExpanded` name (it names the wire expansion, which is still "upload_rules") — decide once and be consistent with services/errors.rs; recommended: KEEP the error names, since they describe the wire expansion. Update only type references that the model rename forces.

Services:
7. agent/src/services/release/get.rs: same treatment as sync — build `file_rule_ids` from `backend_rls.upload_rules`. agent/src/services/backend.rs expansion string "upload_rules" untouched. services/errors.rs untouched except forced type references.

Scan:
8. agent/src/scan/state.rs: `Config.rule: FileRule`; `Observation.file_rule_id`; `StableFile { file_rule_id, retention: Option<FileRuleRetention> #[serde(default)] }` (delete `delete_policy`); ScannerSnapshot keying unchanged (UploadCollectionID); `set_config` still compares upload_collection_id — now via `rule.upload.as_ref().map(|u| &u.upload_collection_id)`; update `#[cfg(test)]` fixtures (the `without_delete_policy_defaults_to_never` test becomes `without_retention_defaults_to_none`).
9. agent/src/scan/scanner.rs: `update_rules(deployment, rules: Vec<FileRule>)` — for each rule, `let Some(upload) = &rule.upload else { warn!("scanner: skipping file rule {} with no upload block", rule.id); continue; }`; key by `upload.upload_collection_id`; duplicate detection unchanged. `get_rules()` helper and `#[cfg(test)]` module updated.
10. agent/src/scan/collection.rs: `build_stable_file` takes/stamps `retention: Option<FileRuleRetention>` cloned from `state.rule().retention`; `observe_file` stamps `file_rule_id = state.cfg.rule.id`; rename `#[cfg(test)]` fixtures (`stamps_after_upload_policy_from_rule` → `stamps_retention_from_rule`, `stamps_never_policy_from_default_rule` → `stamps_no_retention_from_default_rule`).
11. agent/src/scan/errors.rs: keep error shapes; message strings may say "file rule" instead of "upload rule" (internal only).

Upload:
12. agent/src/upload/job.rs: `Job { file_rule_id: String, retention: Option<FileRuleRetention> #[serde(default)] }` (delete `delete_policy` field).
13. agent/src/upload/executor.rs: `new_upl_request` builds `CreateUploadRequest { upload_rule_id: job.file_rule_id.clone(), .. }` (wire field name literal unchanged); `delete_source_file` becomes:

        async fn delete_source_file(&self, job: &Job) {
            if job.retention == Some(FileRuleRetention { require_upload: true, ttl_secs: 0 }) {
                info!("upload: deleting local source file {} per retention policy", job.file);
                if let Err(e) = files::delete(&job.file).await { warn!(...); }
            }
        }

    (Keep the existing log wording if preferred — behavior, not strings, is the contract. Nonzero ttl values are out of scope; exact-match semantics per Decision Log.)
14. agent/src/upload/uploader.rs: log-site renames `entry.job.file_rule_id` (lines 181/260/283/290/299).

Workers/app:
15. agent/src/workers/sync_scan_bridge.rs: `Storage.file_rules` Arc; `resolve_and_push` calls `disk::file_rules_for_deployed` + `scanner.update_rules`.
16. agent/src/workers/scan_upload_bridge.rs: `enqueue_stable_file` maps `stable.file_rule_id` and `stable.retention` into Job.
17. agent/src/app/run.rs line 347: `file_rules: app_state.storage.file_rules.clone()`.

Sanity: `cargo check --package miru-agent` and `cargo check --package miru-agent --features test` (the in-source `#[cfg(test)]`/`#[cfg(feature = "test")]` fixture blocks must also compile).

### M3 — Tests and fixtures

18. `git mv agent/tests/models/upload_rule.rs agent/tests/models/file_rule.rs`; same for agent/tests/disk/upload_rules.rs → file_rules.rs. Update module decls in agent/tests/models/mod.rs and agent/tests/disk/mod.rs.
19. models/file_rule.rs tests: Required/OptionalField tables reflect the new required set (id, digest, source) and optional upload/retention; `defaults()` asserts upload/retention None; `backend_rule()` wire fixture unchanged in shape (still builds BaseUploadRule with AFTER_UPLOAD); `from_backend` asserts the retention mapping both ways (after_upload → Some{true,0}; never → None); `from_backend_invalid_dates` unchanged in spirit; `delete_policy_default` becomes a retention-default test.
20. Sweep remaining test files per the reference map: models/release.rs (file_rule_ids), disk/file_rules.rs, disk/layout.rs (path assertion → "/var/lib/miru/resources/file_rules.json"), disk/caches.rs, sync/helpers.rs (wire fixture builders keep producing BaseUploadRule; internal expectations use FileRule; `assert_upload_rule_stored` → `assert_file_rule_stored`), sync/deployments.rs (`dir.file("file_rules.json")`), sync/syncer.rs, sync/errors.rs, upload/executor.rs (delete section: Job fixtures carry `retention`; `after_upload_deletes_source_after_confirm` now sets `retention: Some(FileRuleRetention { require_upload: true, ttl_secs: 0 })`; add/keep the negative case retention None → file kept), upload/uploader.rs, upload/queue.rs, workers/*, services/*, mocks/scanner.rs, server/handlers.rs, server/response.rs. agent/tests/http/uploads.rs: wire `upload_rule_id: "uplr_1"` stays byte-identical.
21. Watch for the field-by-field-assert lint on updated fixtures; add `// lint:allow(field-by-field-assert)` only where genuinely needed.

### M4 — Preflight and CI

22. Run ./scripts/test.sh, ./scripts/preflight.sh; fix until "Preflight clean". Push and confirm CI green on the branch head. Verify start-fresh paths (see Validation).

## Concrete Steps

All commands run from /home/ben/miru/workbench6/repos/agent unless noted. Commits are SSH-signed automatically (commit.gpgsign is on — never disable it).

Setup:

    cd /home/ben/miru/workbench6/repos/agent
    git checkout refactor/upload-rule-to-file-rule
    git pull --ff-only origin main 2>/dev/null || true   # or: git merge --ff-only main if branch is behind
    git status                                            # expect clean tree

M1 — model + adapter (Plan of Work steps 1-2 plus forced call-site renames to reach a compiling state):

    # ... edits per Plan of Work ...
    cargo check --package miru-agent
    cargo check --package miru-agent --features test
    git add -A
    git commit -m "refactor(models): replace UploadRule with FileRule and v0.4 wire adapter"

Expected: both checks exit 0.

M2 — threading (Plan of Work steps 3-17, whatever wasn't forced in M1: retention plumbing through StableFile/Job, cache filename change, scanner upload-None skip, delete-site rewrite):

    git mv agent/src/disk/upload_rules.rs agent/src/disk/file_rules.rs
    # ... edits per Plan of Work ...
    cargo check --package miru-agent --features test
    cargo clippy --package miru-agent --features test -- -D warnings
    git add -A
    git commit -m "refactor(agent): thread FileRule through disk, sync, services, scan, upload, workers"

M3 — tests + covgates:

    git mv agent/tests/models/upload_rule.rs agent/tests/models/file_rule.rs
    git mv agent/tests/disk/upload_rules.rs agent/tests/disk/file_rules.rs
    # ... test edits per Plan of Work ...
    ./scripts/test.sh
    ./scripts/covgate.sh
    git add -A
    git commit -m "test(agent): update test surface for FileRule rename"

Expected: test.sh exits 0 with all tests passing; covgate.sh reports no missing gates.

M4 — preflight, push, CI:

    ./scripts/preflight.sh          # expect final line: "Preflight clean"
    cargo fmt --package miru-agent  # never --all
    git status                      # expect clean or commit any fmt deltas into M3-style fixup commit
    git push -u origin refactor/upload-rule-to-file-rule
    gh run watch --exit-status $(gh run list --branch refactor/upload-rule-to-file-rule --limit 1 --json databaseId --jq '.[0].databaseId')

Expected: preflight prints "Preflight clean"; CI run for the pushed head concludes success. If a PR is opened, keep it draft until both hold. (gh pr edit --body fails on mirurobotics repos — set the body at `gh pr create` time.)

Verification of start-fresh paths (one-shot test, part of M4 — see Validation for what to assert):

    cargo test --package miru-agent --features test models::file_rule
    cargo test --package miru-agent --features test disk::file_rules

## Validation and Acceptance

Behavioral acceptance criteria:

1. **Delete-after-upload preserved**: a Job whose rule came from wire `delete_policy: after_upload` has `retention == Some(FileRuleRetention { require_upload: true, ttl_secs: 0 })` and its source file is deleted after `confirm_upload`; a Job from `delete_policy: never` (retention None) leaves the file in place. Covered by agent/tests/upload/executor.rs (`after_upload_deletes_source_after_confirm` successor + negative case).
2. **Adapter mapping**: `From<BaseUploadRule> for FileRule` produces `upload: Some(..)` with all five fields, `name == upload_collection_name`, and the retention mapping in criterion 1. Covered by agent/tests/models/file_rule.rs `from_backend` tests.
3. **Wire surface unchanged**: `CreateUploadRequest` still serializes `upload_rule_id`; expansion string "upload_rules" and context string "release.upload_rules" byte-identical; agent/tests/http/uploads.rs passes unmodified in its wire assertions (`upload_rule_id: "uplr_1"`). Verify with:

        grep -rn '"upload_rules"' agent/src/          # expect exactly one hit: services/backend.rs:58
        grep -rn '"release.upload_rules"' agent/src/   # expect exactly one hit: sync/deployments.rs:134
        grep -n 'upload_rule_id' agent/src/upload/executor.rs   # expect the CreateUploadRequest field literal

4. **Start-fresh paths**: (a) scanner/uploader state — a scanner.json or upload_queue.json that fails to parse is overwritten with defaults (existing SingleThreadStateFile tests under agent/tests/ keep passing; app/state.rs wiring untouched); (b) cache — the layout test asserts the new path "/var/lib/miru/resources/file_rules.json", and disk/file_rules.rs round-trip tests show a fresh empty cache is created when the file is absent. No code anywhere reads "upload_rules.json" (grep for the literal in agent/src must return nothing).

        grep -rn 'upload_rules.json' agent/src/       # expect: no matches

5. **Scanner totality**: `update_rules` with a FileRule whose `upload` is None skips it (no panic, no scanner entry) — add a unit test in the scanner test module for this.

Exact commands and expected results:

    ./scripts/test.sh          # all tests pass, exit 0
    ./scripts/preflight.sh     # prints "Preflight clean", exit 0

CI: after pushing, the workflow run on the branch head must conclude green. The PR must not leave draft, and the task must not be reported complete, until BOTH preflight reports CLEAN locally AND CI is green on the pushed head.

## Idempotence and Recovery

- All edits are ordinary file edits + `git mv` on an existing branch; re-running any milestone's edits is safe (Edit/mv are no-ops or fail loudly if already applied).
- Each milestone ends in a commit, so a broken step rolls back with `git checkout -- <paths>` (uncommitted) or `git reset --hard <last-good-commit>` (committed). The branch can be fully reset with `git reset --hard main` and restarted.
- `cargo check` / `./scripts/test.sh` / `./scripts/preflight.sh` are read-only with respect to source and safe to repeat.
- Force-pushing the branch is acceptable before review starts; after a PR has reviewers, prefer follow-up commits.
- Nothing in this plan touches libs/backend-api, the vendored spec, or any file outside the agent repo; a stray change there means a mistake — revert it (`git checkout -- libs/`).
