#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<EOF
Usage: $(basename "$0") [options]

Run the Debian postinst against a throwaway debian:bookworm-slim container and
check the owners, modes, and symlink handling it applies on upgrade and fresh
install, then lint the systemd units with systemd-analyze.

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

expect_upgrade_table() {
	expect_stat /var/lib/miru '700 miru miru'
	expect_stat /var/lib/miru/auth '700 miru miru'
	expect_stat /var/lib/miru/auth/token.json '600 miru miru'
	expect_stat /var/lib/miru/auth/private_key.pem '600 miru miru'
	expect_stat /var/lib/miru/auth/public_key.pem '640 miru miru'
	expect_stat /var/lib/miru/device.json '600 miru miru'
	expect_stat /var/lib/miru/events '700 miru miru'
	expect_stat /var/lib/miru/events/events.jsonl '600 miru miru'
	expect_stat /var/log/miru '750 miru miru'
	expect_stat /var/log/miru/miru.log '640 miru miru'
	expect_stat /srv/miru '755 miru miru'
	# configs stay world-readable; the socket's /run/miru stays group miru
	expect_stat /srv/miru/configs '755 miru miru'
	expect_stat /srv/miru/configs/v1 '755 miru miru'
	expect_stat /srv/miru/configs/v1/motion.json '644 miru miru'
	expect_stat /run/miru '750 miru miru'
}

# expect_systemctl_log <token mode seen at stop>
expect_systemctl_log() {
	local want
	want=$(printf '%s\n' \
		'stop miru.socket miru.service' \
		"token $1" \
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

# Record systemctl calls instead of talking to a (missing) systemd manager. The
# token mode at stop time shows whether permissions changed before the stop.
cat >/usr/local/bin/systemctl <<'EOF'
#!/bin/sh
echo "$*" >>/tmp/systemctl.log
if [ "$1" = stop ]; then
	echo "token $(stat -c %a /var/lib/miru/auth/token.json 2>/dev/null || echo missing)" >>/tmp/systemctl.log
fi
exit 0
EOF
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

mkdir -p /var/lib/miru/auth /var/lib/miru/events /var/log/miru /run/miru \
	/srv/miru/configs/v1
echo '{}' >/var/lib/miru/auth/token.json
echo key >/var/lib/miru/auth/private_key.pem
echo key >/var/lib/miru/auth/public_key.pem
echo '{}' >/var/lib/miru/device.json
echo '{}' >/var/lib/miru/events/events.jsonl
echo log >/var/log/miru/miru.log
echo '{}' >/srv/miru/configs/v1/motion.json
chown -R miru:miru /var/lib/miru /var/log/miru /run/miru /srv/miru
chmod 755 /var/lib/miru /var/lib/miru/events /var/log/miru /srv/miru \
	/srv/miru/configs /srv/miru/configs/v1
chmod 775 /var/lib/miru/auth
chmod 644 /var/lib/miru/auth/token.json /var/lib/miru/device.json \
	/var/lib/miru/events/events.jsonl /var/log/miru/miru.log \
	/srv/miru/configs/v1/motion.json
chmod 600 /var/lib/miru/auth/private_key.pem
chmod 640 /var/lib/miru/auth/public_key.pem
chmod 750 /run/miru

run_postinst configure 0.10.3
expect_upgrade_table
pass "upgrade: owners and modes"

expect_systemctl_log 644
pass "upgrade: service stopped before permissions change"

# ------------------------------- idempotence ------------------------------- #
runuser -u miru -- sh -c \
	'umask 022; mkdir -p /srv/miru/configs/new && echo x > /srv/miru/configs/new/f'
run_postinst configure 0.10.3
expect_upgrade_table
expect_stat /srv/miru/configs/new '755 miru miru'
expect_stat /srv/miru/configs/new/f '644 miru miru'
pass "idempotence: owners and modes"

expect_systemctl_log 600
pass "idempotence: systemctl calls"

# ----------------------------- symlink safety ------------------------------ #
mkdir /victim /victim_auth
chmod 755 /victim /victim_auth
echo key >/victim_auth/public_key.pem
chmod 644 /victim_auth/public_key.pem
echo x >/victim_f
chmod 644 /victim_f

rm -rf /srv/miru/configs
ln -s /victim /srv/miru/configs
chown -h miru:miru /srv/miru/configs
mv /var/lib/miru/auth /tmp/auth.saved
ln -s /victim_auth /var/lib/miru/auth
ln -s /victim_f /var/lib/miru/evil
ln -s /victim_f /var/log/miru/evil
chown -h miru:miru /var/lib/miru/auth /var/lib/miru/evil /var/log/miru/evil

run_postinst configure 0.10.3
[ -L /srv/miru/configs ] || fail "/srv/miru/configs is no longer a symlink"
expect_stat /victim '755 root root'
expect_stat /victim_auth '755 root root'
expect_stat /victim_auth/public_key.pem '644 root root'
expect_stat /victim_f '644 root root'
expect_stat /srv/miru '755 miru miru'
pass "symlinks: targets outside the trees untouched"

rm /srv/miru/configs /var/lib/miru/auth /var/lib/miru/evil /var/log/miru/evil
mv /tmp/auth.saved /var/lib/miru/auth

# ------------------------------ fresh install ------------------------------ #
rm -rf /var/lib/miru /var/log/miru /srv/miru /run/miru
run_postinst configure
expect_stat /var/lib/miru '700 miru miru'
expect_stat /var/log/miru '750 miru miru'
expect_stat /srv/miru '755 miru miru'
expect_stat /run/miru '750 miru miru'
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
