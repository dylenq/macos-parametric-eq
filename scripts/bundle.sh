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
mkdir -p "$app/Contents/MacOS"
cp bundle/Info.plist "$app/Contents/Info.plist"
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
