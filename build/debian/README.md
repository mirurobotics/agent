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
| `/srv/miru/configs` and its subfolders | `2750` | `miru` | `miru-users` |
| Configs deployed under `/srv/miru/configs` | `0644` (umask; `0640` after an upgrade until redeployed) | `miru` | `miru-users` |
| `/run/miru` | `2750` | `miru` | `miru-users` |
| `/run/miru/miru.sock` | `0660` | `root` | `miru-users` |
| `/run/miru/device-api.json` (only with `enable_tcp_server`) | `0640` | `miru` | `miru-users` |

`/srv/miru/configs` and `/run/miru` are setgid (the leading `2`), so files and
folders the agent creates inside them get the `miru-users` group even though
the agent is not a member. The folder, not the file mode, is the boundary:
accounts outside `miru-users` cannot enter it. Upgrades remove access by
others (`o-rwx`) from existing configs.

Configs deployed outside `/srv/miru/configs` keep their umask-derived modes
(normally `0644`) and the `miru` group.

## Access for local applications

`miru-users` is the Linux counterpart of the Windows `Miru Agent Users` group.
Members can use the device API socket `/run/miru/miru.sock`, read the TCP
discovery file `/run/miru/device-api.json`, and read deployed configs in
`/srv/miru/configs`. Members get nothing in `/var/lib/miru`.

Add each account whose applications call the API or read configs:

```sh
sudo usermod -a -G miru-users <account>
```

Membership takes effect at the account's next login, or when a service next
starts; log out and back in, or restart the application's service. For a
systemd service, `SupplementaryGroups=miru-users` in its unit (or a drop-in)
works as well.

Never add accounts to the `miru` group: it is the agent's own primary group and
grants nothing to applications.

## Upgrading (breaking)

Before this change, apps used the `miru` group for the socket, configs in
`/srv/miru` were world-readable, and the data root (including
`auth/token.json`) was readable by every local account. Upgrading:

- tightens the data root, `auth/`, and the agent's files to owner-only;
- moves the socket, `/run/miru`, and `/srv/miru/configs` to the `miru-users`
  group and removes access by others from `/srv/miru/configs`;
- creates `miru-users` and, once, copies into it every account listed as a
  `miru` member in `/etc/group` and every account whose primary group is
  `miru`. Those accounts must log in again, or their services must restart,
  before the new membership applies.

Not migrated, and losing access on upgrade:

- applications that read `/srv/miru/configs` without being in `miru` (configs
  were world-readable, so this is most config readers);
- services that get `miru` only through systemd `Group=` or
  `SupplementaryGroups=`.

Add them to `miru-users` as shown above (for systemd services, change `miru`
to `miru-users` in `Group=` or `SupplementaryGroups=`). Configs deployed
outside `/srv/miru/configs` are unchanged.

Removing someone from `miru-users` sticks: the migration runs only when the
group is first created.

## Uninstall

`apt purge` removes `/var/lib/miru`, `/var/log/miru`, and `/srv/miru`, but keeps
the `miru-users` group (like the `miru` user and group) so memberships survive a
reinstall. Remove it with:

```sh
sudo groupdel miru-users
```
