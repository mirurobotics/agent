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

if [ -z "$MIRU_PROVISIONING_TOKEN" ]; then
    fatal "The MIRU_PROVISIONING_TOKEN environment variable is not set"
fi

# Reset the /srv/miru directory to be owned by the miru user and group
sudo chown -R miru:miru /srv/miru

# Execute the provision flow
sudo -u miru -E env MIRU_PROVISIONING_TOKEN="$MIRU_PROVISIONING_TOKEN" /usr/sbin/miru-agent --provision $args