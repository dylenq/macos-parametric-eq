#!/bin/sh
# Builds a universal release binary and wraps it in target/EQ.app. macOS only grants system
# audio capture to a bundle with NSAudioCaptureUsageDescription, so run the app from there.
# Pass --zip to also write target/EQ.zip and its SHA-256 checksum for sharing.
#
# Signing is configured with environment variables, which may also be kept in a .env file
# next to Cargo.toml (see .env.example):
#   EQ_SIGN_IDENTITY     codesign identity. "-", the default, signs ad hoc.
#   EQ_NOTARY_PROFILE    notarytool keychain profile. With --zip, the zip is notarized and
#                        the ticket stapled to the app.
#   EQ_NOTARY_KEYCHAIN   keychain holding that profile, if not the default one.
#   EQ_HARDENED_RUNTIME  0 signs without the hardened runtime.
set -eu
cd "$(dirname "$0")/.."
zip=${1:-}

if [ -f .env ]; then
    set -a
    . ./.env
    set +a
fi
identity=${EQ_SIGN_IDENTITY:--}

export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-14.2}
cargo build --release --target aarch64-apple-darwin --target x86_64-apple-darwin

app=target/EQ.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp bundle/Info.plist "$app/Contents/Info.plist"
# macOS 26 and later draw the layered icon from Assets.car, which needs Xcode's actool;
# earlier versions, and builds without Xcode, use the .icns.
if xcrun --find actool >/dev/null 2>&1; then
    partial=$(mktemp)
    xcrun actool bundle/AppIcon.icon --compile "$app/Contents/Resources" \
        --platform macosx --minimum-deployment-target 14.2 --app-icon AppIcon \
        --output-partial-info-plist "$partial" >/dev/null
    rm -f "$partial"
else
    echo "actool not found: building with the flat icon only" >&2
fi
cp bundle/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
lipo -create -output "$app/Contents/MacOS/eq" \
    target/aarch64-apple-darwin/release/eq \
    target/x86_64-apple-darwin/release/eq

# The hardened runtime keeps other code from being loaded into EQ, which holds the
# permission to record system audio. Under it, recording audio needs the entitlement in
# bundle/EQ.entitlements. Notarization requires the runtime and a secure timestamp.
set -- --force --sign "$identity"
if [ "${EQ_HARDENED_RUNTIME:-1}" != 0 ]; then
    set -- "$@" --options runtime --entitlements bundle/EQ.entitlements
fi
if [ "$identity" != - ]; then
    set -- "$@" --timestamp
fi
codesign "$@" "$app"
codesign --verify --strict "$app"
echo "Built $app (signed by $identity)"

if [ "$zip" = --zip ]; then
    rm -f target/EQ.zip target/EQ.zip.sha256
    ditto -c -k --keepParent "$app" target/EQ.zip
    if [ -n "${EQ_NOTARY_PROFILE:-}" ]; then
        set -- --keychain-profile "$EQ_NOTARY_PROFILE"
        if [ -n "${EQ_NOTARY_KEYCHAIN:-}" ]; then
            set -- "$@" --keychain "$EQ_NOTARY_KEYCHAIN"
        fi
        xcrun notarytool submit target/EQ.zip "$@" --wait
        xcrun stapler staple "$app"
        # The zip must carry the stapled app.
        rm target/EQ.zip
        ditto -c -k --keepParent "$app" target/EQ.zip
    fi
    (cd target && shasum -a 256 EQ.zip > EQ.zip.sha256)
    echo "Built target/EQ.zip and target/EQ.zip.sha256"
fi
