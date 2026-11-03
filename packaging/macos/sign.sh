#!/usr/bin/env bash
# Sign (and optionally notarize) Lightforge.app / the DMG.
#
#   packaging/macos/sign.sh sign      <app>
#   packaging/macos/sign.sh notarize  <app> <dmg>
#   packaging/macos/sign.sh cleanup
#
# Developer ID secrets, when present:
#   MACOS_CERT_P12          base64-encoded .p12
#   MACOS_CERT_PASSWORD     that archive's password
#   MACOS_SIGN_IDENTITY     "Developer ID Application: … (TEAMID)"
#   APPLE_API_KEY_P8        App Store Connect .p8 body
#   APPLE_API_KEY_ID
#   APPLE_API_ISSUER_ID
#
# Missing cert secrets → ad-hoc sign, notice, exit 0.
# Identity without notary secrets → signed, not notarized, notice, exit 0.
# Never prints secret values. The keychain lives under $RUNNER_TEMP (or
# /tmp) and is deleted by `cleanup`, which the workflow runs with
# `if: always()`.
set -euo pipefail

# Tracing would dump env and file contents. Leave it off.
set +x

root="$(cd "$(dirname "$0")/../.." && pwd)"
entitlements="$root/packaging/macos/Lightforge.entitlements"
state_dir="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
state_dir="${state_dir%/}"
keychain_file="$state_dir/lightforge-signing.keychain-db"
keychain_pw_file="$state_dir/lightforge-signing.pw"
p12_file="$state_dir/lightforge-signing.p12"
api_key_file=""

have_identity() {
  [[ -n "${MACOS_CERT_P12:-}" && -n "${MACOS_CERT_PASSWORD:-}" && -n "${MACOS_SIGN_IDENTITY:-}" ]]
}

have_notary() {
  [[ -n "${APPLE_API_KEY_P8:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]
}

notice() {
  echo "notice: $*"
}

write_output() {
  local name="$1"
  local value="$2"
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    printf '%s=%s\n' "$name" "$value" >>"$GITHUB_OUTPUT"
  fi
}

cleanup() {
  if [[ -n "${APPLE_API_KEY_ID:-}" ]]; then
    rm -f "$state_dir/AuthKey_${APPLE_API_KEY_ID}.p8"
  fi
  rm -f "$state_dir"/AuthKey_*.p8 "$p12_file" "$keychain_pw_file"
  if [[ -f "$keychain_file" ]]; then
    security delete-keychain "$keychain_file" >/dev/null 2>&1 || true
    rm -f "$keychain_file"
    echo "ok   deleted signing keychain"
  fi
}

prepare_keychain() {
  if [[ -f "$keychain_file" ]]; then
    return 0
  fi
  local password
  password="$(openssl rand -base64 32)"
  umask 077
  printf '%s' "$password" >"$keychain_pw_file"
  security create-keychain -p "$password" "$keychain_file"
  security set-keychain-settings -lut 21600 "$keychain_file"
  security unlock-keychain -p "$password" "$keychain_file"

  printf '%s' "$MACOS_CERT_P12" | openssl base64 -d -A -out "$p12_file"
  security import "$p12_file" \
    -k "$keychain_file" \
    -P "$MACOS_CERT_PASSWORD" \
    -T /usr/bin/codesign \
    -T /usr/bin/security \
    -T /usr/bin/productsign
  rm -f "$p12_file"
  security set-key-partition-list \
    -S apple-tool:,apple: \
    -s \
    -k "$password" \
    "$keychain_file" >/dev/null
  # Keep the login keychain on the search list so other steps still work.
  local existing
  existing="$(security list-keychains -d user | tr -d '"')"
  # shellcheck disable=SC2086
  security list-keychains -d user -s "$keychain_file" $existing
}

sign_one() {
  local path="$1"
  local extra=()
  # Entitlements belong on the app, not on every nested dylib.
  if [[ "$path" == *.app ]]; then
    extra+=(--entitlements "$entitlements")
  fi
  codesign \
    --force \
    --options runtime \
    --timestamp \
    ${extra[@]+"${extra[@]}"} \
    --sign "$MACOS_SIGN_IDENTITY" \
    --keychain "$keychain_file" \
    "$path"
}

# Deepest nested code first, then the bundle itself. `--deep` is not used:
# it treats data files next to a Mach-O as unsigned nested code.
sign_bundle() {
  local app="$1"
  local path
  while IFS= read -r path; do
    sign_one "$path"
  done < <(
    find "$app" \( -name '*.framework' -o -name '*.dylib' -o -name '*.so' \) -print \
      | awk '{ print length, $0 }' | sort -nr | cut -d' ' -f2-
  )
  while IFS= read -r path; do
    [[ "$path" == "$app" ]] && continue
    sign_one "$path"
  done < <(
    find "$app" -name '*.app' -print \
      | awk '{ print length, $0 }' | sort -nr | cut -d' ' -f2-
  )
  if [[ -d "$app/Contents/MacOS" ]]; then
    for path in "$app/Contents/MacOS"/*; do
      [[ -f "$path" ]] || continue
      sign_one "$path"
    done
  fi
  sign_one "$app"
  codesign --verify --verbose=2 "$app"
}

sign_adhoc() {
  local app="$1"
  codesign --force --deep --sign - "$app"
  codesign --verify --verbose "$app"
  notice "signed $app ad-hoc (Developer ID secrets are not set)"
  write_output method adhoc
  write_output notarized no
}

cmd_sign() {
  local app="$1"
  if [[ ! -d "$app" ]]; then
    echo "missing $app" >&2
    exit 1
  fi
  if ! have_identity; then
    sign_adhoc "$app"
    return 0
  fi
  prepare_keychain
  sign_bundle "$app"
  echo "ok   signed $app as Developer ID (identity not printed)"
  write_output method identity
}

write_api_key() {
  api_key_file="$state_dir/AuthKey_${APPLE_API_KEY_ID}.p8"
  umask 077
  # The secret is the file body. A missing trailing newline is fine.
  printf '%s' "$APPLE_API_KEY_P8" >"$api_key_file"
  if [[ -s "$api_key_file" ]] && [[ "$(tail -c1 "$api_key_file" | wc -c)" -ne 0 ]]; then
    printf '\n' >>"$api_key_file"
  fi
}

cmd_notarize() {
  local app="$1"
  local dmg="$2"
  if [[ ! -d "$app" || ! -f "$dmg" ]]; then
    echo "missing $app or $dmg" >&2
    exit 1
  fi
  if ! have_identity; then
    notice "skipping notarization: Developer ID secrets are not set"
    write_output notarized no
    return 0
  fi
  if ! have_notary; then
    notice "skipping notarization: App Store Connect API secrets are not set"
    write_output notarized no
    return 0
  fi
  prepare_keychain
  codesign --force --sign "$MACOS_SIGN_IDENTITY" --timestamp --keychain "$keychain_file" "$dmg"
  write_api_key
  xcrun notarytool submit "$dmg" \
    --key "$api_key_file" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" \
    --wait
  rm -f "$api_key_file"
  xcrun stapler staple "$app"
  xcrun stapler staple "$dmg"
  xcrun stapler validate "$app"
  xcrun stapler validate "$dmg"
  spctl --assess --type execute --verbose=2 "$app"
  echo "ok   notarized and stapled $dmg"
  write_output notarized yes
}

usage() {
  echo "usage: $0 sign <app> | notarize <app> <dmg> | cleanup" >&2
  exit 2
}

cmd="${1:-}"
shift || true
case "$cmd" in
  sign)
    [[ $# -eq 1 ]] || usage
    cmd_sign "$1"
    ;;
  notarize)
    [[ $# -eq 2 ]] || usage
    cmd_notarize "$1" "$2"
    ;;
  cleanup)
    cleanup
    ;;
  *)
    usage
    ;;
esac
