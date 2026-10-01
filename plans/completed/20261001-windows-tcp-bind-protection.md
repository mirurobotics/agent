# Record and test Windows bind protection for the loopback TCP listener

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` (/home/ben/miru/workbench2/repos/agent) | read-write | Rust workspace for the Miru device agent (crate `miru-agent` in `agent/`). One plan edit, one Windows-only test module, one doc comment. |

Branch `docs/windows-tcp-bind-protection` (already checked out, created from `main` at `09262125`), PR base `main`. The plan lives here because every change is in the agent repo. It resolves the "Windows port hijacking" risk in the roadmap `plans/active/20260910-windows-support.md`.

## Purpose / Big Picture

The roadmap lists an open risk: on Windows, another local process might bind the agent's local device API port while the agent holds it ("port hijacking"), because the agent does not set the Windows socket option `SO_EXCLUSIVEADDRUSE`. Research shows the option adds no protection for this listener, so this change records that decision, marks the risk resolved, and adds Windows-only tests that fail if Windows accepts a second bind (with or without `SO_REUSEADDR`) to the agent's address or routes a loopback connection to a wildcard listener on the agent's port. After the change, the `windows-check` CI job shows the new tests passing; Linux behavior and tests are unchanged. There is no production behavior change.

## Progress

- [x] (2026-10-01) M0 Activate plan (move to `plans/active/`; commit) — `29bf3e50`
- [x] (2026-10-01) M1 Roadmap: replace the hijacking risk, add the 2026-10-01 decision log entry (commit) — `4f5f18a6`, `5a71b5e7`
- [x] (2026-10-01) M2 Windows-only `windows_bind` tests and `tcp::bind` doc comment; fmt, Linux tests, lint (commit) — `2b13d728`, `36158195`
- [x] M3 Preflight CLEAN (CI green on `d35486f7` in one round; `windows_bind::rejects_second_bind_to_same_address` and `windows_bind::wildcard_bind_does_not_take_loopback_connections` `ok` in the `windows-check` log; PR #277 marked ready); plan moved to `plans/completed/` with Outcomes filled

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- 2026-10-01: The roadmap's hijacking risk entry cites Microsoft's page only for bind outcomes, limits the wildcard-bind claim to binds without `SO_EXCLUSIVEADDRUSE`, and credits the loopback-routing claim to the `windows_bind` tests. Rationale: the page states wildcard-vs-specific routing only for an exclusive first socket, and its tables show a wildcard bind that sets `SO_EXCLUSIVEADDRUSE` fails with `WSAEADDRINUSE`.

## Outcomes & Retrospective

Delivered in PR #277 with no production behavior change. The roadmap's Windows port-hijacking risk is resolved: the agent binds `127.0.0.1:<port>` with no socket options, Windows refuses a second bind to that address with or without `SO_REUSEADDR`, and a wildcard bind on the same port does not take loopback connections. Both properties are now locked in by Windows-only tests that ran and passed in CI's `windows-check` on the first round. `SO_EXCLUSIVEADDRUSE` is not set, since it adds no protection for a specific-address listener and brings an unsafe `setsockopt` call plus a rebind delay after restarts. The port-squatting-after-a-crash risk is unchanged; no socket option addresses it.

## Context and Orientation

All paths are relative to the repo root `/home/ben/miru/workbench2/repos/agent`.

The **local device API** can be served over loopback TCP. `agent/src/server/tcp.rs` `pub async fn bind(port: u16) -> Result<(TcpListener, u16), ServerErr>` (doc comment L26-28, body L29-47) binds the specific address `127.0.0.1:<port>` with `tokio::net::TcpListener::bind`. Tokio delegates to `mio` 1.2.2 (`Cargo.lock`), whose `TcpListener::bind` sets `SO_REUSEADDR` only on non-Windows targets and deliberately sets nothing on Windows to prevent "socket hijacking". So on Windows the agent's socket has no socket options and is bound to a specific (not wildcard) address.

**Socket options.** `SO_REUSEADDR` asks the OS to let a socket bind an address already in use. `SO_EXCLUSIVEADDRUSE` (Windows only) asks the OS to refuse every later bind that overlaps this socket's address. A wildcard bind (`0.0.0.0:<port>`) listens on every IPv4 interface, so it overlaps `127.0.0.1:<port>`. Microsoft's "Using SO_REUSEADDR and SO_EXCLUSIVEADDRUSE" (https://learn.microsoft.com/en-us/windows/win32/winsock/using-so-reuseaddr-and-so-exclusiveaddruse) tabulates bind outcomes on Windows 7 and later; the M1 risk entry below states what they mean for this listener, and the M1 decision entry why the option is not set. The page's prose spells out wildcard-vs-specific routing explicitly only for the `SO_EXCLUSIVEADDRUSE` case, so one `windows_bind` test also checks that, without the option, a loopback connection still reaches the agent rather than a wildcard listener.

**Roadmap file.** `plans/active/20260910-windows-support.md` has `## Decision log` at L176 (entries formatted `- YYYY-MM-DD: <decision>. Rationale: <why>.`, wrapped at about 80 columns with 2-space continuation; the last entry is L227-229, dated 2026-09-30) and `## Risks` at L231. The entry to replace is L252-255, "**Windows port hijacking while the agent holds the port**". The entry above it, "**Local API port squatting after a crash**" (L245-251), stays unchanged: after a crash the port is free, so no socket option addresses it.

**Tests.** Integration tests compile into one target named `mod` (`agent/tests/mod.rs`). `agent/tests/server/tcp.rs` already imports `std::net::{Ipv4Addr, SocketAddr}` and `miru_agent::server::{..., tcp, ServerErr, ...}` at the top. `pub mod bind { use super::*; ... }` (L104-141) holds `errors_when_port_in_use` (L133-140), which takes a port with `let (_taken, port) = tcp::bind(0).await.unwrap();`; `pub mod serve` starts at L143. All tests use `#[tokio::test]`. Port `0` (OS-assigned) avoids collisions, so no `#[serial]` is needed. The Windows-only module precedent is `agent/tests/disk/device_api.rs:107`, `#[cfg(windows)] pub mod windows_sharing { use super::*; use std::os::windows::fs::OpenOptionsExt; ... }`; nested modules import external crates right after `use super::*;` (e.g. `agent/tests/cache/errors.rs:28`). `tokio::net::{TcpListener, TcpSocket, TcpStream}` need tokio's `net` feature, which is already enabled through feature unification (`cargo tree -p miru-agent -e features -i tokio` lists `tokio feature "net"`), so no `Cargo.toml` change. `std::time::Duration` is imported at the top, and the file already bounds waits with `tokio::time::timeout` (L94); `TcpListener` and `TcpStream` are not imported at the top, so the module's own imports do not clash with `use super::*;`.

**CI.** `.github/workflows/ci.yml` job `windows-check` (L41-69) runs `cargo test --package miru-agent --locked` with `RUST_LOG=off` on a Windows runner, which includes the `mod` integration target, so the new tests run there. They cannot run on the Linux dev host and are validated by CI only. Clippy (`--all-targets -D warnings`) and `cargo fmt` checks run only on Linux; rustfmt still formats `cfg(windows)` code. A top-level `use tokio::net::TcpSocket;` without `#[cfg(windows)]` would be unused on Linux and fail clippy, so the import goes inside the cfg'd module.

**Conventions.** Run tests with `RUST_LOG=off`. Lint gate: `./scripts/lint.sh` from the repo root. Format with `cargo fmt -p miru-agent`, never `cargo fmt --all` (it reformats generated code). Comments are concise and present tense. Commits use Conventional Commits, are signed (`git commit -S`), end with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, are made from the repo root on `docs/windows-tcp-bind-protection`, one per milestone.

## Plan of Work

**M0.** Move this file from `plans/backlog/` to `plans/active/` and commit.

**M1.** In `plans/active/20260910-windows-support.md`, append this entry at the end of `## Decision log` (after the 2026-09-30 `tcp::serve` failure entry, L227-229):

    - 2026-10-01: The loopback TCP listener does not set `SO_EXCLUSIVEADDRUSE`
      on Windows. Rationale: for a socket bound to a specific address, the
      option changes no bind outcome (see the resolved "Windows port
      hijacking" risk), and it would add an unsafe `windows-sys` `setsockopt`
      call plus a restart caveat (an exclusive socket cannot rebind until
      prior connections become inactive). The `windows_bind` tests in
      `agent/tests/server/tcp.rs` check the behavior in the `windows-check`
      CI job.

Replace the risk entry at L252-255 with:

    - **Windows port hijacking while the agent holds the port** (resolved
      2026-10-01): tokio/mio set no socket options on Windows and the agent
      binds the specific address `127.0.0.1:<port>`. Per the bind tables in
      Microsoft's "Using SO_REUSEADDR and SO_EXCLUSIVEADDRUSE"
      (https://learn.microsoft.com/en-us/windows/win32/winsock/using-so-reuseaddr-and-so-exclusiveaddruse),
      on Windows 7+ the bind outcomes for such a socket are the same with or
      without `SO_EXCLUSIVEADDRUSE`, for same- and different-account callers:
      a second bind to `127.0.0.1:<port>` fails (`WSAEADDRINUSE`, or
      `WSAEACCES` with `SO_REUSEADDR`), and a wildcard `0.0.0.0:<port>` bind
      succeeds unless it sets `SO_EXCLUSIVEADDRUSE`. The page states routing
      beside a wildcard listener only for an exclusive first socket, so the
      `windows_bind` tests check that loopback connections still reach the
      agent. The agent therefore does not set the option; see the 2026-10-01
      decision.

Leave the "Local API port squatting after a crash" entry untouched.

**M2.** In `agent/tests/server/tcp.rs`, insert this module between the closing `}` of `pub mod bind` (L141) and `pub mod serve` (L143), with a blank line on each side:

    #[cfg(windows)]
    pub mod windows_bind {
        use super::*;
        use tokio::net::{TcpListener, TcpSocket, TcpStream};

        #[tokio::test]
        async fn rejects_second_bind_to_same_address() {
            let (_taken, port) = tcp::bind(0).await.unwrap();
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

            let reuse = TcpSocket::new_v4().unwrap();
            reuse.set_reuseaddr(true).unwrap();
            assert!(reuse.bind(addr).is_err(), "SO_REUSEADDR bind to {addr} must fail");

            let plain = TcpSocket::new_v4().unwrap();
            assert!(plain.bind(addr).is_err(), "plain bind to {addr} must fail");
        }

        #[tokio::test]
        async fn wildcard_bind_does_not_take_loopback_connections() {
            let (agent, port) = tcp::bind(0).await.unwrap();
            let _wildcard = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
                .await
                .expect("wildcard bind beside a specific bind must succeed");

            let _client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), agent.accept())
                .await
                .expect("loopback connection must reach the agent, not the wildcard listener")
                .unwrap();
        }
    }

The refusal test asserts only that the bind errors, not a specific `ErrorKind`: Windows refuses with `WSAEADDRINUSE`, or `WSAEACCES` when the second socket sets `SO_REUSEADDR`, and Rust maps these to different kinds. The routing test bounds `accept` with a 5-second timeout, so a misrouted connection fails the test instead of hanging it. CI runs as one account; no different-account table cell is more permissive than its same-account cell and the prose exception relaxes only same-account binds, so a passing same-account test also covers different accounts.

In `agent/src/server/tcp.rs`, extend the `bind` doc comment (L26-28) with one sentence so it reads:

    /// Bind a listener on the IPv4 loopback interface only, so the local device
    /// API is never reachable from the network. Port `0` lets the OS pick a free
    /// port; the returned port is the one actually bound. On Windows the socket
    /// sets no reuse options, so Windows refuses a second bind to the same
    /// address, even one that sets `SO_REUSEADDR`.

No other production code changes. Run `cargo fmt -p miru-agent` (it may rewrap the new asserts), Linux tests, and lint, then commit.

**M3.** Run preflight until it reports `CLEAN`, confirm the new tests passed in the `windows-check` log, mark the PR ready, then move this plan to `plans/completed/` with Progress ticked and Outcomes filled.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent`. Every commit uses `-S` and the trailer, for example:

    git commit -S -m "<subject>" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

M0:

    mkdir -p plans/active
    mv plans/backlog/20261001-windows-tcp-bind-protection.md plans/active/
    git add plans/active/20261001-windows-tcp-bind-protection.md
    git commit -S -m "docs(plans): activate windows tcp bind protection plan" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

M1 (after the edits):

    git diff --stat    # only plans/active/20260910-windows-support.md
    git add plans/active/20260910-windows-support.md
    git commit -S -m "docs(plans): resolve windows tcp port hijacking risk" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

M2 (after the edits):

    cargo fmt -p miru-agent
    RUST_LOG=off cargo test --package miru-agent --test mod server::tcp
    ./scripts/lint.sh
    git add agent/tests/server/tcp.rs agent/src/server/tcp.rs
    git commit -S -m "test(server): lock in windows tcp bind protection" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

Expected: the test run ends with `test result: ok.`, lists the existing `server::tcp::bind::*`, `server::tcp::serve::*`, and `server::tcp::redaction::*` tests, and lists no `windows_bind` tests (compiled out on Linux). Lint exits 0; re-stage anything it reformats.

M3: run preflight (the `preflight` skill). It pushes the branch, opens a draft PR against `main` if none exists, watches CI (`lint`, `test`, `tools`, `windows-check`, `windows-package-scope`), fixes failures from CI logs in follow-up commits (not amends), and must report `CLEAN`. By hand:

    git push -u origin docs/windows-tcp-bind-protection
    gh pr create --draft --base main --title "<title>" --body "<summary; last line: 🤖 Generated with [Claude Code](https://claude.com/claude-code)>"
    gh pr checks --watch
    gh run list --branch docs/windows-tcp-bind-protection --workflow CI --limit 1 --json databaseId    # <run-id>
    gh run view <run-id> --log | grep windows_bind

Expected `grep` output in the `windows-check` job (any order):

    test server::tcp::windows_bind::rejects_second_bind_to_same_address ... ok
    test server::tcp::windows_bind::wildcard_bind_does_not_take_loopback_connections ... ok

Then tick Progress (with the green SHA), fill Outcomes & Retrospective, and:

    git mv plans/active/20261001-windows-tcp-bind-protection.md plans/completed/
    git add plans/completed/20261001-windows-tcp-bind-protection.md
    git commit -S -m "docs(plans): complete windows tcp bind protection plan" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
    git push
    gh pr checks --watch    # must be green on this head
    gh pr ready

## Validation and Acceptance

**Preflight must report `CLEAN` (CI green on the pushed branch head, including `windows-check`) before the PR leaves draft or the task is reported complete.**

On Linux: `RUST_LOG=off cargo test --package miru-agent --test mod server::tcp` passes with the same tests as before (the new module is compiled out), and `./scripts/lint.sh` exits 0.

On Windows (CI only): the `windows-check` log shows the two `... ok` lines expected under M3 in Concrete Steps. With the agent bound to `127.0.0.1:<port>` through `tcp::bind`, a second bind to that address returns `Err` with and without `SO_REUSEADDR`, a wildcard `0.0.0.0:<port>` bind succeeds, and the agent's listener accepts a loopback connection within 5 seconds.

In `plans/active/20260910-windows-support.md`, the hijacking risk reads as resolved, cites Microsoft's bind tables, credits loopback routing to the `windows_bind` tests, and points to the 2026-10-01 decision log entry, and the port-squatting risk is byte-for-byte unchanged (`git diff` shows no change to L245-251).

## Idempotence and Recovery

The doc edits and the test module are safe to repeat; re-check that the decision log entry and test module are not duplicated. `cargo fmt` and `./scripts/lint.sh` rewrite files in place; re-stage after them. If a milestone commit is wrong, fix forward with a new commit.

If a `windows_bind` test fails in CI, that is a real finding: Windows accepted a second bind to the agent's address, refused the wildcard bind, or routed a loopback connection to the wildcard listener. Record it in Surprises & Discoveries with the log excerpt. Do not weaken or remove assertions without a Decision Log entry. Before the PR leaves draft, fix forward in new commits each claim the failure falsifies: drop or correct the M2 `tcp::bind` doc-comment sentence, rewrite the roadmap's 2026-10-01 decision log entry and hijacking risk entry to state the observed outcome, with the risk restored as open. Setting `SO_EXCLUSIVEADDRUSE` in `tcp::bind` is then the fallback, as a separate production change.
