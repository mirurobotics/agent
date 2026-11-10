# Pull the latest calibrations on upgrade

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | A `needs_pull` mark in the latest store's index; `put_if_newer`; upgrade reconcile marks the store; the syncer pulls a marked store; `AppState` spawns the store before the syncer; tests and `ARCHITECTURE.md`. |

Part of ENG-1726 (RFC: Latest Calibrations on the Device), after #450 and #451. Based on `main` at `394f78c`.

## Purpose / Big Picture

The latest calibrations store survives upgrades (#451), but agents before it never wrote the store at `calibrations/latest/`. A device that upgrades from one of them, or from a version it was rolled back to, starts with a store that lacks the reports that agent took, though Miru has them. Until each is reported again, `GET /v0.2/calibrations/latest` doesn't list it.

After this change, upgrade reconcile marks the store (`needs_pull: true` in `index.json`). Each sync that reaches Miru checks the mark; if set, it merges Miru's latest into the store and clears the mark. The merge keeps whichever report is newer per key, so reports the device took that Miru hasn't received are kept. A failed pull leaves the mark, so the next sync retries.

Observable: upgrade an agent on a device whose calibrations are in Miru but not in `calibrations/latest/`. `index.json` has `"needs_pull": true` once the agent starts; after the first sync that reaches Miru, the log shows `calibrations: wrote N latest calibrations from Miru`, `needs_pull` is `false`, and `GET /v0.2/calibrations/latest` lists them.

## Decisions

- **Mark on upgrade, pull on a later sync (the user's pick in the project thread).** The agent starts serving without waiting on the pull. Until the first successful pull after an upgrade, the list can lack reports that only Miru has; the docs say so. The alternative, pulling inside reconcile, would make the list complete at start but makes startup wait on the downloads.
- **The mark lives in `index.json`.** `index.json` is an object so a store-wide field can be added (#451). `needs_pull` defaults to `false` when missing, so existing stores read as unmarked, and an agent rolled back to ignores it. An older agent that rewrites the index drops it, which is fine: the next upgrade marks it again.
- **Mark before `reset`.** `reset` writes the version marker last, so a crash before the mark leaves the old marker and the next start reruns reconcile.
- **A failed mark is logged.** A calibration problem never keeps the agent from starting.
- **The merge compares inside the store's worker.** The agent runs while the syncer pulls, so intake may write the same key. `LatestStore::put_if_newer` checks `reported_at` and puts in one command, so a report the device just took is never overwritten by an older one from Miru. Equal times keep the stored entry.
- **One merge for reprovision and the syncer.** Reprovision deletes `calibrations/` first, so merging into its empty store puts every key, as before; its behavior and tests are unchanged.
- **Best-effort in the syncer, like the system metadata sync.** The pull runs after the deployments sync unless that hit a network error, and a failure is logged and never fails the sync.
- **The syncer gets the store's handle.** `AppState` spawns the calibrations pipeline before the syncer, and shuts the store down after the syncer.

### Names

| New name | Why |
|----------|-----|
| `needs_pull` (field, `LatestStore::needs_pull`, `set_needs_pull`) | A question, for a bool. |
| `calibrations::mark_needs_pull` | Sets the mark on a store in a layout, while no agent runs. |
| `calibrations::pull_if_needed` | Pulls only when marked. |
| `LatestStore::put_if_newer` | `put`, with the condition. |

### Precedent

`sync_system_metadata` in `syncer_impl.rs` for the best-effort sync step; `pull_latest` (#450) for spawning a store handle on a layout. Tests extend `tests/calibrations/latest.rs`, `tests/calibrations/pull.rs`, the syncer's inline `Fixture` and the `reconcile_impl` module in `tests/app/upgrade.rs`.

### Left out

- Keying by component alone (ENG-1823). The merge keys by `ReportMetadata::key()`, so it follows that change.
- Skipping the download for keys the store already has as new. Each pull downloads every content once per upgrade; the worker's check decides.
- Marking on a failed reprovision pull.

## Progress

- [x] Mark, `put_if_newer` and the merge.
- [x] Upgrade reconcile marks; the syncer pulls a marked store.
- [x] Tests written.
- [x] `ARCHITECTURE.md`.
- [ ] CI green.

## Surprises & Discoveries

- Upgrade reconcile already needs Miru, so pulling inside it was possible; the user chose the later-sync design.

## Decision Log

- First draft pulled inside reconcile; reworked to mark-then-sync after the user picked that option.

## Outcomes & Retrospective

To be filled in when CI is green.

## Validation and Acceptance

CI runs the suite; preflight must report `CLEAN` (CI green on the pushed branch head) before the PR leaves draft.

- `tests/calibrations/latest.rs`: `put_if_newer` puts a new key or a later report, and keeps an entry reported at the same time or later; the mark persists, survives puts and clears; the index shape includes `needs_pull`.
- `tests/calibrations/pull.rs`: replaces an older entry; keeps an entry as new or newer; keeps keys Miru doesn't have; `mark_needs_pull` marks; `pull_if_needed` pulls and unmarks a marked store, does nothing unmarked, and keeps the mark on failure.
- `sync/syncer.rs` `latest_calibrations`: a sync pulls and unmarks a marked store; does nothing unmarked; skips the pull when Miru is unreachable; a failed pull keeps the mark and doesn't fail the sync.
- `tests/app/upgrade.rs` `reconcile_impl`: marks the store without pulling.
