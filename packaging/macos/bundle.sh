#!/usr/bin/env bash
# Assemble Lightforge.app from release binaries. Run on macOS after
# `cargo build --release`.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
# Tagged macOS releases are `v<upstream>-mac.N`. Cargo.toml stays on the
# upstream SemVer; the tag is what the update check compares.
if [[ -n "${LIGHTFORGE_VERSION:-}" ]]; then
  version="$LIGHTFORGE_VERSION"
else
  version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
fi
bin_dir="${1:-$root/target/release}"
out_dir="${2:-$root/target/dist}"
app="$out_dir/Lightforge.app"

if [[ ! -x "$bin_dir/lightforge-gui" ]]; then
  echo "missing $bin_dir/lightforge-gui — build the GUI first" >&2
  exit 1
fi

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

sed "s/VERSION/$version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"

cp "$bin_dir/lightforge-gui" "$app/Contents/MacOS/lightforge-gui"
chmod 755 "$app/Contents/MacOS/lightforge-gui"
if [[ -x "$bin_dir/lightforge" ]]; then
  cp "$bin_dir/lightforge" "$app/Contents/MacOS/lightforge"
  chmod 755 "$app/Contents/MacOS/lightforge"
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
# iconutil refuses a folder whose name does not end in `.iconset`.
iconset="$scratch/Lightforge.iconset"
mkdir -p "$iconset"
icons="$root/packaging/icons/hicolor"
cp "$icons/16x16/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_16x16.png"
cp "$icons/32x32/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_32x32.png"
cp "$icons/32x32/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_16x16@2x.png"
cp "$icons/64x64/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_32x32@2x.png"
cp "$icons/128x128/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_128x128.png"
cp "$icons/256x256/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_128x128@2x.png"
cp "$icons/256x256/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_256x256.png"
cp "$icons/512x512/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_256x256@2x.png"
cp "$icons/512x512/apps/pro.tuxcore.Lightforge.png" "$iconset/icon_512x512.png"
if command -v sips >/dev/null 2>&1; then
  sips -z 1024 1024 "$icons/512x512/apps/pro.tuxcore.Lightforge.png" --out "$iconset/icon_512x512@2x.png" >/dev/null
fi
if command -v iconutil >/dev/null 2>&1; then
  iconutil --convert icns --output "$app/Contents/Resources/Lightforge.icns" "$iconset"
else
  # CI without iconutil: keep the 512px PNG; macOS prefers icns but the
  # bundle still launches.
  cp "$icons/512x512/apps/pro.tuxcore.Lightforge.png" "$app/Contents/Resources/Lightforge.png"
fi

echo "ok   $app"
