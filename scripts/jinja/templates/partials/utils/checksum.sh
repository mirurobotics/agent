# shellcheck shell=sh
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