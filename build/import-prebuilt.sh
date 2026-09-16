#!/usr/bin/env bash
# GoReleaser Rust-builder `tool:` that imports an externally built binary
# instead of compiling one.
#
# The builder image ships the OSS GoReleaser binary, which has no `prebuilt`
# builder (a Pro feature), so the Windows lane reuses the Rust builder with
# this script as its tool. GoReleaser invokes it from the repository root as
#
#     import-prebuilt.sh build --target=<triple> --release -p=miru-agent
#
# and afterwards copies target/<triple>/release/miru-agent.exe into dist/.
# This script puts that file in place from build/prebuilt/<os>_<arch>/, the
# directory the windows-release-build CI job's artifact is downloaded into,
# and fails if the staged binary is missing so a release can never silently
# drop the Windows archive.
set -euo pipefail

target=""
for arg in "$@"; do
	case "$arg" in
		--target=*) target="${arg#--target=}" ;;
	esac
done

if [ -z "$target" ]; then
	echo "import-prebuilt: no --target=<triple> argument in: $*" >&2
	exit 1
fi

case "$target" in
	x86_64-pc-windows-msvc)
		src="build/prebuilt/windows_amd64/miru-agent.exe"
		dst="target/$target/release/miru-agent.exe"
		;;
	*)
		echo "import-prebuilt: no prebuilt mapping for target $target" >&2
		exit 1
		;;
esac

if [ ! -f "$src" ]; then
	echo "import-prebuilt: staged binary $src is missing; the" \
		"windows-release-build artifact must be downloaded there first" >&2
	exit 1
fi

mkdir -p "$(dirname "$dst")"
cp "$src" "$dst"
ls -l "$dst"
