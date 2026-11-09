#!/usr/bin/env bash
#
# Fetch the Sparkle release pins/redist.env pins (SPARKLE_VERSION and
# SPARKLE_SHA256), which the launcher links against and the bundle ships.
#
# Usage:
#   launcher/scripts/fetch-sparkle.sh [--force] [--dest <dir>]
#
# The framework is a binary dependency, so it is not committed. This script
# downloads exactly that release, refuses to continue unless its SHA-256
# matches, and lays the pieces out in SPARKLE_DIR (target/launcher/sparkle
# unless the environment or --dest names another directory):
#
#   Sparkle.framework   linked at build time, copied into Contents/Frameworks
#                       when the bundle is assembled
#   bin/                generate_keys, generate_appcast, sign_update,
#                       BinaryDelta
#
# The dSYMs Sparkle ships alongside the framework are dropped: codesign-bundle.sh
# would have to strip them out of the bundle again before notarization.
#
# Re-running is a no-op once the pinned version is in place, apart from
# touching .version in that directory so make sees it as newer than this
# script and redist.env; --force refetches. The stamp is written last, after
# the framework verifies, so an interrupted or failed fetch is retried on the
# next run.
#
# Only stock macOS tools are used (curl, shasum, tar).

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
# SPARKLE_VERSION and SPARKLE_SHA256.
# shellcheck source=../../pins/redist.env
source "$REPO_ROOT/pins/redist.env"
[[ -n "${SPARKLE_VERSION:-}" && -n "${SPARKLE_SHA256:-}" ]] \
  || die "$REPO_ROOT/pins/redist.env pins no SPARKLE_VERSION or SPARKLE_SHA256"

DEST="$SPARKLE_DIR"
FORCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    --dest) DEST="${2:?--dest needs a path}"; shift 2 ;;
    -h|--help) usage 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

require_macos

STAMP="$DEST/.version"
if [[ $FORCE -eq 0 && -f "$STAMP" && "$(cat "$STAMP")" == "$SPARKLE_VERSION" \
      && -d "$DEST/Sparkle.framework" && -x "$DEST/bin/generate_appcast" ]]; then
  # The Makefile rule that runs this script has the stamp as its target and
  # the script and redist.env as its prerequisites, so after an edit to
  # either that leaves the version alone, the stamp has to move or make runs
  # this on every build.
  touch "$STAMP"
  echo "Sparkle $SPARKLE_VERSION already in $DEST"
  exit 0
fi

URL="https://github.com/sparkle-project/Sparkle/releases/download/$SPARKLE_VERSION/Sparkle-$SPARKLE_VERSION.tar.xz"
TMP_DIR="$(mktemp -d -t fetch-sparkle)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "==> Downloading Sparkle $SPARKLE_VERSION"
curl -fsSL --retry 3 --proto '=https' --tlsv1.2 -o "$TMP_DIR/sparkle.tar.xz" "$URL" \
  || die "download failed: $URL"

echo "==> Verifying checksum"
GOT="$(shasum -a 256 "$TMP_DIR/sparkle.tar.xz" | cut -d' ' -f1)"
[[ "$GOT" == "$SPARKLE_SHA256" ]] \
  || die "checksum mismatch for Sparkle $SPARKLE_VERSION"$'\n'"  expected $SPARKLE_SHA256"$'\n'"  got      $GOT"

echo "==> Extracting"
tar -xJf "$TMP_DIR/sparkle.tar.xz" -C "$TMP_DIR"
[[ -d "$TMP_DIR/Sparkle.framework" ]] || die "no Sparkle.framework in the tarball"

rm -rf "$DEST"
mkdir -p "$DEST/bin"
# ditto, not cp: the framework is a signed bundle full of symlinks, and
# following them instead of copying them breaks the signature.
ditto "$TMP_DIR/Sparkle.framework" "$DEST/Sparkle.framework"
for tool in generate_keys generate_appcast sign_update BinaryDelta; do
  [[ -f "$TMP_DIR/bin/$tool" ]] || die "no $tool in the tarball"
  ditto "$TMP_DIR/bin/$tool" "$DEST/bin/$tool"
done
codesign --verify --strict "$DEST/Sparkle.framework" \
  || die "the extracted framework does not verify"
printf '%s\n' "$SPARKLE_VERSION" > "$STAMP"

echo "==> Done: $DEST (Sparkle $SPARKLE_VERSION)"
