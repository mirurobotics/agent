# Require the scanner and uploader: collapse dead optionality in app init

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench4/repos/agent | read-write | crate `miru-agent`; app/ + tests only |

Base `main` (`fc7333c`); branch `refactor/require-scan-upload-pipeline`. Standalone —
lands independently of open PRs #197/#198, which touch the same init code and will
rebase over it (or it over them, merge-order Ben's call).

## Purpose / Big Picture

`AppState.scanner` and `.uploader` are `Option<Arc<_>>`, gated by
`AppOptions.enable_scanner` and wrapped in fail-open spawn matches. Investigation
(2026-08-12, with Ben) showed every leg of that optionality is dead:

1. **No kill-switch exists.** The on-device `Settings` file exposes
   `enable_socket_server`/`enable_mqtt_worker`/`enable_poller` — not `enable_scanner`.
   `main.rs` builds `AppOptions` with `..Default::default()`, so the flag is always
   true in production; only tests ever pass false. It was the dark-launch gate for the
   upload pipeline (#151–#167), vestigial since uploads shipped in v0.10.0.
2. **The spawn-error fail-open arms are unreachable.** `Scanner::spawn` and
   `Uploader::spawn` have no executable error path — their `Result`s never carry `Err`.
3. **The fail-open that matters survives untouched**: snapshot-file errors degrade to
   running without persistence via the `Option<SnapshotFile>` *inside* each actor.

Collapse: scanner and uploader become required fields; the enable flag is deleted; the
unreachable matches become `?` (behavior identical — the errors cannot occur). A real
fleet kill-switch, if ever wanted, belongs in `Settings` like `enable_poller`.

## Progress

- [ ] M1: state.rs — required fields, infallible-path `?`, unconditional shutdown; options.rs — drop `enable_scanner`; run.rs — unconditional worker wiring
- [ ] M2: Test surface — `TestEnv::init()` loses the flag param, disabled-mode tests deleted, unwraps dropped
- [ ] M3: Full validation (test.sh, covgate, lint, fmt, no-features check), push, PR

## Surprises & Discoveries

_(filled during execution)_

## Decision Log

- **Spawn `Result`s stay in the actor APIs** (`Scanner::spawn` etc.) and are `?`'d in
  init rather than stripped: stripping would churn every test callsite for zero
  behavior change, and a future genuinely-fallible spawn slots back in cleanly.
- **Tests that used `init(false)` to skip actors now spawn the full pipeline.** The
  actors are cheap (parked tasks over empty state in temp layouts); no test semantics
  depend on their absence except the deleted `*_absent_when_disabled` cases.

## Validation and Acceptance

- `./scripts/test.sh` green; `scripts/covgate.sh` all modules; import/assert linter and
  `cargo fmt --check` clean; `cargo check` with no features clean.
- CI green on the pushed branch head before the PR leaves draft.
