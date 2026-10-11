#!/bin/sh
# Regenerates bundle/AppIcon.icns from scripts/make-icon.swift.
set -eu
cd "$(dirname "$0")/.."

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
set=$work/AppIcon.iconset
mkdir "$set"

swift scripts/make-icon.swift "$work/icon.png"
for size in 16 32 128 256 512; do
    sips -z $size $size "$work/icon.png" --out "$set/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$work/icon.png" --out "$set/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set" -o bundle/AppIcon.icns
cp "$work/icon.png" bundle/icon.png
echo "Built bundle/AppIcon.icns"
