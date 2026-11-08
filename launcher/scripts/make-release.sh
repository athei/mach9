#!/usr/bin/env bash
#
# Turn an assembled Mach9.app into a release: a signed, notarized bundle, the
# DMG that ships it, and the appcast with the deltas into it.
#
# Usage:
#   launcher/scripts/make-release.sh [options] <path/to/Mach9.app>
#
# The bundle comes from `make -C launcher app VERSION=<version>`, which this
# script never runs. Before anything is signed, it refuses when
#
#   - launcher/, compatdb/core, Cargo.toml, Cargo.lock, .cargo/ or
#     release-notes/ has uncommitted changes or untracked files, since a
#     release has to match a commit;
#   - the bundle's CFBundleShortVersionString is not the version, its
#     CFBundleVersion is not that version's (version.sh --bundle), or its
#     CFBundleIdentifier, CFBundleName, SUFeedURL or SUPublicEDKey differ
#     from launcher/app/Info.plist, which means it was not assembled for
#     this release;
#   - the bundle has no Sparkle.framework, or its launcher does not link
#     Sparkle through @rpath/Sparkle.framework with the rpath
#     @executable_path/../Frameworks, which means it was built without
#     SPARKLE_DIR and would ship with no updater;
#   - the minimum macOS version (minos) the launcher or the Dock tile plug-in
#     was built for differs from LSMinimumSystemVersion in its Info.plist,
#     which means it was built without .cargo/config.toml;
#   - `appcast.sh --preflight` refuses: the version, the workspace version,
#     the release notes, SPARKLE_ED_KEY and the current appcast.
#
# Then it signs the bundle (codesign-bundle.sh sign), notarizes and staples
# it (codesign-bundle.sh notarize), packs <out>/Mach9.dmg around the stapled
# bundle (make-dmg.sh) and adds the appcast and the deltas (appcast.sh).
# <out> then holds exactly the files to attach to the release. Every release
# has its DMG under that one name, which the download link asks
# releases/latest/download for.
#
# Options:
#   --version <version>   The release version, X.Y.Z or X.Y.Z-rc.N.
#                         Default: the workspace version.
#   --out <dir>           Where the release files go. It has to be empty or
#                         not exist yet. Default: target/launcher/release
#   --profile <name>      notarytool keychain profile, passed to
#                         `codesign-bundle.sh notarize`. Without it,
#                         NOTARY_KEY_PATH, NOTARY_KEY_ID and NOTARY_ISSUER
#                         name an API key, or the profile is `default`.
#   --skip-notarize       Rehearse a release: sign, but do not notarize and
#                         build no appcast, so <out> holds only a DMG that is
#                         never to be published. The checks above still
#                         run, the preflight with SPARKLE_ED_KEY included;
#                         run appcast.sh on its own to rehearse that step.
#   --adhoc               Sign ad hoc instead of with the Developer ID
#                         identity, so no signing identity or notarization
#                         credentials are needed. Implies --skip-notarize.
#   --force               Passed to codesign-bundle.sh: sign and notarize
#                         even a bundle that already carries a valid stapled
#                         ticket.
#
# The environment carries the secrets: SPARKLE_ED_KEY for appcast.sh, the
# NOTARY_* variables for codesign-bundle.sh, and a token for gh. It runs
# without a terminal, as in CI.

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

VERSION=""
OUT="$BUILD_DIR/release"
PROFILE=""
NOTARIZE=1
ADHOC=0
FORCE=0
APP=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
    --out) OUT="${2:?--out needs a path}"; shift 2 ;;
    --profile) PROFILE="${2:?--profile needs a keychain profile name}"; shift 2 ;;
    --skip-notarize) NOTARIZE=0; shift ;;
    --adhoc) ADHOC=1; NOTARIZE=0; shift ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage 0 ;;
    -*) die "unknown option: $1" ;;
    *)
      [[ -z "$APP" ]] || die "unexpected extra argument: $1"
      APP="$1"; shift ;;
  esac
done

require_macos
[[ -n "$APP" ]] || usage
APP="${APP%/}"
[[ -d "$APP" ]] || die "not an app bundle: $APP"
[[ -f "$APP/Contents/Info.plist" ]] || die "missing Contents/Info.plist in: $APP"
command -v otool >/dev/null && command -v vtool >/dev/null \
  || die "otool and vtool are missing; install the Xcode command line tools"
VERSION="${VERSION:-$(workspace_version)}"
check_version "$VERSION"

# Every check below runs before anything is signed: everything after them is a
# signing pass and a notarization round trip, and finding a mistake afterwards
# would mean doing all of it again.

# --- the commit -----------------------------------------------------------------

SOURCE=(launcher compatdb/core Cargo.toml Cargo.lock .cargo release-notes)
changed="$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all -- "${SOURCE[@]}")"
[[ -z "$changed" ]] \
  || die "the source of the app has changes that are not committed:"$'\n'"$changed"
COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"

# --- the bundle -----------------------------------------------------------------

# A key missing from either file reads as empty, and an empty value never
# matches a present one, so the comparisons below catch a missing key too.
plist_get() {
  $PLIST -c "Print :$1" "$2" 2>/dev/null || true
}

APP_PLIST="$APP/Contents/Info.plist"
# CFBundleName names the DMG here, and appcast.sh reads it from the source.
for key in CFBundleIdentifier CFBundleName SUFeedURL SUPublicEDKey; do
  want="$(plist_get "$key" "$INFO_SRC")"
  have="$(plist_get "$key" "$APP_PLIST")"
  [[ -n "$want" && "$have" == "$want" ]] \
    || die "the bundle has $key '$have', $INFO_REL has '$want'; assemble it again with 'make -C launcher app'"
done
have="$(plist_get CFBundleShortVersionString "$APP_PLIST")"
[[ "$have" == "$VERSION" ]] \
  || die "the bundle has the version '$have', not $VERSION; assemble it with 'make -C launcher app VERSION=$VERSION'"
have="$(plist_get CFBundleVersion "$APP_PLIST")"
[[ "$have" == "$(bundle_version "$VERSION")" ]] \
  || die "the bundle has CFBundleVersion '$have', not $(bundle_version "$VERSION"); assemble it with 'make -C launcher app VERSION=$VERSION'"
[[ -d "$APP/Contents/Frameworks/Sparkle.framework" ]] \
  || die "no Sparkle.framework in the bundle; assemble it with 'make -C launcher app'"

# A dotted version without trailing .0 parts, so 26.4 and 26.4.0 compare equal.
version_trim() {
  printf '%s' "$1" | sed -e 's/\(\.0\)*$//'
}

# Exit unless the main executable of the bundle at $1 was built for the
# LSMinimumSystemVersion in that bundle's Info.plist. Sets CHECKED_EXE to its
# path.
check_minos() {
  local bundle="$1" exe want got
  exe="$(plist_get CFBundleExecutable "$bundle/Contents/Info.plist")"
  [[ -n "$exe" && -f "$bundle/Contents/MacOS/$exe" ]] \
    || die "no executable in ${bundle#"$APP"/}; assemble the bundle with 'make -C launcher app'"
  exe="$bundle/Contents/MacOS/$exe"
  want="$(plist_get LSMinimumSystemVersion "$bundle/Contents/Info.plist")"
  [[ -n "$want" ]] || die "no LSMinimumSystemVersion in $bundle/Contents/Info.plist"
  # Read whole before matching: a reader that stops early would leave the
  # tool to die of SIGPIPE, which pipefail turns into a failure.
  got="$(vtool -show-build "$exe" 2>/dev/null || true)"
  got="$(awk '$1 == "minos" { print $2; exit }' <<<"$got")"
  [[ -n "$got" && "$(version_trim "$got")" == "$(version_trim "$want")" ]] \
    || die "${exe#"$APP"/} was built for macOS ${got:-unknown}, its Info.plist says $want; build it with 'make -C launcher app'"
  CHECKED_EXE="$exe"
}

check_minos "$APP"
LAUNCHER="$CHECKED_EXE"
DOCKTILE="$APP/Contents/PlugIns/GameLauncherDockTile.docktileplugin"
[[ -d "$DOCKTILE" ]] || die "no Dock tile plug-in in the bundle; assemble it with 'make -C launcher app'"
check_minos "$DOCKTILE"
# The tool output is captured whole and matched afterwards, for the same
# reason as in check_minos.
libs="$(otool -L "$LAUNCHER" 2>/dev/null || true)"
grep -q '^[[:space:]]*@rpath/Sparkle\.framework/' <<<"$libs" \
  || die "${LAUNCHER#"$APP"/} does not link Sparkle, so it was built without SPARKLE_DIR and has no updater; build it with 'make -C launcher app'"
load_commands="$(otool -l "$LAUNCHER" 2>/dev/null || true)"
rpaths="$(awk '$1 == "cmd" { rpath = ($2 == "LC_RPATH") } rpath && $1 == "path" { print $2 }' <<<"$load_commands")"
grep -qxF '@executable_path/../Frameworks' <<<"$rpaths" \
  || die "${LAUNCHER#"$APP"/} has no rpath @executable_path/../Frameworks, so it cannot find the bundle's Sparkle; build it with 'make -C launcher app'"

# --- the output directory, the version and the key -------------------------------

if [[ -e "$OUT" ]]; then
  [[ -d "$OUT" && -z "$(ls -A "$OUT")" ]] \
    || die "$OUT is not an empty directory; it would mix this release's files with others"
fi

"$LAUNCHER_DIR/scripts/appcast.sh" --preflight "$VERSION"

# --- release -----------------------------------------------------------------------

APP_NAME="$(plist_get CFBundleName "$APP_PLIST")"
DMG="$OUT/$APP_NAME.dmg"

echo "==> Releasing $VERSION from $COMMIT"

# Both codesign-bundle.sh commands stop on an already-stapled bundle, so a
# re-run after a failure further down costs neither a signing pass nor an
# upload.
FORCE_ARGS=()
if [[ $FORCE -eq 1 ]]; then
  FORCE_ARGS=(--force)
fi
SIGN_ARGS=("${FORCE_ARGS[@]+"${FORCE_ARGS[@]}"}")
if [[ $ADHOC -eq 1 ]]; then
  SIGN_ARGS+=(--adhoc)
fi
NOTARIZE_ARGS=("${FORCE_ARGS[@]+"${FORCE_ARGS[@]}"}")
if [[ -n "$PROFILE" ]]; then
  NOTARIZE_ARGS+=(--profile "$PROFILE")
fi

echo "==> Signing"
"$LAUNCHER_DIR/scripts/codesign-bundle.sh" sign "${SIGN_ARGS[@]+"${SIGN_ARGS[@]}"}" "$APP"

# The app has to be stapled before the DMG is built, because the DMG is what
# carries it to users.
if [[ $NOTARIZE -eq 1 ]]; then
  echo "==> Notarizing"
  "$LAUNCHER_DIR/scripts/codesign-bundle.sh" notarize "${NOTARIZE_ARGS[@]+"${NOTARIZE_ARGS[@]}"}" "$APP"
else
  echo "==> Skipping notarization; do not publish this build"
fi

mkdir -p "$OUT"
echo "==> Building $DMG"
"$LAUNCHER_DIR/scripts/make-dmg.sh" --out "$DMG" "$APP"

# A rehearsal stops at the DMG: an appcast would carry deltas and
# signatures made with the release key for a build nobody may publish.
if [[ $NOTARIZE -eq 1 ]]; then
  echo "==> Building the appcast"
  "$LAUNCHER_DIR/scripts/appcast.sh" "$VERSION" "$OUT"
else
  echo "==> Skipping the appcast for a rehearsal"
fi

echo
echo "==> Done"
echo "    version: $VERSION"
echo "    commit:  $COMMIT"
echo "    files:   $OUT"
if [[ $NOTARIZE -eq 0 ]]; then
  echo "    a rehearsal, not notarized and without an appcast: never publish it"
fi
