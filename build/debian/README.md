# Debian packaging

This directory holds the maintainer scripts, systemd units, and tmpfiles.d
entry that nfpm (`build/.goreleaser.yaml`) assembles into the `.deb`. The
package runs `/usr/sbin/miru-agent` as the system user `miru` (primary group
`miru`) under `miru.service`, socket-activated by `miru.socket`.

## Paths, owners, and modes

The folder modes are declared once, in the tmpfiles.d entry
(`miru-agent.tmpfiles`), which `postinst` applies on every install and upgrade
and systemd applies at every boot. `StateDirectoryMode` and `LogsDirectoryMode`
in `miru.service` must stay equal to them: without them systemd defaults to
`0755`. Each folder is the access boundary for its contents, as the installer's
folder ACLs are on Windows, so the agent and `postinst` leave the modes of files
inside alone (the private key is still written `0600`).

| Path | Mode | Owner | Group |
| --- | --- | --- | --- |
| `/var/lib/miru` (data root) | `0700` | `miru` | `miru` |
| `/var/log/miru` | `0750` | `miru` | `miru` |
| `/srv/miru` | `0750` | `miru` | `miru` |
| Configs deployed under `/srv/miru` and their folders | umask (normally `0644` / `0755`) | `miru` | `miru` |
| `/run/miru` | `0750` | `miru` | `miru` |
| `/run/miru/miru.sock` | `0660` | `root` | `miru` |
| `/run/miru/device-api.json` (only with `enable_tcp_server`) | `0640` | `miru` | `miru` |

Configs deployed under `/srv/miru` keep their umask-derived modes; `/srv/miru`
itself limits them to the `miru` group. Configs deployed outside `/srv/miru`
keep their umask-derived modes, and their folders decide who can read them.

## Access for local applications

Members of the `miru` group can use the device API socket
`/run/miru/miru.sock`, read the TCP discovery file
`/run/miru/device-api.json`, and read configs deployed under `/srv/miru`. Add
each account whose applications call the API or read configs:

```sh
sudo usermod -a -G miru <account>
```

Membership takes effect at the account's next login, or when a service next
starts; log out and back in, or restart the application's service. For a
systemd service, `SupplementaryGroups=miru` in its unit (or a drop-in) works as
well.

`/var/lib/miru` and `/var/log/miru` are for the agent's internal use. Members of
`miru` get nothing in the data root, and can read the logs in `/var/log/miru`;
other accounts can read neither. The same logs are in `journalctl -u miru`.

## Upgrading

Upgrades apply the table above. Compared with earlier releases, the data root
(and so `auth/token.json`) becomes owner-only, and `/var/log/miru` loses access
by others. The socket and the discovery file are unchanged.

**Breaking:** `/srv/miru` changes from `0755` to `0750`, so configs deployed
under it are readable only by the `miru` group. Applications that read those
configs from an account outside `miru` must join it (see above) before the
upgrade, or they lose access.

A log shipper (fluent-bit, promtail, and the like) that tails
`/var/log/miru/*.log` as an account outside `miru` stops receiving logs after
the upgrade. Add its account to `miru` (see above), or read the same logs from
`journalctl -u miru`.

## Uninstall

`apt purge` removes `/var/lib/miru`, `/var/log/miru`, and `/srv/miru`. The
`miru` user and group are kept.
