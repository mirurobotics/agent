# shellcheck shell=sh
INSTALLED_VERSION=$(dpkg-query -W -f='${Version}' "$AGENT_DEB_PKG_NAME" 2>/dev/null || echo "")
# replace '~' with '-'
if [ -n "$INSTALLED_VERSION" ]; then
    # shellcheck disable=SC2001 # POSIX sh lacks ${var//search/replace}; sed is required
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