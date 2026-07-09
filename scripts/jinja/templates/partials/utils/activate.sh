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

# Execute the installer. The activation token is a secret, so it is streamed to
# the miru user over stdin and exported inside the privileged shell rather than
# passed as an `env VAR=...` argument. Command arguments are world-readable via
# /proc/<pid>/cmdline on Linux, so keeping the token out of argv stops local
# users from reading it while the installer runs. Exporting after the sudo
# transition also avoids relying on the sudoers environment-preservation policy.
sudo -u miru -E sh -c '
    IFS= read -r MIRU_ACTIVATION_TOKEN
    export MIRU_ACTIVATION_TOKEN
    exec /usr/sbin/miru-agent --install "$@"
' miru-agent $args <<EOF
$MIRU_ACTIVATION_TOKEN
EOF