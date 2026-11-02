#!/usr/bin/env bash
# Pack Lightforge.app into a UDZO disk image with an Applications shortcut.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
app="${1:-$root/target/dist/Lightforge.app}"
out="${2:-$root/target/dist/Lightforge-$version-arm64.dmg}"

if [[ ! -d "$app" ]]; then
  echo "missing $app — run packaging/macos/bundle.sh first" >&2
  exit 1
fi

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/Lightforge.app"
ln -s /Applications "$stage/Applications"

mkdir -p "$(dirname "$out")"
rm -f "$out"
hdiutil create \
  -volname "Lightforge" \
  -srcfolder "$stage" \
  -ov \
  -format UDZO \
  "$out"

echo "ok   $out"
