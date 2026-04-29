#!/bin/sh
set -e

# Script: staging-provision.sh
# Jinja Template: provision.j2
# Build Timestamp: 2026-04-29T16:37:22.018919
# Description: Provision a device & install the Miru Agent in the staging environment

# DISPLAY #
# ======= #
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NO_COLOR='\033[0m'

debug() { echo "${BLUE}==>${NO_COLOR} $1"; }
log() { echo "${GREEN}==>${NO_COLOR} $1"; }
warn() { echo "${YELLOW}Warning:${NO_COLOR} $1"; }
error() { echo "${RED}Error:${NO_COLOR} $1"; }
fatal() { echo "${RED}Error:${NO_COLOR} $1"; exit 1; }

# ARGUMENTS #
# ========= #
DEBUG=false
for arg in "$@"; do
    case $arg in
    --debug=*) DEBUG="${arg#*=}";;
    --debug) DEBUG=true;;
    esac
done

PRERELEASE=false
for arg in "$@"; do
    case $arg in
    --prerelease=*) PRERELEASE="${arg#*=}";;
    --prerelease) PRERELEASE=true;;
    esac
done
if [ "$DEBUG" = true ]; then
    debug "prerelease: '$PRERELEASE' (should be true or false)"
fi

VERSION=''
for arg in "$@"; do
    case $arg in
    --version=*) VERSION="${arg#*=}";;
    esac
done
if [ "$DEBUG" = true ]; then
    debug "version: '$VERSION' (should be a semantic version string like 'v1.2.3')"
fi

DEVICE_NAME=$(hostname)
for arg in "$@"; do
    case $arg in
    --device-name=*) DEVICE_NAME="${arg#*=}";;
    esac
done
if [ -z "$DEVICE_NAME" ]; then
    fatal "The --device-name argument is required but not provided"
fi
if [ "$DEBUG" = true ]; then
    debug "device-name: '$DEVICE_NAME' (should be the name of the device)"
fi

FROM_PKG=''
for arg in "$@"; do
    case $arg in
    --from-pkg=*) FROM_PKG="${arg#*=}";;
    esac
done
if [ "$DEBUG" = true ]; then
    debug "from-pkg: '$FROM_PKG' (should be the path to the agent package on this machine)"
fi

ALLOW_REACTIVATION=false
for arg in "$@"; do
    case $arg in
    --allow-reactivation=*) ALLOW_REACTIVATION="${arg#*=}";;
    --allow-reactivation) ALLOW_REACTIVATION=true;;
    esac
done
if [ "$DEBUG" = true ]; then
    debug "allow-reactivation: '$ALLOW_REACTIVATION' (should be true or false)"
fi

BACKEND_HOST="https://staging.api.mirurobotics.com"
for arg in "$@"; do
    case $arg in
    --backend-host=*) BACKEND_HOST="${arg#*=}";;
    esac
done
if [ "$DEBUG" = true ]; then
    debug "backend-host: '$BACKEND_HOST' (should be the URL of the backend host)"
fi

MQTT_BROKER_HOST="staging.mqtt.mirurobotics.com"
for arg in "$@"; do
    case $arg in
    --mqtt-broker-host=*) MQTT_BROKER_HOST="${arg#*=}";;
    esac
done
if [ "$DEBUG" = true ]; then
    debug "mqtt-broker-host: '$MQTT_BROKER_HOST' ()"
fi

# UTILITIES #
# ========= #
cmd_exists() { 
    command -v "$1" >/dev/null 2>&1
}

for cmd in curl grep cut jq; do
    cmd_exists "$cmd" || fatal "$cmd is required but not installed"
done


verify_checksum() {
    file=$1
    expected_checksum=$2

    if [ -z "$expected_checksum" ]; then
        fatal "Expected checksum is required but not provided"
    fi
    if [ -z "$file" ]; then
        fatal "File is required but not provided"
    fi

    if cmd_exists sha256sum; then
        # use printf here for precise control over the spaces since sha256sum requires exactly two spaces in between the checksum and the file
        printf "%s  %s\n" "$expected_checksum" "$file" | sha256sum -c >/dev/null 2>&1 || {
            fatal "Checksum verification failed using sha256sum"
        }
    elif cmd_exists shasum; then
        printf "%s  %s\n" "$expected_checksum" "$file" | shasum -a 256 -c >/dev/null 2>&1 || {
            fatal "Checksum verification failed using shasum"
        }
    else
        fatal "Could not verify checksum: no sha256sum or shasum command found"
    fi
}

# VARIABLES #
# ========= #
ARCH="$(uname -m)"
DOWNLOAD_DIR="$HOME/.miru/downloads"
AGENT_DEB_PKG_NAME="miru-agent"
GITHUB_REPO="mirurobotics/agent"
CHECKSUMS_FILE="$DOWNLOAD_DIR/checksums.txt"
DEB_PKG_MIME_TYPE="application/vnd.debian.binary-package"

# MAIN LOGIC #
# ========== #
OS=$(uname -s)
if [ "$OS" != "Linux" ]; then
    fatal "'${OS}' is not a supported operating system, the Miru Agent is only supported on Linux machines"
fi
DEB_ARCH=$ARCH
case $DEB_ARCH in
    x86_64|amd64) DEB_ARCH="amd64" ;;
    aarch64|arm64) DEB_ARCH="arm64" ;;
    *) fatal "Unsupported architecture: $DEB_ARCH" ;;
esac


# USE PROVIDED PACKAGE #
# -------------------- #
if [ -n "$FROM_PKG" ]; then
    log "Installing from package on local machine: '$FROM_PKG'"
    if [ ! -f "$FROM_PKG" ]; then
        fatal "The provided package does not exist on this machine: '$FROM_PKG'"
    fi
    if [ "$(file -b --mime-type "$FROM_PKG")" != "$DEB_PKG_MIME_TYPE" ]; then
        fatal "The provided package is not a valid Debian package. Expected mimetype '$DEB_PKG_MIME_TYPE' but got '$(file -b --mime-type "$FROM_PKG")'."
    fi
    if [ "$(dpkg -f "$FROM_PKG" Package)" != "$AGENT_DEB_PKG_NAME" ]; then
        fatal "The provided package is not a valid Miru Agent package. Expected package name '$AGENT_DEB_PKG_NAME' but got '$(dpkg -f "$FROM_PKG" Package)'."
    fi
    if [ "$(dpkg -f "$FROM_PKG" Architecture)" != "$DEB_ARCH" ]; then
        fatal "The provided package architecture ($(dpkg -f "$FROM_PKG" Architecture)) does not match this machine's architecture ($DEB_ARCH)."
    fi
    AGENT_DEB_PKG=$FROM_PKG

    VERSION=$(dpkg -f "$FROM_PKG" Version)
fi

# DEPRECATION GATE (v0.9.0) #
# ------------------------- #
# These scripts are being phased out as of agent v0.9.0. Reject any request
# that targets v0.9.0 or later, and reject the "latest" path (--version unset
# and --from-pkg unset) because "latest" now resolves to v0.9.0+. Customers
# must use the installation method documented at
# https://docs.mirurobotics.com/docs/agent-sdk.

if [ -z "$FROM_PKG" ] && [ -z "$VERSION" ]; then
    error "These installation scripts are deprecated as of agent v0.9.0."
    error "Resolving the 'latest' release is no longer supported here because"
    error "the latest release is v0.9.0 or newer."
    error "If you need a pre-v0.9.0 release, pass an explicit --version=vX.Y.Z (where X.Y.Z is < 0.9.0)."
    fatal "Otherwise, see https://docs.mirurobotics.com/docs/agent-sdk for the supported installation method."
fi

if [ -n "$VERSION" ]; then
    # Strip leading 'v' so users can pass either 'v0.8.5' or '0.8.5'.
    DEPRECATION_VERSION=$(echo "$VERSION" | sed 's/^v//')
    DEPRECATION_MAJOR=$(echo "$DEPRECATION_VERSION" | cut -d '.' -f 1)
    DEPRECATION_MINOR=$(echo "$DEPRECATION_VERSION" | cut -d '.' -f 2)
    if echo "$DEPRECATION_MAJOR" | grep -q '^[0-9]\+$' && echo "$DEPRECATION_MINOR" | grep -q '^[0-9]\+$'; then
        if [ "$DEPRECATION_MAJOR" -ge 1 ] || { [ "$DEPRECATION_MAJOR" -eq 0 ] && [ "$DEPRECATION_MINOR" -ge 9 ]; }; then
            error "These installation scripts are deprecated as of agent v0.9.0."
            error "You requested v$DEPRECATION_VERSION, which is v0.9.0 or newer."
            error "These scripts can only be used to install pre-v0.9.0 releases."
            fatal "See https://docs.mirurobotics.com/docs/agent-sdk for the supported installation method."
        fi
    fi
    unset DEPRECATION_VERSION DEPRECATION_MAJOR DEPRECATION_MINOR
fi

# PROVISION THE DEVICE #
# --------------------- #
if [ -z "$MIRU_API_KEY" ]; then
    echo "MIRU_API_KEY is not set"
    exit 1
fi

response_body=$(curl --request POST \
  --url "$BACKEND_HOST"/v1/devices \
  --header 'Content-Type: application/json' \
  --header "X-API-Key: $MIRU_API_KEY" \
  --header "Miru-Version: 2026-03-09.tetons" \
  --data "{
  \"name\": \"$DEVICE_NAME\"
}" \
  --write-out "\n%{http_code}" \
  --silent)

# Extract HTTP status code (last line) and response body (everything else)
http_code=$(echo "$response_body" | tail -n1)
response_body=$(echo "$response_body" | head -n -1)

# Check if the request succeeded
if [ "$http_code" -eq 200 ] || [ "$http_code" -eq 201 ]; then
    log "Created device '$DEVICE_NAME'"
    device="$response_body"
elif [ "$http_code" -eq 409 ]; then
    log "Device '$DEVICE_NAME' already exists"
    # Search for the device by name
    response_body=$(curl --request GET \
    --url "$BACKEND_HOST"/v1/devices?name="$DEVICE_NAME" \
    --header "X-API-Key: $MIRU_API_KEY" \
    --header "Miru-Version: 2026-03-09.tetons" \
    --write-out "\n%{http_code}" \
    --silent)

    http_code=$(echo "$response_body" | tail -n1)
    response_body=$(echo "$response_body" | head -n -1)

    # check there is only one device
    if [ "$(echo "$response_body" | jq -r '.data | length')" -ne 1 ]; then
        error "Expected exactly one device with name '$DEVICE_NAME'. Instead got:"
        fatal "$response_body"
    fi

    # Extract the first device from the array since the endpoint returns a list
    device=$(echo "$response_body" | jq -r '.data[0]')
else
    error "Device creation failed (HTTP status $http_code)"
    error "Response body:"
    fatal "$response_body"
fi

device_id=$(echo "$device" | jq -r '.id')
device_name=$(echo "$device" | jq -r '.name')


log "Creating activation token for device '$device_name'"
log "Allow reactivation: $ALLOW_REACTIVATION (must be true if the device has been activated before)"
response_body=$(curl --request POST \
  --url "$BACKEND_HOST"/v1/devices/"$device_id"/activation_token \
  --header "X-API-Key: $MIRU_API_KEY" \
  --header "Miru-Version: 2026-03-09.tetons" \
  --data "{
  \"allow_reactivation\": $ALLOW_REACTIVATION
}" \
  --write-out "\n%{http_code}" \
  --silent)

# Extract HTTP status code (last line) and response body (everything else)
http_code=$(echo "$response_body" | tail -n1)
response_body=$(echo "$response_body" | head -n -1)

# Check if the request succeeded
if [ "$http_code" -eq 200 ] || [ "$http_code" -eq 201 ]; then
    log "Successfully created activation token"
    MIRU_ACTIVATION_TOKEN=$(echo "$response_body" | jq -r '.token')
else
    error "Activation token request failed (HTTP status $http_code)"
    error "Response body:"
    fatal "$response_body"
fi

# DETERMINE THE VERSION #
# --------------------- #
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

# DOWNLOAD THE AGENT #
# ------------------ #
INSTALLED_VERSION=$(dpkg-query -W -f='${Version}' "$AGENT_DEB_PKG_NAME" 2>/dev/null || echo "")
# replace '~' with '-' 
if [ -n "$INSTALLED_VERSION" ]; then
    INSTALLED_VERSION=$(echo "$INSTALLED_VERSION" | sed 's/~/-/g')
fi

if [ "$INSTALLED_VERSION" != "$VERSION" ]; then
    rm -rf "$DOWNLOAD_DIR"
    mkdir -p "$DOWNLOAD_DIR"

    # download the agent deb package if not provided locally
    if [ -z "$AGENT_DEB_PKG" ] || [ ! -f "$AGENT_DEB_PKG" ]; then
        log "Downloading version ${VERSION}"
        AGENT_DEB_PKG="$DOWNLOAD_DIR/${AGENT_DEB_PKG_NAME}.deb"
        AGENT_DEB_PKG_URL="https://github.com/${GITHUB_REPO}/releases/download/v${VERSION}/${AGENT_DEB_PKG_NAME}_${VERSION}_${DEB_ARCH}.deb"
        curl -#fL "$AGENT_DEB_PKG_URL" -o "$AGENT_DEB_PKG" ||
            fatal "Failed to download ${AGENT_DEB_PKG_NAME}"
    fi

    # download the checksums file
    CHECKSUM_URL="https://github.com/${GITHUB_REPO}/releases/download/v${VERSION}/agent_${VERSION}_checksums.txt"
    curl -fsSL "$CHECKSUM_URL" -o "$CHECKSUMS_FILE" || fatal "Failed to download checksums.txt"

    EXPECTED_CHECKSUM=$(grep "${AGENT_DEB_PKG_NAME}_${VERSION}_${DEB_ARCH}.deb" "$CHECKSUMS_FILE" | cut -d ' ' -f 1)
    if [ -n "$EXPECTED_CHECKSUM" ]; then
        verify_checksum "$AGENT_DEB_PKG" "$EXPECTED_CHECKSUM" ||
            fatal "Checksum verification failed"
    else
        fatal "Checksums not found inside $CHECKSUM_URL" 
    fi

    if [ -n "$INSTALLED_VERSION" ]; then
        log "Replacing version ${INSTALLED_VERSION} with version ${VERSION}"
    else
        log "Installing version ${VERSION}"
    fi
    sudo dpkg -i "$AGENT_DEB_PKG" || fatal "Failed to install the agent"

    log "Removing downloaded files"
    rm -rf "$DOWNLOAD_DIR"
else 
    log "Version ${VERSION} is already installed"
fi

# ACTIVATE THE AGENT #
# ------------------ #
cleanup() {
    exit_code=$?

    # restart the agent
    log "Restarting the Miru Agent"
    sudo systemctl restart miru >/dev/null 2>&1

    exit $exit_code
}

trap cleanup EXIT INT TERM QUIT HUP

log "Activating the Miru Agent..."
if systemctl is-active --quiet miru; then
    log "Stopping the currently running agent"
    sudo systemctl stop miru >/dev/null 2>&1
fi

# Collect the arguments
args=""
args="$args --backend-host=$BACKEND_HOST"
args="$args --mqtt-broker-host=$MQTT_BROKER_HOST"
if [ -n "$DEVICE_NAME" ]; then
    args="$args --device-name=$DEVICE_NAME"
fi

if [ -z "$MIRU_ACTIVATION_TOKEN" ]; then
    fatal "The MIRU_ACTIVATION_TOKEN environment variable is not set"
fi

# Reset the /srv/miru directory to be owned by the miru user and group
sudo chown -R miru:miru /srv/miru

# Execute the installer
sudo -u miru -E env MIRU_ACTIVATION_TOKEN="$MIRU_ACTIVATION_TOKEN" /usr/sbin/miru-agent --install $args
exit 0