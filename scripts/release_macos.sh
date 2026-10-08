#!/usr/bin/env bash
# Builds dist/TeamsFast.app and dist/TeamsFast-<version>-<arch>.zip.
#   scripts/release_macos.sh [aarch64-apple-darwin|x86_64-apple-darwin]
# Optional environment (CI secrets; never commit them):
#   MACOS_SIGNING_IDENTITY                 Developer ID Application identity in the keychain
#   APPLE_API_KEY_ID, APPLE_API_ISSUER_ID  App Store Connect API key for notarization
#   APPLE_API_KEY (base64 .p8) or APPLE_API_KEY_PATH
# Build-time app configuration (public IDs, baked in so the app opens straight to sign-in):
#   TEAMSFAST_CLIENT_ID, TEAMSFAST_TENANT, TEAMSFAST_RELAY_URL
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
APP_NAME="TeamsFast"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
TARGET="${1:-}"
DIST_DIR="$ROOT_DIR/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"

if [[ -n "$TARGET" ]]; then
    rustup target add "$TARGET"
    cargo build --release --locked --bin teamsfast --target "$TARGET"
    BIN_PATH="target/$TARGET/release/teamsfast"
    case "$TARGET" in
        aarch64-apple-darwin) ARCH=arm64 ;;
        x86_64-apple-darwin) ARCH=x86_64 ;;
        *) echo "Unsupported target $TARGET" >&2; exit 1 ;;
    esac
else
    cargo build --release --locked --bin teamsfast
    BIN_PATH="target/release/teamsfast"
    ARCH="$(uname -m)"
fi

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/$APP_NAME"
cp packaging/macos/AppIcon.icns "$APP_DIR/Contents/Resources/"
cp packaging/macos/Info.plist "$APP_DIR/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" \
    -c "Set :CFBundleVersion $VERSION" "$APP_DIR/Contents/Info.plist"

if [[ -n "${MACOS_SIGNING_IDENTITY:-}" ]]; then
    codesign --force --options runtime --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$APP_DIR"
    codesign --verify --strict --verbose=2 "$APP_DIR"
else
    echo "MACOS_SIGNING_IDENTITY not set: ad-hoc signature, local use only."
    codesign --force --sign - "$APP_DIR"
fi

ZIP_PATH="$DIST_DIR/$APP_NAME-$VERSION-macos-$ARCH.zip"
rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"

if [[ -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]; then
    KEY_PATH="${APPLE_API_KEY_PATH:-}"
    if [[ -z "$KEY_PATH" ]]; then
        KEY_PATH="$(mktemp -t authkey).p8"
        trap 'rm -f "$KEY_PATH"' EXIT
        printf '%s' "${APPLE_API_KEY:?Set APPLE_API_KEY or APPLE_API_KEY_PATH}" | base64 -D > "$KEY_PATH"
    fi
    xcrun notarytool submit "$ZIP_PATH" --key "$KEY_PATH" --key-id "$APPLE_API_KEY_ID" \
        --issuer "$APPLE_API_ISSUER_ID" --wait --timeout 30m
    xcrun stapler staple "$APP_DIR"
    rm -f "$ZIP_PATH"
    ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"
fi

shasum -a 256 "$ZIP_PATH" | awk '{print $1}' > "$ZIP_PATH.sha256"
echo "Packaged: $ZIP_PATH"
