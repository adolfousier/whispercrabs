#!/usr/bin/env bash
# Wrap a built whispercrabs binary in a macOS .app bundle.
#
# A bare Mach-O has no icon slot on macOS: Finder and the Dock read the icon
# from the bundle (CFBundleIconFile), and for a loose binary they fall back to
# the generic "exec" placeholder. The runtime code in src/app_icon.rs covers
# the Dock while the process runs; the bundle is what gives the app a real
# icon everywhere else.
#
# Usage: scripts/bundle-macos.sh [binary] [output-dir]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${1:-$root/target/release/whispercrabs}"
outdir="${2:-$root/target/bundle}"

if [ ! -f "$binary" ]; then
    echo "binary not found: $binary" >&2
    echo "run 'cargo build --release' first" >&2
    exit 1
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
app="$outdir/WhisperCrabs.app"
contents="$app/Contents"

rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Resources"

install -m 755 "$binary" "$contents/MacOS/whispercrabs"
install -m 644 "$root/assets/icon.icns" "$contents/Resources/icon.icns"

cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>whispercrabs</string>
    <key>CFBundleIconFile</key>
    <string>icon.icns</string>
    <key>CFBundleIdentifier</key>
    <string>dev.whispercrabs.app</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>WhisperCrabs</string>
    <key>CFBundleDisplayName</key>
    <string>WhisperCrabs</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${version}</string>
    <key>CFBundleVersion</key>
    <string>${version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
PLIST

echo "$app"
