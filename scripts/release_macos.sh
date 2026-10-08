#!/usr/bin/env bash
# Builds dist/TeamsFast.app and dist/TeamsFast-<version>-<arch>.zip.
#   scripts/release_macos.sh [aarch64-apple-darwin|x86_64-apple-darwin]
# Optional environment (CI secrets; never commit them):
#   MACOS_SIGNING_IDENTITY                 Developer ID Application identity in the keychain
#   APPLE_API_KEY_ID, APPLE_API_ISSUER_ID  App Store Connect API key for notarization
#   APPLE_API_KEY (base64 .p8) or APPLE_API_KEY_PATH
#   SPARKLE_PRIVATE_KEY                    Ed25519 seed from Keychain/CI secret, never a repo file
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
WORK="$(mktemp -d -t teamsfast-release)"
trap 'rm -rf "$WORK"' EXIT
export SPARKLE_FRAMEWORK_PATH
SPARKLE_FRAMEWORK_PATH="$(scripts/setup_sparkle.sh)"
# A CI release must never silently publish an app without a signed update feed.
if [[ "${GITHUB_ACTIONS:-}" == true ]]; then
    : "${SPARKLE_PRIVATE_KEY:?Configure the Sparkle signing secret before releasing}"
fi

if [[ -n "$TARGET" ]]; then
    rustup target add "$TARGET"
    cargo build --release --locked --features auto-update --bin teamsfast --target "$TARGET"
    BIN_PATH="target/$TARGET/release/teamsfast"
    case "$TARGET" in
        aarch64-apple-darwin) ARCH=arm64 ;;
        x86_64-apple-darwin) ARCH=x86_64 ;;
        *) echo "Unsupported target $TARGET" >&2; exit 1 ;;
    esac
else
    cargo build --release --locked --features auto-update --bin teamsfast
    BIN_PATH="target/release/teamsfast"
    ARCH="$(uname -m)"
fi

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/$APP_NAME"
cp packaging/macos/AppIcon.icns "$APP_DIR/Contents/Resources/"
cp packaging/macos/Info.plist "$APP_DIR/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" \
    -c "Set :CFBundleVersion $VERSION" \
    -c "Add :SUFeedURL string https://github.com/ggagosh/teams-fast/releases/latest/download/appcast-$ARCH.xml" \
    -c "Add :SUPublicEDKey string $(tr -d '\n' < packaging/macos/sparkle-public-key.txt)" \
    -c "Add :SUEnableAutomaticChecks bool true" \
    -c "Add :SUAutomaticallyUpdate bool true" \
    -c "Add :SUVerifyUpdateBeforeExtraction bool true" \
    -c "Add :SURequireSignedFeed bool true" "$APP_DIR/Contents/Info.plist"
mkdir -p "$APP_DIR/Contents/Frameworks"
FRAMEWORK="$APP_DIR/Contents/Frameworks/Sparkle.framework"
ditto "$SPARKLE_FRAMEWORK_PATH/Sparkle.framework" "$FRAMEWORK"
python3 - "$APP_DIR" <<'PY'
import pathlib, plistlib, sys
app = pathlib.Path(sys.argv[1])
version = lambda value: tuple(int(n) for n in (value.split(".") + ["0"] * 3)[:3])
minimum = plistlib.loads((app / "Contents/Info.plist").read_bytes())["LSMinimumSystemVersion"]
for path in (app / "Contents/Frameworks").rglob("Info.plist"):
    requirement = plistlib.loads(path.read_bytes()).get("LSMinimumSystemVersion")
    assert not requirement or version(requirement) <= version(minimum), f"{path}: requires macOS {requirement}, app promises {minimum}"
PY
# Sign executable code inside-out, preserving the upstream helpers' sandbox entitlements.
SIGN=(--force --options runtime --sign "${MACOS_SIGNING_IDENTITY:--}")
if [[ -n "${MACOS_SIGNING_IDENTITY:-}" ]]; then
    SIGN+=(--timestamp)
else
    echo "MACOS_SIGNING_IDENTITY not set: ad-hoc signature, local use only."
fi
for nested in "$FRAMEWORK/Versions/B/Autoupdate" \
    "$FRAMEWORK/Versions/B/Updater.app" "$FRAMEWORK/Versions/B/XPCServices/"*.xpc; do
    codesign "${SIGN[@]}" --preserve-metadata=entitlements "$nested"
done
codesign "${SIGN[@]}" "$FRAMEWORK"
codesign "${SIGN[@]}" "$APP_DIR"
codesign --verify --deep --strict --verbose=2 "$APP_DIR"

ZIP_PATH="$DIST_DIR/$APP_NAME-$VERSION-macos-$ARCH.zip"
rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"

if [[ -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]; then
    KEY_PATH="${APPLE_API_KEY_PATH:-}"
    if [[ -z "$KEY_PATH" ]]; then
        KEY_PATH="$WORK/AuthKey.p8"
        umask 077
        printf '%s' "${APPLE_API_KEY:?Set APPLE_API_KEY or APPLE_API_KEY_PATH}" | base64 -D > "$KEY_PATH"
    fi
    xcrun notarytool submit "$ZIP_PATH" --key "$KEY_PATH" --key-id "$APPLE_API_KEY_ID" \
        --issuer "$APPLE_API_ISSUER_ID" --wait --timeout 30m
    xcrun stapler staple "$APP_DIR"
    rm -f "$ZIP_PATH"
    ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"
fi

shasum -a 256 "$ZIP_PATH" | awk '{print $1}' > "$ZIP_PATH.sha256"
if [[ -n "${SPARKLE_PRIVATE_KEY:-}" ]]; then
    # Sign only the final, stapled archive. The generator also signs the XML and checks the
    # archive against the public key embedded in the app. One feed per CPU prevents mix-ups.
    mkdir "$WORK/feed"
    cp "$ZIP_PATH" "$WORK/feed/"
    printf '%s' "$SPARKLE_PRIVATE_KEY" | "$SPARKLE_FRAMEWORK_PATH/bin/generate_appcast" \
        --ed-key-file - --maximum-deltas 0 \
        --download-url-prefix "https://github.com/ggagosh/teams-fast/releases/download/v$VERSION/" \
        --link "https://github.com/ggagosh/teams-fast/releases/tag/v$VERSION" \
        -o "$WORK/appcast-$ARCH.xml" "$WORK/feed"
    printf '%s' "$SPARKLE_PRIVATE_KEY" | "$SPARKLE_FRAMEWORK_PATH/bin/sign_update" \
        --ed-key-file - --verify "$WORK/appcast-$ARCH.xml"
    python3 - "$WORK/appcast-$ARCH.xml" "$VERSION" "$ARCH" "$ZIP_PATH" <<'PY'
import pathlib, sys, xml.etree.ElementTree as ET
feed, version, arch, archive = sys.argv[1:]
ns = {"s": "http://www.andymatuschak.org/xml-namespaces/sparkle"}
items = ET.parse(feed).findall("./channel/item")
assert len(items) == 1, "Expected exactly one release in this architecture feed"
item = items[0]
assert item.findtext("s:version", namespaces=ns) == version, "Wrong update version"
asset = item.find("enclosure")
expected = f"https://github.com/ggagosh/teams-fast/releases/download/v{version}/TeamsFast-{version}-macos-{arch}.zip"
assert asset is not None and asset.get("url") == expected, "Wrong update asset"
assert int(asset.get("length")) == pathlib.Path(archive).stat().st_size, "Wrong archive size"
assert asset.get(f"{{{ns['s']}}}edSignature"), "Missing update signature"
PY
    mv "$WORK/appcast-$ARCH.xml" "$DIST_DIR/appcast-$ARCH.xml"
else
    rm -f "$DIST_DIR/appcast-$ARCH.xml"
    echo "SPARKLE_PRIVATE_KEY not set: no update feed generated (local package only)."
fi
echo "Packaged: $ZIP_PATH"
