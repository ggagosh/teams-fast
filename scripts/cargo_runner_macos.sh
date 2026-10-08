#!/usr/bin/env bash
# `cargo run` runner: signs the desktop binary for stable Keychain access, then runs it.
set -euo pipefail
binary="$1"
shift
if [[ "$(basename "$binary")" == "teamsfast" ]]; then
    "$(dirname "${BASH_SOURCE[0]}")/sign_dev.sh" "$binary"
fi
exec "$binary" "$@"
