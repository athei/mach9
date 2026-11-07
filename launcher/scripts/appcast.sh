#!/usr/bin/env bash
#
# Build the appcast of a release, with the deltas into it, on top of the
# appcast of the latest published release.
#
# Usage:
#   launcher/scripts/appcast.sh --preflight <version>
#   launcher/scripts/appcast.sh <version> <dir>
#
# <version> is the release version, X.Y.Z or X.Y.Z-rc.N (lib.sh). Both forms
# need SPARKLE_ED_KEY in the environment: Sparkle's private EdDSA key, as
# `generate_keys -x` exports it. It only ever reaches the Sparkle tools on
# their stdin and is never written to a file or printed.
#
# --preflight is the quick check before anything is built. It refuses when
#   - the version is not a release version;
#   - the workspace version in Cargo.toml is not X.Y.Z;
#   - release-notes/X.Y.Z.md is missing or empty;
#   - the public half of SPARKLE_ED_KEY is not SUPublicEDKey in
#     launcher/app/Info.plist, which every copy in the field checks updates
#     against;
#   - the current appcast lists this version or a higher one.
#
# Without it, <dir> holds the signed, notarized DMG of this release,
# <app>-<version>.dmg, and nothing else. The script then
#   1. runs every check of --preflight;
#   2. fetches the current appcast and downloads the DMGs of the newest 20
#      releases it lists, the sources of the deltas, each from its release,
#      v<short version>, with `gh release download`. generate_appcast keeps
#      the item of a release it has no DMG for as it is, and with 21
#      releases kept the one left is dropped anyway;
#   3. writes the release notes of each listed release, its <description>,
#      next to its DMG, so they survive (generate_appcast drops the notes of
#      a release it has no file for), and release-notes/X.Y.Z.md next to the
#      new DMG;
#   4. runs generate_appcast, which signs every archive, makes the deltas
#      from up to 20 earlier releases and keeps 21 releases;
#   5. points every enclosure and delta at its release on GitHub,
#      .../releases/download/v<short version>/<file>; deltas belong to the
#      release they update to. The EdDSA signatures cover the files, not the
#      URLs, so this changes no signature;
#   6. checks that every enclosure has a signature, which has to verify
#      with the key where the file is here, and a URL of that form, and
#      warns when the new release has fewer deltas than there are earlier
#      releases to make them from (those copies update with the whole DMG);
#   7. leaves appcast.xml and the new release's deltas in <dir>, next to its
#      DMG: the files to attach to the release.
# The earlier DMGs go into a work directory next to <dir>, which is deleted
# at the end, as is the cache generate_appcast unpacks every DMG into
# (~/Library/Caches/Sparkle_generate_appcast) when it did not exist before.
# `df -h` of the work directory is printed before and after.
#
# Environment, for tests:
#   APPCAST_URL       The current appcast, a URL or a file. Default: the one
#                     of the latest release of APPCAST_REPO.
#   APPCAST_ARCHIVES  A directory to take the earlier DMGs from instead of
#                     downloading them.
#   APPCAST_REPO      The GitHub repository. Default: athei/mach9
#   APPCAST_INFO_PLIST
#                     The Info.plist with the SUPublicEDKey to check the key
#                     against. Default: launcher/app/Info.plist
#   APPCAST_MAXIMUM_DELTAS, APPCAST_MAXIMUM_VERSIONS
#                     The 20 and 21 above, to test with fewer releases.
#
# Needs the Xcode command line tools (swiftc), gh with a token for
# downloads, and the Sparkle tools from fetch-sparkle.sh.

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

APPCAST_REPO="${APPCAST_REPO:-athei/mach9}"
APPCAST_URL="${APPCAST_URL:-https://github.com/$APPCAST_REPO/releases/latest/download/appcast.xml}"
APPCAST_INFO_PLIST="${APPCAST_INFO_PLIST:-$INFO_SRC}"
DOWNLOAD_PREFIX="https://github.com/$APPCAST_REPO/releases/download"
MAXIMUM_DELTAS="${APPCAST_MAXIMUM_DELTAS:-20}"
MAXIMUM_VERSIONS="${APPCAST_MAXIMUM_VERSIONS:-21}"
# A release that has left the appcast is no delta source any more, so the
# appcast has to keep more releases than there are deltas.
if ! [[ "$MAXIMUM_DELTAS" =~ ^[0-9]+$ && "$MAXIMUM_VERSIONS" =~ ^[0-9]+$ ]] \
    || (( MAXIMUM_VERSIONS <= MAXIMUM_DELTAS )); then
  die "the appcast has to keep more releases ($MAXIMUM_VERSIONS) than there are deltas ($MAXIMUM_DELTAS)"
fi

PREFLIGHT=0
case "${1:-}" in
  --preflight) PREFLIGHT=1; shift; [[ $# -eq 1 ]] || usage ;;
  -h|--help) usage 0 ;;
  -*) die "unknown option: $1" ;;
  *) [[ $# -eq 2 ]] || usage ;;
esac
VERSION="$1"
OUT="${2:-}"

require_macos
check_version "$VERSION"
BASE="$(base_version "$VERSION")"
BUILD="$(bundle_version "$VERSION")"
[[ -f "$APPCAST_INFO_PLIST" ]] || die "no Info.plist at $APPCAST_INFO_PLIST"
APP_NAME="$($PLIST -c 'Print :CFBundleName' "$INFO_SRC")"
NOTES="$REPO_ROOT/release-notes/$BASE.md"

TMP_DIR="$(mktemp -d -t appcast)"
WORK=""
# Where generate_appcast unpacks every DMG it reads, about the size of the
# app per release, and keeps it after it exits. It is only a cache, but every
# generate_appcast run of this user shares it, so it is deleted only when
# this run created it: one that runs at the same time may be using it.
SPARKLE_CACHE="$HOME/Library/Caches/Sparkle_generate_appcast"
SPARKLE_CACHE_OURS=1
if [[ -e "$SPARKLE_CACHE" ]]; then SPARKLE_CACHE_OURS=0; fi
cleanup() {
  rm -rf "$TMP_DIR"
  if [[ -n "$WORK" ]]; then
    rm -rf "$WORK"
    if [[ $SPARKLE_CACHE_OURS -eq 1 ]]; then rm -rf "$SPARKLE_CACHE"; fi
  fi
}
trap cleanup EXIT

# --- checks that need nothing built --------------------------------------------

workspace="$(workspace_version)"
[[ "$workspace" == "$BASE" ]] \
  || die "Cargo.toml has the workspace version $workspace, this release is $VERSION; commit the version bump to $BASE first"
[[ -f "$NOTES" ]] || die "no release notes at ${NOTES#"$REPO_ROOT/"}"
[[ -n "$(tr -d '[:space:]' < "$NOTES")" ]] || die "the release notes at ${NOTES#"$REPO_ROOT/"} are empty"
[[ -n "${SPARKLE_ED_KEY:-}" ]] || die "SPARKLE_ED_KEY is not set"

echo "==> Building appcast-tool"
TOOL="$TMP_DIR/appcast-tool"
xcrun swiftc -O -o "$TOOL" "$LAUNCHER_DIR/scripts/appcast-tool.swift" \
  || die "could not build appcast-tool.swift"

# Every copy in the field verifies an update with the SUPublicEDKey it was
# shipped with, so a release signed with any other key is one nobody can
# install.
want_key="$($PLIST -c 'Print :SUPublicEDKey' "$APPCAST_INFO_PLIST" 2>/dev/null || true)"
[[ -n "$want_key" ]] || die "$APPCAST_INFO_PLIST has no SUPublicEDKey"
have_key="$(printf '%s\n' "$SPARKLE_ED_KEY" | "$TOOL" public-key)" \
  || die "SPARKLE_ED_KEY is not a key in generate_keys' format"
[[ "$have_key" == "$want_key" ]] \
  || die "the public half of SPARKLE_ED_KEY is $have_key, SUPublicEDKey is $want_key; this is not the key the app ships with"
echo "    SPARKLE_ED_KEY matches SUPublicEDKey $want_key"

# --- the current appcast ---------------------------------------------------------

# Fetch the appcast at APPCAST_URL into $1.
fetch_appcast() {
  echo "==> Fetching $APPCAST_URL"
  case "$APPCAST_URL" in
    https://*)
      curl -fsSL --proto '=https' --retry 3 -o "$1" "$APPCAST_URL" \
        || die "could not fetch $APPCAST_URL; a release needs the appcast of the latest one" ;;
    *) cp "$APPCAST_URL" "$1" ;;
  esac
}

# Exit unless this release is above every one in the appcast at $1. Sets
# LISTED to its items, one "version<TAB>short version<TAB>file" per line.
check_above() {
  local version short file order
  LISTED="$("$TOOL" items "$1")"
  [[ -n "$LISTED" ]] || die "the appcast at $APPCAST_URL lists no release"
  while IFS=$'\t' read -r version short file; do
    echo "    listed: $short ($version), $file"
    [[ "$short" != "$VERSION" ]] || die "the appcast already lists $VERSION"
    order="$(compare_bundle_versions "$BUILD" "$version")"
    [[ "$order" == 1 ]] \
      || die "this release's CFBundleVersion $BUILD is not above $version, which the appcast lists; Sparkle would never offer it"
  done <<<"$LISTED"
}

if [[ $PREFLIGHT -eq 1 ]]; then
  fetch_appcast "$TMP_DIR/appcast.xml"
  check_above "$TMP_DIR/appcast.xml"
  echo "==> Preflight passed for $VERSION (CFBundleVersion $BUILD)"
  exit 0
fi

# --- build -----------------------------------------------------------------------

for tool in generate_appcast sign_update; do
  [[ -x "$SPARKLE_BIN/$tool" ]] || die "no $tool in $SPARKLE_BIN; run fetch-sparkle.sh"
done
[[ -d "$OUT" ]] || die "not a directory: $OUT"
OUT="$(cd "$OUT" && pwd)"
DMG_NAME="$APP_NAME-$VERSION.dmg"
[[ -f "$OUT/$DMG_NAME" ]] || die "no $DMG_NAME in $OUT"
others="$(find "$OUT" -mindepth 1 -maxdepth 1 ! -name "$DMG_NAME" | head -5)"
[[ -z "$others" ]] || die "$OUT holds more than $DMG_NAME:"$'\n'"$others"

# The earlier releases' DMGs only feed generate_appcast, so they stay out of
# <dir>. Next to it, on the same volume, so the new DMG can be linked in.
WORK="$(mktemp -d "$(dirname "$OUT")/appcast-work.XXXXXX")"
echo "==> Disk space before"
df -h "$WORK"

fetch_appcast "$WORK/appcast.xml"
check_above "$WORK/appcast.xml"

# The number of releases in LISTED above the CFBundleVersion $1.
newer_listed() {
  local other rest n=0
  while IFS=$'\t' read -r other rest; do
    if [[ "$(compare_bundle_versions "$other" "$1")" == 1 ]]; then n=$((n + 1)); fi
  done <<<"$LISTED"
  echo "$n"
}

# Only the delta sources: a DMG generate_appcast diffs. The item of an older
# release has no use for its DMG and stays as it is.
echo "==> Getting the DMGs of the newest $MAXIMUM_DELTAS listed releases"
while IFS=$'\t' read -r version short file; do
  if (( $(newer_listed "$version") >= MAXIMUM_DELTAS )); then
    echo "    $file not needed, its item stays as it is"
    continue
  fi
  if [[ -n "${APPCAST_ARCHIVES:-}" ]]; then
    cp "$APPCAST_ARCHIVES/$file" "$WORK/$file" || die "no $file in $APPCAST_ARCHIVES"
  else
    gh release download "v$short" --repo "$APPCAST_REPO" --pattern "$file" --dir "$WORK" \
      || die "could not download $file from the release v$short of $APPCAST_REPO"
  fi
  [[ -f "$WORK/$file" ]] || die "release v$short has no asset $file"
  echo "    $file"
done <<<"$LISTED"

echo "==> Release notes"
"$TOOL" notes "$WORK/appcast.xml" "$WORK" | sed 's/^/    kept: /'
cp "$NOTES" "$WORK/$APP_NAME-$VERSION.md"
echo "    new:  ${NOTES#"$REPO_ROOT/"}"

ln "$OUT/$DMG_NAME" "$WORK/$DMG_NAME" 2>/dev/null || cp "$OUT/$DMG_NAME" "$WORK/$DMG_NAME"

echo "==> Generating the appcast and deltas"
printf '%s\n' "$SPARKLE_ED_KEY" | "$SPARKLE_BIN/generate_appcast" \
  --ed-key-file - \
  --embed-release-notes \
  --maximum-deltas "$MAXIMUM_DELTAS" \
  --maximum-versions "$MAXIMUM_VERSIONS" \
  "$WORK"

# A signed appcast would carry a signature over the URLs rewritten below.
if grep -q 'sparkle-signatures' "$WORK/appcast.xml"; then
  die "generate_appcast signed the appcast itself, and rewriting its URLs would break that signature"
fi

echo "==> Pointing the enclosures at $DOWNLOAD_PREFIX"
"$TOOL" rewrite "$WORK/appcast.xml" "$DOWNLOAD_PREFIX"

echo "==> Checking the enclosures"
NEW_FILES=()
new_items=0
deltas=0
while IFS=$'\t' read -r short kind file signature url; do
  [[ "$url" == "$DOWNLOAD_PREFIX/v$short/$file" ]] || die "$file has the URL $url"
  [[ -n "$signature" ]] \
    || die "$file in the appcast has no EdDSA signature; does the app in it carry SUPublicEDKey $want_key?"
  # No delta of an earlier release is ever downloaded, so one here was made
  # in this run. generate_appcast makes deltas into the newest release of
  # each minimum macOS version; this one would be attached to nothing, and
  # the appcast would point at v$short/$file, which does not exist.
  if [[ "$kind" == delta && "$short" != "$VERSION" && -f "$WORK/$file" ]]; then
    die "generate_appcast made $file, a delta into the earlier release $short, which would never be published. That happens when LSMinimumSystemVersion changes: the appcast then keeps the newest release of each minimum macOS version. Releasing across a floor change needs those deltas attached to their releases, which this script does not do."
  fi
  if [[ -f "$WORK/$file" ]]; then
    printf '%s\n' "$SPARKLE_ED_KEY" \
      | "$SPARKLE_BIN/sign_update" --verify --ed-key-file - "$WORK/$file" "$signature" >/dev/null \
      || die "the signature of $file does not verify"
    verified="verified"
  else
    # A delta of an earlier release, or the DMG of a release older than the
    # delta sources: it stays in that release, and so does its signature.
    verified="not here, kept"
  fi
  echo "    $kind $file ($verified)"
  if [[ "$short" == "$VERSION" ]]; then
    NEW_FILES+=("$file")
    if [[ "$kind" == full ]]; then new_items=$((new_items + 1)); else deltas=$((deltas + 1)); fi
  fi
done < <("$TOOL" enclosures "$WORK/appcast.xml")
[[ $new_items -eq 1 ]] || die "the appcast has $new_items items for $VERSION"

listed="$(grep -c . <<<"$LISTED")"
expected=$(( listed < MAXIMUM_DELTAS ? listed : MAXIMUM_DELTAS ))
if (( deltas < expected )); then
  echo "warning: $deltas delta(s) for $listed earlier release(s); the others update with the whole DMG" >&2
fi

for file in "${NEW_FILES[@]}"; do
  if [[ "$file" != "$DMG_NAME" ]]; then mv "$WORK/$file" "$OUT/$file"; fi
done
mv "$WORK/appcast.xml" "$OUT/appcast.xml"

echo "==> Disk space after"
df -h "$WORK"

echo "==> Done: $OUT"
ls -l "$OUT" | sed 's/^/    /'
