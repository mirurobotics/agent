if [ -n "$FROM_PKG" ]; then
    log "Installing from package on local machine: '$FROM_PKG'"
    if [ ! -f "$FROM_PKG" ]; then
        fatal "The provided package does not exist on this machine: '$FROM_PKG'"
    fi
    if [ "$(file -b --mime-type "$FROM_PKG")" != "$DEB_PKG_MIME_TYPE" ]; then
        fatal "The provided package is not a valid Debian package. Expected mimetype '$DEB_PKG_MIME_TYPE' but got '$(file -b --mime-type "$FROM_PKG")'."
    fi
    if [ "$(dpkg -f "$FROM_PKG" Package)" != "$DAEMON_DEB_PKG_NAME" ]; then
        fatal "The provided package is not a valid Miru Daemon package. Expected package name '$DAEMON_DEB_PKG_NAME' but got '$(dpkg -f "$FROM_PKG" Package)'."
    fi
    if [ "$(dpkg -f "$FROM_PKG" Architecture)" != "$DEB_ARCH" ]; then
        fatal "The provided package architecture ($(dpkg -f "$FROM_PKG" Architecture)) does not match this machine's architecture ($DEB_ARCH)."
    fi
    DAEMON_DEB_PKG=$FROM_PKG

    VERSION=$(dpkg -f "$FROM_PKG" Version)
fi