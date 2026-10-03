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

### Restricting configs to the `miru` group (opt-in)

By default `/srv/miru` is `0755`, so any local account can read configs
deployed under it. To limit them to the `miru` group, as Windows limits
`ProgramData\Miru\configs` to `Miru Agent Users`:

1. Add every account whose applications read configs to `miru` (see above),
   and restart those applications. Accounts left out lose access in step 3.
   Membership also grants the device API socket and the discovery file, so
   every account that can read configs can also call the device API.
2. Override the packaged tmpfiles.d entry. A file in `/etc/tmpfiles.d` replaces
   the packaged one with the same name entirely, so copy it and change only the
   `/srv/miru` line:

   ```sh
   sudo cp /usr/lib/tmpfiles.d/miru-agent.conf /etc/tmpfiles.d/miru-agent.conf
   sudo sed -i 's|^d /srv/miru 0755 |d /srv/miru 0750 |' /etc/tmpfiles.d/miru-agent.conf
   ```

3. Apply it now: `sudo systemd-tmpfiles --create miru-agent.conf`. systemd
   re-applies it at every boot, and the package on every upgrade.

Nothing inside `/srv/miru` changes: members read the existing configs through
the folder. To undo, delete `/etc/tmpfiles.d/miru-agent.conf` and repeat step 3.
After an upgrade, compare the override with
`/usr/lib/tmpfiles.d/miru-agent.conf`: while the override exists, changes to
the packaged entry do not apply.

## Upgrading

Upgrades apply the table above. Compared with earlier releases, the data root
(and so `auth/token.json`) becomes owner-only, and `/var/log/miru` loses access
by others. The socket, the discovery file, and configs in `/srv/miru` are
unchanged.

A log shipper (fluent-bit, promtail, and the like) that tails
`/var/log/miru/*.log` as an account outside `miru` stops receiving logs after
the upgrade. Add its account to `miru` (see above), or read the same logs from
`journalctl -u miru`.

## Uninstall

`apt purge` removes `/var/lib/miru`, `/var/log/miru`, and `/srv/miru`. The
`miru` user and group are kept.
