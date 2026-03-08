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