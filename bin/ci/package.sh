#!/bin/bash
set -e

# package.sh [--base <release> [--base-notes <file>]] <tag> <outdir>: pack
# the bundle bundle-wine.sh made in its place in the layout (BUNDLE_DEST in
# profile.sh) as the release <tag>, into <outdir>:
#
#   wine-<tag>-macos-arm64.tar.xz          the bundle, as wine/
#   wine-<tag>-macos-arm64-symbols.tar.xz  wine-symbols/ (bundle-wine.sh
#                                          --symbols), without its tree marker
#   notes.md                               the start of the release notes
#
# The notes name what the bundle was built from: the Wine source, as the ref
# in pins/wine-src.ref with the commit and `git describe` of WINE_SRC, this
# repository's commit, and the commits FEX and Mesa were built from, whose
# refs are branches. The rest is pinned by this repository's commit. The
# Wine source and this repository must have no uncommitted changes, and the
# Wine SDK must be staged from that Wine commit, or the notes would name
# something the bundle was not built from.
#
# With --base, the bundle is the release <release> with compatdb.so replaced
# (bundle-wine.sh --compatdb-only). Only the bundle is packed, since the PE
# files and their symbols are the base's, and the notes name this
# repository's commit and the base, followed by the first line of
# <file>, which holds the base's notes.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# BUNDLE_DEST, WINE_SRC, WINE_SDK_DIR, FEX_BUILD_DIR, MESA_DIST, the
# repositories and wine_src_commit.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

usage() {
    echo "Usage: $0 [--base <release> [--base-notes <file>]] <tag> <outdir>"
    exit 1
}

BASE=""
BASE_NOTES=""
while [ $# -gt 0 ]; do
    case "$1" in
        --base) [ $# -ge 2 ] || usage; BASE="$2"; shift 2 ;;
        --base-notes) [ $# -ge 2 ] || usage; BASE_NOTES="$2"; shift 2 ;;
        -*) usage ;;
        *) break ;;
    esac
done
[ $# -eq 2 ] || usage
TAG="$1"
OUT="$2"
if [ -n "$BASE_NOTES" ] && [ -z "$BASE" ]; then
    usage
fi
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

# pack <out> <dir> <name> [<tar option>...]: write <dir>/<name> to <out> as
# an xz-compressed tar, compressing with every core. Extended attributes,
# ACLs, file flags and AppleDouble files are left out, so nothing the build
# host attached (quarantine, provenance) travels with the bundle. Code
# signatures are inside the Mach-O files and wine.app, not in attributes.
pack() {
    local out="$1" dir="$2" name="$3"
    shift 3
    echo "    $(basename "$out")"
    rm -f "$out"
    COPYFILE_DISABLE=1 tar --no-xattrs --no-acls --no-fflags --no-mac-metadata \
        --options xz:threads=0 "$@" -C "$dir" -cJf "$out" "$name"
}

if [ ! -L "$WINE_DIR/bin/wine" ] && [ ! -e "$WINE_DIR/bin/wine" ]; then
    echo "Error: no bundle at $WINE_DIR (run bundle-wine.sh)"
    exit 1
fi
clean_tree "$REPO_DIR"
self="$(github_name "${GITHUB_REPOSITORY:-$(git -C "$REPO_DIR" remote get-url origin)}")@$(git -C "$REPO_DIR" rev-parse --short=11 HEAD)"
mkdir -p "$OUT"

if [ -n "$BASE" ]; then
    echo "==> Packing $WINE_DIR as $TAG, from $BASE"
    pack "$BUNDLE_TAR" "$BUNDLE_DEST" wine
    {
        echo "compatdb from $self on Wine $BASE"
        if [ -n "$BASE_NOTES" ] && [ -n "$(head -n 1 "$BASE_NOTES" | tr -d '\r')" ]; then
            echo ""
            echo "$BASE: $(head -n 1 "$BASE_NOTES" | tr -d '\r')"
        fi
    } > "$NOTES"
else
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
    for f in "$FEX_BUILD_DIR/COMMIT" "$MESA_DIST/COMMIT"; do
        if [ ! -s "$f" ]; then
            echo "Error: no $f (run build-wine.sh)"
            exit 1
        fi
    done

    echo "==> Packing $WINE_DIR as $TAG"
    pack "$BUNDLE_TAR" "$BUNDLE_DEST" wine
    pack "$SYMBOLS_TAR" "$BUNDLE_DEST" wine-symbols --exclude "$TREE_MARKER"
    {
        echo "Wine distribution built from $wine_repo@$wine_ref ($(short "$wine_commit"), $describe) with $self."
        echo ""
        echo "- FEX: $(github_name "$FEX_REPO")@$(tr -d '[:space:]' < "$PINS_DIR/fex-src.ref") ($(short "$(cat "$FEX_BUILD_DIR/COMMIT")"))"
        echo "- Mesa: $(github_name "$MESA_REPO")@$(tr -d '[:space:]' < "$PINS_DIR/mesa-src.ref") ($(short "$(cat "$MESA_DIST/COMMIT")"))"
    } > "$NOTES"
fi

echo "==> Notes ($NOTES)"
sed 's/^/    /' "$NOTES"
