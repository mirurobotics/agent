# Pull the latest calibrations on upgrade

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | `put_if_newer` and a merging pull; the uploader stores queued reports at startup; the syncer pulls once per run; tests and `ARCHITECTURE.md`. |

Part of ENG-1726 (RFC: Latest Calibrations on the Device), after #450 and #451. Based on `main` at `394f78c`. Split from #455 into three stacked PRs, at the user's request.

## Purpose / Big Picture

The latest calibrations store survives upgrades (#451), but agents before it never wrote the store at `calibrations/latest/`. A device that upgrades from one of them, or from a version it was rolled back to, starts with a store that lacks the reports that agent took, though Miru has them, and lacks the reports it queued but hadn't uploaded. Until each is reported again, `GET /v0.2/calibrations/latest` doesn't list it.

After this change, the uploader stores each queued report the store lacks when it starts, and the first sync of each run that reaches Miru merges Miru's latest into the store. Both keep whichever report is newer per key, so a report the device took is never replaced by an older one.

Observable: upgrade an agent on a device whose calibrations are in Miru but not in `calibrations/latest/`. After the first sync that reaches Miru, the log shows `calibrations: wrote N latest calibrations from Miru` and `GET /v0.2/calibrations/latest` lists them.

## Plan of work

1. **Merge instead of overwrite.** `ReportMetadata::is_as_new_as`; `LatestStore::put_if_newer`, which checks and puts in one worker command; the pull lists the store once, skips keys it has as new or newer before downloading, and puts the rest with `put_if_newer`. Reprovision, the only caller, pulls into an emptied store, so its behavior doesn't change.
2. **Store queued reports at startup.** The uploader's worker, after `prune_orphans`, puts each queued report with `put_if_newer` unless the store has its key as new or newer. Failures are logged.
3. **Pull once per run.** A worker in `workers/` subscribes to the syncer's events and, after each successful sync until a pull succeeds, runs `calibrations::pull_latest` with a token from the token manager. It joins on shutdown before `AppState` shuts the store down, so the syncer and `AppState` are unchanged.

## Decisions

- **Pull on a later sync (the user's pick in the project thread).** The agent starts serving without waiting on the pull. Until the first successful pull of a run, the list can lack reports that only Miru has.
- **Once per run, not a persisted mark** ([AI design review](https://github.com/mirurobotics/agent/pull/455)). An earlier draft marked the store in `index.json` on upgrade and pulled while marked. Pulling once per run needs no on-disk state or upgrade hook, covers rollbacks the same way, and costs one list call when nothing changed.
- **Queued reports are stored by the queue's owner, at startup** ([AI design review](https://github.com/mirurobotics/agent/pull/455)). The uploader owns `queue.json` and the report files, so it reads them; no network is needed.
- **Skip downloads the store doesn't need.** A pull after an upgrade from an agent that already writes the store costs one list call, and a pull that failed part way resumes.
- **The merge compares inside the store's worker.** Intake may write the same key while the syncer pulls. `put_if_newer` checks `reported_at` and puts in one command. Equal times keep the stored entry.
- **Its own worker, not the syncer** ([AI design review](https://github.com/mirurobotics/agent/pull/458)). The deployment syncer stays about deployments; the worker reacts to sync events as `sync_scan_bridge` does, and its downloads never hold up a sync.

### Precedent

`workers/sync_scan_bridge.rs` for a worker reacting to sync events; `prune_orphans` for the uploader's startup step; `pull_latest` (#450) for the pull.

### Left out

- Keying by component alone (ENG-1823). The merge keys by `ReportMetadata::key()`, so it follows that change.

## Progress

- [x] PR 1: merge instead of overwrite.
- [x] PR 2: store queued reports at startup.
- [x] PR 3: pull once per run.

## Surprises & Discoveries

- Upgrade reconcile already needs Miru, so pulling inside it was possible; the user chose the later-sync design.

## Decision Log

- First draft pulled inside reconcile; reworked to mark-then-sync after the user picked that option.
- Replaced the persisted mark with a once-per-run pull and moved the queue merge to the uploader, after the AI design review.
- Split #455 into three PRs at the user's request.
- Moved the per-run pull from the syncer into its own worker, after the AI design review on #458.

## Outcomes & Retrospective

To be filled in when the last PR merges.

## Validation and Acceptance

CI runs the suite on each PR.

- PR 1, `tests/calibrations/latest.rs`: `put_if_newer` puts a new key or a later report and keeps an entry reported at the same time or later. `tests/calibrations/pull.rs`: replaces an older entry; keeps an entry as new or newer without downloading; keeps keys Miru doesn't have.
- PR 2, `tests/calibrations/uploader.rs`: queued reports the store lacks are stored at startup; a stored entry as new or newer is kept.
- PR 3, `tests/workers/calibrations_pull.rs`: pulls after a successful sync, including one before it subscribed, and returns; doesn't pull after a failed sync; retries a failed pull after the next successful sync; skips the pull when it gets no token; stops on shutdown; returns when it can't subscribe or the events end.
