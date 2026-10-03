#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<EOF
Usage: $(basename "$0") [options]

Run the Debian postinst against a throwaway debian:bookworm-slim container and
check the owners, modes, and access it applies on upgrade and fresh install,
then lint the systemd units with systemd-analyze.

Options:
  --in-container  Run the checks in the current environment (used inside the
                  container; it modifies system users, groups, and paths)
  -h, --help      Show this help message
EOF
}

in_container=0
while [ $# -gt 0 ]; do
	case "$1" in
	--in-container)
		in_container=1
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		echo "unknown option: $1" >&2
		usage >&2
		exit 1
		;;
	esac
done

repo_root=$(cd "$(dirname "$0")/../../.." && pwd)
# pinned by digest so upstream tag moves cannot change the test
image=debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251

if [ "$in_container" -eq 0 ]; then
	exec docker run --rm -v "$repo_root:/src:ro" "$image" \
		/src/build/debian/tests/postinst-test.sh --in-container
fi

readonly debian_dir="$repo_root/build/debian"
# outside /tmp, which the boot-time tmpfiles run cleans
readonly postinst_out=/root/postinst.out
readonly systemctl_log=/root/systemctl.log

# Folders postinst owns, with the modes it must leave them in. All come from
# the tmpfiles.d entry: only systemctl is mocked, so postinst's
# `systemd-tmpfiles --create` runs for real.
readonly folders=(
	"/var/lib/miru 700"
	"/var/log/miru 750"
	"/srv/miru 755"
	"/run/miru 750"
)

# What an older release left inside those folders ("<d|f> <path> <mode>").
# postinst must not change any of it: the folders are the access boundary.
readonly legacy_contents=(
	"d /var/lib/miru/auth 775"
	"f /var/lib/miru/auth/token.json 644"
	"f /var/log/miru/miru.log 644"
	"d /srv/miru/configs 755"
	"d /srv/miru/configs/v1 755"
	"f /srv/miru/configs/v1/motion.json 644"
)

# ================================ assertions ================================ #

fail() {
	echo "FAIL: $*" >&2
	if [ -s "$postinst_out" ]; then
		echo "--- last postinst output ---" >&2
		cat "$postinst_out" >&2
	fi
	exit 1
}

# expect_stat <path> <mode>: <path> has <mode> and is owned by miru:miru
expect_stat() {
	local got
	got=$(stat -c '%a %U %G' "$1") || fail "stat $1"
	[ "$got" = "$2 miru miru" ] || fail "$1: got '$got', want '$2 miru miru'"
}

expect_folder_modes() {
	local entry
	for entry in "${folders[@]}"; do
		# shellcheck disable=SC2086 # split "<path> <mode>"
		expect_stat $entry
	done
}

expect_legacy_contents_unchanged() {
	local entry type path mode
	for entry in "${legacy_contents[@]}"; do
		read -r type path mode <<<"$entry"
		expect_stat "$path" "$mode"
	done
}

# expect_readable_by <account> <path>
expect_readable_by() {
	runuser -u "$1" -- cat "$2" >/dev/null 2>&1 || fail "$1 cannot read $2"
}

# expect_unreadable_by <account> <path>
expect_unreadable_by() {
	! runuser -u "$1" -- cat "$2" >/dev/null 2>&1 || fail "$1 can read $2"
}

# An unrelated account stands in for "other" local users.
expect_readable_by_others() {
	expect_readable_by nobody "$1"
}

expect_unreadable_by_others() {
	expect_unreadable_by nobody "$1"
}

expect_units_restarted() {
	local want
	want=$(printf '%s\n' \
		'daemon-reload' \
		'enable miru.socket' \
		'restart miru.socket' \
		'enable miru.service' \
		'restart miru.service')
	[ "$(cat "$systemctl_log")" = "$want" ] ||
		fail "systemctl calls: got '$(cat "$systemctl_log")', want '$want'"
}

# ================================= fixtures ================================= #

# One-time container setup: systemd tooling, a recording systemctl, the units,
# and a stub agent binary.
install_fakes() {
	apt-get update -qq >/dev/null
	DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends systemd >/dev/null

	cat >/usr/local/bin/systemctl <<EOF
#!/bin/sh
echo "\$*" >>$systemctl_log
EOF
	chmod 0755 /usr/local/bin/systemctl

	cp "$debian_dir/miru-agent.tmpfiles" /usr/lib/tmpfiles.d/miru-agent.conf
	cp "$debian_dir/miru.socket" "$debian_dir/miru.service" /lib/systemd/system/
	printf '#!/bin/sh\nexit 0\n' >/usr/sbin/miru-agent
	chmod 0755 /usr/sbin/miru-agent
}

# Return the container to a never-installed state.
reset_system() {
	local entry
	for entry in "${folders[@]}"; do
		rm -rf "${entry%% *}"
	done
	rm -f /etc/tmpfiles.d/miru-agent.conf
	userdel app 2>/dev/null || true
	userdel miru 2>/dev/null || true
	groupdel miru 2>/dev/null || true
	rm -f "$postinst_out" "$systemctl_log"
}

# Lay out an install from a release before the folders were tightened: every
# folder world-readable, the token and logs 0644.
seed_legacy_install() {
	local entry type path mode
	groupadd -r miru
	useradd -r -g miru -s /bin/false miru
	for entry in "${folders[@]}"; do
		mkdir -p "${entry%% *}"
		chmod 755 "${entry%% *}"
	done
	chmod 750 /run/miru
	for entry in "${legacy_contents[@]}"; do
		read -r type path mode <<<"$entry"
		if [ "$type" = d ]; then mkdir -p "$path"; else echo '{}' >"$path"; fi
		chmod "$mode" "$path"
	done
	chown -R miru:miru /var/lib/miru /var/log/miru /srv/miru /run/miru
}

# run_postinst <args...>: run postinst, recording its output and systemctl calls
run_postinst() {
	: >"$systemctl_log"
	sh "$debian_dir/postinst" "$@" >"$postinst_out" 2>&1
}

expect_postinst_ok() {
	run_postinst "$@" || fail "postinst $* exited non-zero"
}

# ================================== tests =================================== #

test_upgrade_sets_folder_modes() {
	seed_legacy_install
	expect_postinst_ok configure 0.10.3
	expect_folder_modes
	expect_legacy_contents_unchanged
	expect_units_restarted
}

test_upgrade_makes_state_and_logs_private() {
	seed_legacy_install
	expect_readable_by_others /var/lib/miru/auth/token.json
	expect_postinst_ok configure 0.10.3
	expect_unreadable_by_others /var/lib/miru/auth/token.json
	expect_unreadable_by_others /var/log/miru/miru.log
	expect_readable_by_others /srv/miru/configs/v1/motion.json
}

test_reconfigure_is_idempotent() {
	seed_legacy_install
	expect_postinst_ok configure 0.10.3
	expect_postinst_ok configure 0.10.3
	expect_folder_modes
	expect_legacy_contents_unchanged
	expect_units_restarted
}

test_fresh_install_creates_account_and_folders() {
	expect_postinst_ok configure
	id miru >/dev/null 2>&1 || fail "postinst did not create the miru user"
	expect_folder_modes
	expect_units_restarted
}

test_boot_restores_drifted_folder_modes() {
	local boot_cmd
	seed_legacy_install
	expect_postinst_ok configure 0.10.3
	chmod 755 /var/lib/miru /var/log/miru
	# what boot runs: systemd-tmpfiles-setup.service over every tmpfiles.d config
	boot_cmd=$(sed -n 's/^ExecStart=//p' /lib/systemd/system/systemd-tmpfiles-setup.service)
	[ -n "$boot_cmd" ] || fail "systemd-tmpfiles-setup.service has no ExecStart"
	# shellcheck disable=SC2086 # split the command line
	$boot_cmd || fail "boot tmpfiles run failed: $boot_cmd"
	expect_folder_modes
}

test_tmpfiles_failure_fails_configure() {
	# systemd-tmpfiles fails when an entry's user does not exist
	sed 's|^\(d /srv/miru [0-7]*\) miru|\1 no-such-user|' \
		"$debian_dir/miru-agent.tmpfiles" >/etc/tmpfiles.d/miru-agent.conf
	! run_postinst configure || fail "postinst succeeded although systemd-tmpfiles failed"
	grep -q 'Failed to create the miru directories' "$postinst_out" ||
		fail "postinst did not report the systemd-tmpfiles failure"
}

# The opt-in in build/debian/README.md, run as documented.
test_admin_can_restrict_srv_miru() {
	seed_legacy_install
	useradd -r -G miru -s /bin/false app
	expect_postinst_ok configure 0.10.3
	cp /usr/lib/tmpfiles.d/miru-agent.conf /etc/tmpfiles.d/miru-agent.conf
	sed -i 's|^d /srv/miru 0755 |d /srv/miru 0750 |' /etc/tmpfiles.d/miru-agent.conf
	systemd-tmpfiles --create miru-agent.conf || fail "systemd-tmpfiles failed"
	[ "$(stat -c '%a %U %G' /srv/miru)" = '750 miru miru' ] ||
		fail "documented check failed: $(stat -c '%a %U %G' /srv/miru)"
	expect_unreadable_by_others /srv/miru/configs/v1/motion.json
	expect_readable_by app /srv/miru/configs/v1/motion.json

	# and it survives an upgrade
	expect_postinst_ok configure 0.10.3
	expect_stat /srv/miru 750
	expect_unreadable_by_others /srv/miru/configs/v1/motion.json
	expect_readable_by app /srv/miru/configs/v1/motion.json
}

test_units_are_valid() {
	local verify_out
	grep -qx 'SocketGroup=miru' "$debian_dir/miru.socket" ||
		fail "miru.socket: want SocketGroup=miru"
	# systemd applies these at service start, so they must match tmpfiles.d
	grep -qx 'StateDirectoryMode=0700' "$debian_dir/miru.service" ||
		fail "miru.service: want StateDirectoryMode=0700"
	grep -qx 'LogsDirectoryMode=0750' "$debian_dir/miru.service" ||
		fail "miru.service: want LogsDirectoryMode=0750"
	verify_out=$(systemd-analyze verify /lib/systemd/system/miru.socket \
		/lib/systemd/system/miru.service 2>&1) ||
		fail "systemd-analyze verify failed:
$verify_out"
	# verify exits 0 on ignored settings; systemd 252 (bookworm) says "Unknown
	# key", newer releases "Unknown key name"
	! grep -Eq 'Unknown (key|section)|Failed to parse' <<<"$verify_out" ||
		fail "systemd-analyze verify reported unit errors:
$verify_out"
}

main() {
	local t
	install_fakes
	for t in $(declare -F | awk '$3 ~ /^test_/ { print $3 }'); do
		reset_system
		"$t"
		echo "PASS ${t#test_}"
	done
	echo "all postinst checks passed"
}

main
