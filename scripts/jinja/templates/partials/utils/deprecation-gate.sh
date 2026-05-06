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
