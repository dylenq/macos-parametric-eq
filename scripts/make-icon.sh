#!/bin/sh
# Regenerates the app icon from scripts/make-icon.swift: bundle/AppIcon.icns for macOS 14
# and 15, and bundle/AppIcon.icon, the layered Icon Composer icon that macOS 26 and later
# use in every icon style. scripts/bundle.sh compiles the latter into the app.
set -eu
cd "$(dirname "$0")/.."

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
set=$work/AppIcon.iconset
mkdir "$set"

assets=bundle/AppIcon.icon/Assets
rm -rf "$assets"
mkdir -p "$assets"
swift scripts/make-icon.swift "$work/icon.png" "$assets"
for size in 16 32 128 256 512; do
    sips -z $size $size "$work/icon.png" --out "$set/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$work/icon.png" --out "$set/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set" -o bundle/AppIcon.icns
cp "$work/icon.png" bundle/icon.png
echo "Built bundle/AppIcon.icns and bundle/AppIcon.icon"
