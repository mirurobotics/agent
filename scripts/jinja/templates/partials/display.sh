# shellcheck shell=sh
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NO_COLOR='\033[0m'

debug() { echo "${BLUE}==>${NO_COLOR} $1"; }
log() { echo "${GREEN}==>${NO_COLOR} $1"; }
# shellcheck disable=SC2317,SC2329 # part of the shared logging API; not every script calls every helper
warn() { echo "${YELLOW}Warning:${NO_COLOR} $1"; }
# shellcheck disable=SC2317,SC2329 # part of the shared logging API; not every script calls every helper
error() { echo "${RED}Error:${NO_COLOR} $1"; }
fatal() { echo "${RED}Error:${NO_COLOR} $1"; exit 1; }