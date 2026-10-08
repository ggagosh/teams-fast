#!/usr/bin/env bash
# Fetch the pinned, upstream Sparkle distribution; print its directory for SPARKLE_FRAMEWORK_PATH.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION=2.9.6
SHA256=52bf9e88cdd972fc0c81501377a880e90d47031bd8ca5462488f843e2609e192
DEST="$ROOT/target/sparkle-$VERSION"
if [[ ! -f "$DEST/.verified-$SHA256" ]]; then
    mkdir -p "$ROOT/target"
    work="$(mktemp -d "$ROOT/target/sparkle-download.XXXXXX")"
    trap 'rm -rf "$work"' EXIT
    curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
        "https://github.com/sparkle-project/Sparkle/releases/download/$VERSION/Sparkle-$VERSION.tar.xz" \
        -o "$work/archive.tar.xz"
    printf '%s  %s\n' "$SHA256" "$work/archive.tar.xz" | shasum -a 256 -c - >&2
    mkdir "$work/unpacked"
    tar -xf "$work/archive.tar.xz" -C "$work/unpacked"
    codesign --verify --deep --strict "$work/unpacked/Sparkle.framework"
    touch "$work/unpacked/.verified-$SHA256"
    # This is only the versioned, ignored dependency cache, never an installed application.
    rm -rf "$DEST"
    mv "$work/unpacked" "$DEST"
fi
codesign --verify --deep --strict "$DEST/Sparkle.framework"
printf '%s\n' "$DEST"
