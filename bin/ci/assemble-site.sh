#!/bin/bash
set -euo pipefail

# assemble-site.sh <tag> <info-plist> <doc-dir> <appcast> <out-dir>:
# assemble the GitHub Pages site that pages.yml deploys at
# https://mach9.app into <out-dir>, which must not exist yet:
#
#   index.html                the landing page, site/index.html with its
#                             placeholders filled in (below)
#   mach9.png, dock-menu.jpg  the images it shows, from docs/images/
#   appcast.xml               <appcast>, unchanged
#   compatdb_core/, ...       everything in <doc-dir>, the rustdoc that
#                             cargo doc -p compatdb-core writes to
#                             target/doc
#
# The page names no version itself, so it never goes stale. The script fills
# in its placeholders:
#
#   @VERSION@  <tag>, the release GitHub marks Latest, without its v
#   @MACOS@    LSMinimumSystemVersion in <info-plist>
#
# <info-plist> is launcher/app/Info.plist as of <tag>, not of the checkout,
# so the page shows the floor of the app it links to even after main has
# raised it. The page text and the images come from the checkout.
#
# A placeholder (@WORD@) still in index.html afterwards, such as one added to
# the page without a value here, fails the script.
#
# Runs locally as well, to open the assembled site in a browser:
#
#   cargo doc --locked -p compatdb-core --no-deps
#   gh release download <tag> --repo athei/mach9 --pattern appcast.xml --dir <dir>
#   gh api "repos/athei/mach9/contents/launcher/app/Info.plist?ref=<tag>" \
#       -H "Accept: application/vnd.github.raw" > <dir>/Info.plist
#   bin/ci/assemble-site.sh <tag> <dir>/Info.plist target/doc <dir>/appcast.xml <out-dir>
#
# Needs python3 to read the Info.plist.

usage() {
    echo "Usage: $0 <tag> <info-plist> <doc-dir> <appcast> <out-dir>" >&2
    exit 2
}

die() {
    echo "Error: $*" >&2
    exit 1
}

[ $# -eq 5 ] || usage
case "$1" in
    -*) usage ;;
esac
TAG=$1
INFO_PLIST=$2
DOC_DIR=$3
APPCAST=$4
OUT_DIR=$5

REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
PAGE="$REPO_DIR/site/index.html"
IMAGES="$REPO_DIR/docs/images"

# Both values go into the page as they are, so only plain version strings
# are accepted: no character that HTML or the substitution below treats
# specially.
[[ "$TAG" =~ ^v[0-9][0-9A-Za-z.+-]*$ ]] || die "not a release tag: '$TAG'"
VERSION=${TAG#v}
[ -f "$INFO_PLIST" ] || die "no Info.plist at $INFO_PLIST"
MACOS=$(python3 -I -c '
import plistlib, sys
with open(sys.argv[1], "rb") as f:
    print(plistlib.load(f)["LSMinimumSystemVersion"])
' "$INFO_PLIST") || die "cannot read LSMinimumSystemVersion from $INFO_PLIST"
[[ "$MACOS" =~ ^[0-9]+(\.[0-9]+)*$ ]] || die "not a macOS version in $INFO_PLIST: '$MACOS'"

[ -f "$DOC_DIR/compatdb_core/index.html" ] || die "$DOC_DIR has no compatdb_core/index.html; run cargo doc -p compatdb-core first"
[ -f "$APPCAST" ] || die "no appcast at $APPCAST"
[ ! -e "$OUT_DIR" ] || die "$OUT_DIR exists already"

mkdir -p "$(dirname "$OUT_DIR")"
mkdir "$OUT_DIR"
cp -R "$DOC_DIR/." "$OUT_DIR/"
cp "$APPCAST" "$OUT_DIR/appcast.xml"
cp "$IMAGES/mach9.png" "$IMAGES/dock-menu.jpg" "$OUT_DIR/"

page=$(<"$PAGE")
page=${page//@VERSION@/$VERSION}
page=${page//@MACOS@/$MACOS}
printf '%s\n' "$page" > "$OUT_DIR/index.html"

if left=$(grep -oE '@[A-Za-z0-9_]+@' "$OUT_DIR/index.html"); then
    die "placeholders left in index.html: $(echo "$left" | sort -u | tr '\n' ' ')"
fi
echo "Assembled the site for $TAG (macOS $MACOS or later) in $OUT_DIR"
