INSTALLED_VERSION=$(dpkg-query -W -f='${Version}' "$DAEMON_DEB_PKG_NAME" 2>/dev/null || echo "")
# replace '~' with '-' 
if [ -n "$INSTALLED_VERSION" ]; then
    INSTALLED_VERSION=$(echo "$INSTALLED_VERSION" | sed 's/~/-/g')
fi

if [ "$INSTALLED_VERSION" != "$VERSION" ]; then
    rm -rf "$DOWNLOAD_DIR"
    mkdir -p "$DOWNLOAD_DIR"

    # download the daemon deb package if not provided locally
    if [ -z "$DAEMON_DEB_PKG" ] || [ ! -f "$DAEMON_DEB_PKG" ]; then
        log "Downloading version ${VERSION}"
        DAEMON_DEB_PKG="$DOWNLOAD_DIR/${DAEMON_DEB_PKG_NAME}.deb"
        DAEMON_DEB_PKG_URL="https://github.com/${GITHUB_REPO}/releases/download/v${VERSION}/${DAEMON_DEB_PKG_NAME}_${VERSION}_${DEB_ARCH}.deb"
        curl -#fL "$DAEMON_DEB_PKG_URL" -o "$DAEMON_DEB_PKG" ||
            fatal "Failed to download ${DAEMON_DEB_PKG_NAME}"
    fi

    # download the checksums file
    CHECKSUM_URL="https://github.com/${GITHUB_REPO}/releases/download/v${VERSION}/agent_${VERSION}_checksums.txt"
    curl -fsSL "$CHECKSUM_URL" -o "$CHECKSUMS_FILE" || fatal "Failed to download checksums.txt"

    EXPECTED_CHECKSUM=$(grep "${DAEMON_DEB_PKG_NAME}_${VERSION}_${DEB_ARCH}.deb" "$CHECKSUMS_FILE" | cut -d ' ' -f 1)
    if [ -n "$EXPECTED_CHECKSUM" ]; then
        verify_checksum "$DAEMON_DEB_PKG" "$EXPECTED_CHECKSUM" ||
            fatal "Checksum verification failed"
    else
        fatal "Checksums not found inside $CHECKSUM_URL" 
    fi

    if [ -n "$INSTALLED_VERSION" ]; then
        log "Replacing version ${INSTALLED_VERSION} with version ${VERSION}"
    else
        log "Installing version ${VERSION}"
    fi
    sudo dpkg -i "$DAEMON_DEB_PKG" || fatal "Failed to install the daemon"

    log "Removing downloaded files"
    rm -rf "$DOWNLOAD_DIR"
else 
    log "Version ${VERSION} is already installed"
fi