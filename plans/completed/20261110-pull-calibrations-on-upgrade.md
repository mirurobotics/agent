# Pull the latest calibrations on upgrade

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | `calibrations::pull_latest` merges instead of overwriting; upgrade reconcile pulls; an `UpgradeErr` variant; tests and `ARCHITECTURE.md`. |

Part of ENG-1726 (RFC: Latest Calibrations on the Device), after #450 and #451. Based on `main` at `394f78c`.

## Purpose / Big Picture

The latest calibrations store survives upgrades (#451), but agents before it never wrote the store at `calibrations/latest/`. A device that upgrades from one of them, or from a version it was rolled back to, starts with a store that lacks the reports that agent took, though Miru has them. Until each is reported again, `GET /v0.2/calibrations/latest` doesn't list it.

After this change, upgrade reconcile pulls Miru's latest calibrations into the store before the agent serves the Device API. The pull merges: per key, it keeps whichever report is newer, so reports the device took that Miru hasn't received are kept.

Observable: upgrade an agent on an online device whose calibrations are in Miru but not in `calibrations/latest/`. The log shows `calibrations: wrote N latest calibrations from Miru` before `upgrade: resetting storage state for version ... complete`, and the first `GET /v0.2/calibrations/latest` lists them.

## Decisions

- **Pull in upgrade reconcile, not on a later sync (the user's pick in the project thread).** Reconcile already blocks until Miru answers (it issues a token and fetches the device, retrying until it succeeds), so pulling there adds no new wait for the network, and the list is complete as soon as the agent serves. The alternative, a "needs pull" field in `index.json` that the syncer clears after a merge, would let the agent serve sooner but list an incomplete store until that sync, and needs an on-disk field, a syncer change and a reordered `AppState::init`.
- **Pull before `reset`.** `reset` writes the version marker last, so a crash or failure during the pull leaves the marker old and the next start reruns reconcile, pull included.
- **Merge, keeping the newer per key.** A key whose stored entry has the same or a later `reported_at` is skipped before its content is downloaded. Equal times keep the stored entry: it is almost always the same report, and it saves a download. Reports Miru doesn't have are never touched. During reconcile and reprovision the agent isn't running, so the pull is the store's only writer and needs no compare-and-put in the store's worker.
- **One pull for reprovision and upgrade.** Reprovision deletes `calibrations/` first, so merging into its empty store puts every key, as before; its behavior and tests are unchanged.
- **Network failures retry; other failures are logged.** A network error fails the reconcile attempt, which reconcile retries with its backoff. Any other failure (a 4xx/5xx, a full store, a disk error) is logged with `upgrade: the latest calibrations may be incomplete` and the upgrade goes on, so a calibration problem never keeps the agent from starting and deploying.

### Names

| New name | Why |
|----------|-----|
| `UpgradeErr::CalibrationsErr` | Named after the wrapped type, like `ProvisionErr::CalibrationsErr`. |
| `app::upgrade::pull_calibrations` | Matches reprovision's helper of the same name. |
| `calibrations::pull::is_newer` | A question, for a bool. |

### Precedent

Reprovision's `pull_calibrations` (#450) for the upgrade helper. Tests extend `tests/calibrations/pull.rs` and `tests/app/upgrade.rs`'s `reconcile_impl` module.

### Left out

- Keying by component alone (ENG-1823). The merge keys by `ReportMetadata::key()`, so it follows that change.
- Pulling on a schedule. Reports are created only on the device, so the store goes stale only through reprovision or an agent that doesn't write it, and both now pull.

## Progress

- [x] `pull_latest` merges.
- [x] Upgrade reconcile pulls before `reset`.
- [x] Tests written.
- [x] `ARCHITECTURE.md`.
- [ ] CI green.

## Surprises & Discoveries

- Upgrade reconcile already needs Miru, so the planned "mark now, pull on a later sync" design was unnecessary.

## Decision Log

- Pull in reconcile rather than on a later sync: see Decisions.

## Outcomes & Retrospective

To be filled in when CI is green.

## Validation and Acceptance

CI runs the suite; preflight must report `CLEAN` (CI green on the pushed branch head) before the PR leaves draft.

- `tests/calibrations/pull.rs`: replaces an older entry; keeps an entry as new or newer without downloading its content; keeps keys Miru doesn't have; the existing cases still pass.
- `tests/app/upgrade.rs` `reconcile_impl`: pulls Miru's latest into the store; a network failure in the pull fails before the marker and the device update; any other failure still upgrades.
