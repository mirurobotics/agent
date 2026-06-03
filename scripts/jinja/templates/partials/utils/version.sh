# shellcheck shell=sh
if [ -z "$VERSION" ]; then
    if [ "$PRERELEASE" = true ]; then
        log "Fetching latest pre-release version..."
        VERSION=$(curl -sL "https://api.github.com/repos/${GITHUB_REPO}/releases" | 
            jq -r '.[] | select(.prerelease==true) | .tag_name' | head -n 1) || fatal "Failed to fetch latest pre-release version"
    else
        log "Fetching latest stable version..."
        VERSION=$(curl -sL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" | 
            grep "tag_name" | cut -d '"' -f 4) || fatal "Failed to fetch latest version"
    fi
fi
VERSION=$(echo "$VERSION" | cut -d 'v' -f 2)
[ -z "$VERSION" ] && fatal "Could not determine latest version"

# Validate the version is supported
MAJOR=$(echo "$VERSION" | cut -d '.' -f 1)
MINOR=$(echo "$VERSION" | cut -d '.' -f 2)
PATCH=$(echo "$VERSION" | cut -d '.' -f 3 | sed 's/[^0-9].*//')
if ! echo "$MAJOR" | grep -q '^[0-9]\+$' || ! echo "$MINOR" | grep -q '^[0-9]\+$' || ! echo "$PATCH" | grep -q '^[0-9]\+$'; then
    fatal "Could not parse version '$VERSION' to determine if it is supported"
else
    if [ "$MAJOR" -lt 0 ] || [ "$MAJOR" -eq 0 ] && [ "$MINOR" -lt 6 ]; then
        fatal "Version v$VERSION has been deprecated, please install v0.6.0 or greater"
    fi
fi
log "Version to install: ${VERSION}"

# REJECT v0.9.0+ — INSTALL VIA APT INSTEAD #
# ---------------------------------------- #
if [ "$MAJOR" -gt 0 ] || { [ "$MAJOR" -eq 0 ] && [ "$MINOR" -ge 9 ]; }; then
    fatal "Version v$VERSION cannot be installed with this script. Miru Agent v0.9.0 and later use a new apt-based provisioning workflow. See https://docs.mirurobotics.com/docs/learn/devices/provision for instructions."
fi