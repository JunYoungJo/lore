#!/bin/bash
set -euo pipefail

bundle_dir="${1:?Usage: verify-macos-bundles.sh <bundle-directory>}"

verify_app() {
  codesign --verify --deep --strict --verbose=2 "$1"
  codesign --verify --strict --verbose=2 "$1/Contents/MacOS/lore-cli"
}

verify_app "$bundle_dir/macos/Lore.app"

archives=("$bundle_dir"/macos/*.app.tar.gz)
images=("$bundle_dir"/dmg/*.dmg)
if [[ ${#archives[@]} -ne 1 || ! -f "${archives[0]}" || ${#images[@]} -ne 1 || ! -f "${images[0]}" ]]; then
  echo 'Expected exactly one macOS updater archive and one DMG.' >&2
  exit 1
fi

temporary="$(mktemp -d)"
mounted=false
cleanup() {
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$temporary/dmg" || return 1
  fi
  rm -rf "$temporary"
}
trap cleanup EXIT

mkdir "$temporary/updater"
tar -xzf "${archives[0]}" -C "$temporary/updater"
verify_app "$temporary/updater/Lore.app"

hdiutil attach -readonly -nobrowse -mountpoint "$temporary/dmg" "${images[0]}"
mounted=true
verify_app "$temporary/dmg/Lore.app"
echo 'Verified macOS app and CLI signatures in all distribution bundles.'
