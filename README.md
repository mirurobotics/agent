# Miru Agent

<p align="center">
  <a href="https://github.com/mirurobotics/agent/actions/workflows/ci.yml"><img src="https://github.com/mirurobotics/agent/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/mirurobotics/agent/releases/latest"><img src="https://img.shields.io/github/v/release/mirurobotics/agent" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue" alt="License"></a>
</p>

The Miru Agent is a Rust binary that runs on devices (robots), manages configuration deployments, reports device state, and communicates with the Miru backend over HTTP and MQTT.

For detailed documentation and usage instructions, please visit the [official documentation](https://docs.mirurobotics.com/docs/agent-sdk).

## Repository structure

```text
agent/
├── agent/                  # Main binary (miru-agent)
│   ├── src/                #   Application source
│   ├── tests/              #   Integration tests (mirrors src/ structure)
│   └── build.rs            #   Build script (embeds git hash, validates version)
├── api/                    # OpenAPI specs and codegen config
├── build/                  # Docker, GoReleaser, release scripts
├── libs/                   # Generated libraries (do not edit)
│   ├── backend-api/        #   Backend API client
│   └── device-api/         #   Device API server
├── scripts/                # Dev tooling (test, lint, coverage, etc.)
└── testdata/               # Test fixtures
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for a deeper explanation of how things fit together.

## Prerequisites

- Rust stable (see `rust-version` in `Cargo.toml` for current MSRV)
- Optional for linting: cargo-machete, cargo-diet, cargo-audit
- Optional for coverage: cargo-llvm-cov, jq

## Building

```bash
cargo build -p miru-agent            # debug
cargo build -p miru-agent --release  # release
```

## Provisioning a device

```bash
export MIRU_API_KEY=<your-api-key>
sudo -E miru-agent provision \
    --device-name=$HOSTNAME \
    --allow-reactivation=false
```

`provision` creates or fetches the device on the backend by name, issues an
activation token, then bootstraps `/srv/miru` and starts the `miru` systemd
unit. It must run as root because it writes to `/srv/miru` (and chowns the
contents to `miru:miru`) and invokes `systemctl`.

### Flags

- `--device-name` (required): name to register the device under. Re-using an
  existing name fetches that device instead of creating a new one.
- `--allow-reactivation` (default `false`): permit a re-issuance of the
  activation token if the device has previously been activated. Without this,
  the command exits with code `4`; pass `--allow-reactivation=true` to opt in
  to retrying activation on an already-activated device.
- `--backend-host` (optional): override the backend URL. Defaults to
  `https://api.mirurobotics.com`.
- `--mqtt-broker-host` (optional): override the MQTT broker host.

### Environment variables

- `MIRU_API_KEY` (required): your API key, sent as `X-API-Key` on the
  device-create and activation-token calls.

### Exit codes

| Code | Meaning |
|------|---------|
| `0`  | Success. |
| `1`  | Generic failure: CLI parse error, missing `--device-name`, non-root caller, or configuration error. |
| `2`  | `MIRU_API_KEY` is unset. |
| `3`  | Backend HTTP failure (network down, 5xx, timeout). Safe to retry. |
| `4`  | Device is already activated and `--allow-reactivation=false`. Re-run with `--allow-reactivation=true` to opt in. |
| `5`  | Install failure: writing `/srv/miru`, RSA keygen, or systemd interaction failed. |

### Systemd interaction

`provision` stops the `miru` systemd unit before install and restarts it
after. On a fresh install where the unit does not yet exist, `systemctl`
returns exit code `5` ("Unit miru.service not loaded"); this is treated as a
no-op so the command works on first-time provisioning as well as
re-provisioning of an already-running device.

## Testing

```bash
./scripts/test.sh
```

This runs `RUST_LOG=off cargo test --features test -- --test-threads=1`. Both flags are required:

- `--features test` enables `#[cfg(feature = "test")]` gated test helpers and mock implementations.
- `--test-threads=1` prevents conflicts on the shared `/tmp/miru.sock` Unix socket.

### Coverage gates

```bash
./scripts/covgate.sh
```

Each module has a `.covgate` file setting its minimum coverage threshold. The script runs tests with `cargo-llvm-cov` and checks each module's coverage against its gate.

## Linting

```bash
./scripts/lint.sh
```

Runs: `cargo update` (updates `Cargo.lock`), the custom import linter, `cargo fmt`, unused dependency checks (machete, diet), security audit, and clippy with `-D warnings`.

## CI/CD

The **CI** workflow (`ci.yml`) runs on every push and pull request. See the workflow file for the current job list.

The **Builder** workflow (`builder.yml`) builds and pushes the builder Docker image to GHCR when `build/Dockerfile.builder` changes.

The **Release** workflow (`release.yml`) triggers on git tags, runs CI, then builds via GoReleaser.

## Releasing

Releases are tag-triggered. GoReleaser cross-compiles for x86_64 and aarch64 Linux and produces `.deb` packages. The build script (`agent/build.rs`) validates that the git tag matches the version in `Cargo.toml`.

## Further reading

- [ARCHITECTURE.md](ARCHITECTURE.md) — System design, codemap, invariants
- [AGENTS.md](AGENTS.md) — Conventions for AI coding agents
- [Official documentation](https://docs.mirurobotics.com/docs/agent-sdk) — User-facing docs
