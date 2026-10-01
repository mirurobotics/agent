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

if [ "$in_container" -eq 0 ]; then
	exec docker run --rm -v "$repo_root:/src:ro" debian:bookworm-slim \
		/src/build/debian/tests/postinst-test.sh --in-container
fi

debian_dir="$repo_root/build/debian"
postinst_out=/tmp/postinst.out
systemctl_log=/tmp/systemctl.log

fail() {
	echo "FAIL: $*" >&2
	if [ -f "$postinst_out" ]; then
		echo "--- last postinst output ---" >&2
		cat "$postinst_out" >&2
	fi
	exit 1
}

pass() {
	echo "PASS $*"
}

run_postinst() {
	: >"$systemctl_log"
	sh "$debian_dir/postinst" "$@" >"$postinst_out" 2>&1 || fail "postinst $* exited non-zero"
}

# expect_stat <path> '<mode> <owner> <group>'
expect_stat() {
	local got
	got=$(stat -c '%a %U %G' "$1") || fail "stat $1"
	[ "$got" = "$2" ] || fail "$1: got '$got', want '$2'"
}

expect_table() {
	expect_stat /var/lib/miru '700 miru miru'
	expect_stat /var/log/miru '750 miru miru'
	expect_stat /srv/miru '755 miru miru'
	expect_stat /run/miru '750 miru miru'
}

# The folders are the boundary; postinst leaves everything inside them as is.
expect_contents_unchanged() {
	expect_stat /var/lib/miru/auth '775 miru miru'
	expect_stat /var/lib/miru/auth/token.json '644 miru miru'
	expect_stat /var/log/miru/miru.log '644 miru miru'
	expect_stat /srv/miru/configs '755 miru miru'
	expect_stat /srv/miru/configs/v1/motion.json '644 miru miru'
}

# expect_access <path> <yes|no>: whether an unrelated account can read <path>
expect_access() {
	if runuser -u nobody -- cat "$1" >/dev/null 2>&1; then
		[ "$2" = yes ] || fail "nobody can read $1"
	else
		[ "$2" = no ] || fail "nobody cannot read $1"
	fi
}

expect_systemctl_log() {
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

# ---------------------------------- setup ---------------------------------- #
apt-get update -qq >/dev/null
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends systemd >/dev/null

# Record systemctl calls instead of talking to a (missing) systemd manager.
cat >/usr/local/bin/systemctl <<'MOCK'
#!/bin/sh
echo "$*" >>/tmp/systemctl.log
exit 0
MOCK
chmod 0755 /usr/local/bin/systemctl

cp "$debian_dir/miru-agent.tmpfiles" /usr/lib/tmpfiles.d/miru-agent.conf
cp "$debian_dir/miru.socket" "$debian_dir/miru.service" /lib/systemd/system/
grep -qx 'SocketGroup=miru' /lib/systemd/system/miru.socket ||
	fail "miru.socket: want SocketGroup=miru"
printf '#!/bin/sh\nexit 0\n' >/usr/sbin/miru-agent
chmod 0755 /usr/sbin/miru-agent
pass "setup"

# ------------------------------ upgrade (legacy) ----------------------------- #
groupadd -r miru
useradd -r -g miru -s /bin/false miru

mkdir -p /var/lib/miru/auth /var/log/miru /run/miru /srv/miru/configs/v1
echo '{}' >/var/lib/miru/auth/token.json
echo log >/var/log/miru/miru.log
echo '{}' >/srv/miru/configs/v1/motion.json
chown -R miru:miru /var/lib/miru /var/log/miru /run/miru /srv/miru
chmod 755 /var/lib/miru /var/log/miru /srv/miru /srv/miru/configs \
	/srv/miru/configs/v1
chmod 775 /var/lib/miru/auth
chmod 644 /var/lib/miru/auth/token.json /var/log/miru/miru.log \
	/srv/miru/configs/v1/motion.json
chmod 750 /run/miru
expect_access /var/lib/miru/auth/token.json yes

run_postinst configure 0.10.3
expect_table
pass "upgrade: owners and modes"

expect_contents_unchanged
pass "upgrade: contents untouched"

expect_access /var/lib/miru/auth/token.json no
expect_access /var/log/miru/miru.log no
expect_access /srv/miru/configs/v1/motion.json yes
pass "upgrade: state and logs private, configs readable"

expect_systemctl_log
pass "upgrade: systemctl calls"

# ------------------------------- idempotence ------------------------------- #
run_postinst configure 0.10.3
expect_table
expect_contents_unchanged
expect_systemctl_log
pass "idempotence"

# ------------------------------ fresh install ------------------------------ #
rm -rf /var/lib/miru /var/log/miru /srv/miru /run/miru
run_postinst configure
expect_table
pass "fresh install: owners and modes"

# ------------------------------- unit files -------------------------------- #
rm -f "$postinst_out"
if ! verify_out=$(systemd-analyze verify /lib/systemd/system/miru.socket \
	/lib/systemd/system/miru.service 2>&1); then
	fail "systemd-analyze verify failed:
$verify_out"
fi
# verify exits 0 on ignored settings; systemd 252 (bookworm) says "Unknown key",
# newer releases "Unknown key name"
if grep -Eq 'Unknown (key|section)|Failed to parse' <<<"$verify_out"; then
	fail "systemd-analyze verify reported unit errors:
$verify_out"
fi
pass "systemd-analyze verify"

echo "all postinst checks passed"
