#!/bin/bash
set -e

# package.sh <tag> <outdir>: pack the bundle bundle-wine.sh made in its
# place in the layout (BUNDLE_DEST in profile.sh) as the release <tag>, into
# <outdir>:
#
#   wine-<tag>-macos-arm64.tar.xz          the bundle, as wine/
#   wine-<tag>-macos-arm64-symbols.tar.xz  wine-symbols/ (bundle-wine.sh
#                                          --symbols), without its tree marker
#   notes.md                               the start of the release notes
#
# The bundle is compressed at xz's default preset, since it is what people
# download. The symbols, more than twice the size, take a faster preset,
# which compresses them several times as fast for a somewhat larger archive.
# Both go through xz's own command, which has to be on PATH (Homebrew's xz).
#
# The notes are one bullet per source the bundle was built from: the Wine
# source, as the ref in pins/wine-src.ref with the commit and `git describe`
# of WINE_SRC, this repository's commit, and the commits FEX, Mesa, DXMT and
# vkd3d-proton were built from, whose refs are branches. The rest is pinned
# by this repository's commit, except the Homebrew libraries in
# lib/external, which the notes do not name. The Wine source and this
# repository must have no uncommitted changes, and the Wine SDK must be
# staged from that Wine commit, or the notes would name something the bundle
# was not built from.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# BUNDLE_DEST, WINE_SRC, WINE_SDK_DIR, FEX_DIST, MESA_DIST, DXMT_DIST,
# VKD3D_PROTON_DIST, the repositories and wine_src_commit.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

usage() {
    echo "Usage: $0 <tag> <outdir>"
    exit 1
}

[ $# -eq 2 ] || usage
case "$1" in
    -*) usage ;;
esac
TAG="$1"
OUT="$2"
WINE_DIR="$BUNDLE_DEST/wine"
SYMBOLS_DIR="$BUNDLE_DEST/wine-symbols"
BUNDLE_TAR="$OUT/wine-$TAG-macos-arm64.tar.xz"
SYMBOLS_TAR="$OUT/wine-$TAG-macos-arm64-symbols.tar.xz"
NOTES="$OUT/notes.md"

# github_name <url>: owner/name of a GitHub repository URL, or the URL as
# given when it is not one.
github_name() {
    local n="${1%.git}"
    n=${n#https://github.com/}
    n=${n#git@github.com:}
    echo "$n"
}

# short <commit>: the commit abbreviated as the release notes write it.
short() {
    echo "${1:0:11}"
}

# clean_tree <dir>: exit unless the git checkout <dir> has no uncommitted
# changes to tracked files.
clean_tree() {
    if [ -n "$(git -C "$1" status --porcelain --untracked-files=no)" ]; then
        echo "Error: $1 has uncommitted changes, so the notes cannot name what was built"
        git -C "$1" status --short --untracked-files=no
        exit 1
    fi
}

# pack <out> <dir> <name> <preset> [<tar option>...]: write <dir>/<name> to
# <out> as a tar compressed by xz at <preset>, with every core. Extended
# attributes, ACLs, file flags and AppleDouble files are left out, so nothing
# the build host attached (quarantine, provenance) travels with the bundle.
# Code signatures are inside the Mach-O files and wine.app, not in
# attributes. The compression is xz's own command and not tar's -J, because
# macOS's tar compresses on one thread whatever its xz:threads option says.
# A failure of either half removes <out>.
pack() {
    local out="$1" dir="$2" name="$3" preset="$4"
    shift 4
    echo "    $(basename "$out") (xz -$preset)"
    rm -f "$out"
    (
        set -o pipefail
        COPYFILE_DISABLE=1 tar --no-xattrs --no-acls --no-fflags --no-mac-metadata \
            "$@" -C "$dir" -cf - "$name" | xz -T0 "-$preset" > "$out"
    ) || {
        rm -f "$out"
        echo "Error: packing $dir/$name into $out failed"
        exit 1
    }
}

if [ ! -L "$WINE_DIR/bin/wine" ] && [ ! -e "$WINE_DIR/bin/wine" ]; then
    echo "Error: no bundle at $WINE_DIR (run bundle-wine.sh)"
    exit 1
fi
clean_tree "$REPO_DIR"
self="$(github_name "${GITHUB_REPOSITORY:-$(git -C "$REPO_DIR" remote get-url origin)}")@$(git -C "$REPO_DIR" rev-parse --short=11 HEAD)"
if [ ! -d "$SYMBOLS_DIR" ]; then
    echo "Error: no $SYMBOLS_DIR (run bundle-wine.sh with --symbols)"
    exit 1
fi
clean_tree "$WINE_SRC"
wine_commit=$(wine_src_commit)
if [ "$(cat "$WINE_SDK_DIR/WINE_COMMIT" 2>/dev/null)" != "$wine_commit" ]; then
    echo "Error: the Wine SDK at $WINE_SDK_DIR is not for $WINE_SRC at $wine_commit,"
    echo "so the bundle may not be either (run build-wine.sh and bundle-wine.sh)"
    exit 1
fi
# The describe form the notes use: the release tag and the number of
# commits on top of it, without the commit, which they name already.
describe=$(git -C "$WINE_SRC" describe --long HEAD 2>/dev/null) || {
    echo "Error: git describe finds no Wine release tag in $WINE_SRC; fetch the tag"
    echo "of the release the source is based on"
    exit 1
}
describe=${describe%-g*}
wine_ref=$(tr -d '[:space:]' < "$PINS_DIR/wine-src.ref")
wine_repo=$(github_name "$(git -C "$WINE_SRC" remote get-url origin)")
for f in "$FEX_DIST/COMMIT" "$MESA_DIST/COMMIT" "$DXMT_DIST/COMMIT" \
         "$VKD3D_PROTON_DIST/COMMIT"; do
    if [ ! -s "$f" ]; then
        echo "Error: no $f (run build-wine.sh)"
        exit 1
    fi
done
if ! command -v xz >/dev/null; then
    echo "Error: no xz on PATH (brew install xz)"
    exit 1
fi
mkdir -p "$OUT"

echo "==> Packing $WINE_DIR as $TAG"
pack "$BUNDLE_TAR" "$BUNDLE_DEST" wine 6
pack "$SYMBOLS_TAR" "$BUNDLE_DEST" wine-symbols 3 --exclude "$TREE_MARKER"
{
    echo "- Wine: $wine_repo@$wine_ref ($(short "$wine_commit"), $describe)"
    echo "- Scripts: $self"
    echo "- FEX: $(github_name "$FEX_REPO")@$(tr -d '[:space:]' < "$PINS_DIR/fex-src.ref") ($(short "$(cat "$FEX_DIST/COMMIT")"))"
    echo "- Mesa: $(github_name "$MESA_REPO")@$(tr -d '[:space:]' < "$PINS_DIR/mesa-src.ref") ($(short "$(cat "$MESA_DIST/COMMIT")"))"
    echo "- DXMT: $(github_name "$DXMT_REPO")@$(tr -d '[:space:]' < "$PINS_DIR/dxmt-src.ref") ($(short "$(cat "$DXMT_DIST/COMMIT")"))"
    echo "- vkd3d-proton: $(github_name "$VKD3D_PROTON_REPO")@$(tr -d '[:space:]' < "$PINS_DIR/vkd3d-proton-src.ref") ($(short "$(cat "$VKD3D_PROTON_DIST/COMMIT")"))"
} > "$NOTES"

echo "==> Notes ($NOTES)"
sed 's/^/    /' "$NOTES"
