#!/bin/sh
# Builds a universal release binary and wraps it in target/EQ.app. macOS only grants system
# audio capture to a bundle with NSAudioCaptureUsageDescription, so run the app from there.
# Pass --zip to also write target/EQ.zip for sharing.
set -eu
cd "$(dirname "$0")/.."

export MACOSX_DEPLOYMENT_TARGET=14.2
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
codesign --force --sign - "$app"
echo "Built $app"

if [ "${1:-}" = "--zip" ]; then
    rm -f target/EQ.zip
    ditto -c -k --keepParent "$app" target/EQ.zip
    echo "Built target/EQ.zip"
fi
