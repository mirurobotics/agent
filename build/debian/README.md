# Debian packaging

This directory holds the maintainer scripts, systemd units, and tmpfiles.d
entry that nfpm (`build/.goreleaser.yaml`) assembles into the `.deb`. The
package runs `/usr/sbin/miru-agent` as the system user `miru` (primary group
`miru`) under `miru.service`, socket-activated by `miru.socket`.

## Paths, owners, and modes

`postinst` applies these on every install and upgrade; systemd
(`StateDirectoryMode`, `LogsDirectoryMode`, tmpfiles.d) keeps the folder modes
at boot and service start.

| Path | Mode | Owner | Group |
| --- | --- | --- | --- |
| `/var/lib/miru` (data root) | `0700` | `miru` | `miru` |
| `/var/lib/miru/auth` | `0700` | `miru` | `miru` |
| Files under `/var/lib/miru` (state, token, private key) | `0600` | `miru` | `miru` |
| `/var/lib/miru/auth/public_key.pem` | `0640` | `miru` | `miru` |
| `/var/log/miru` | `0750` | `miru` | `miru` |
| `/srv/miru` | `0755` | `miru` | `miru` |
| Configs deployed under `/srv/miru` and their folders | umask (normally `0644` / `0755`) | `miru` | `miru` |
| `/run/miru` | `0750` | `miru` | `miru` |
| `/run/miru/miru.sock` | `0660` | `root` | `miru` |
| `/run/miru/device-api.json` (only with `enable_tcp_server`) | `0640` | `miru` | `miru` |

`postinst` sets only `/srv/miru` itself; deployed configs keep their
umask-derived modes, so any local account can read them, as before. Configs
deployed outside `/srv/miru` keep their umask-derived modes too.

## Access for local applications

Members of the `miru` group can use the device API socket
`/run/miru/miru.sock` and read the TCP discovery file
`/run/miru/device-api.json`. Add each account whose applications call the API:

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
(including `auth/token.json`), `auth/`, and the agent's files become owner-only,
and `/var/log/miru` and its log files lose access by others. The socket, the
discovery file, and configs in `/srv/miru` are unchanged.

## Uninstall

`apt purge` removes `/var/lib/miru`, `/var/log/miru`, and `/srv/miru`. The
`miru` user and group are kept.
