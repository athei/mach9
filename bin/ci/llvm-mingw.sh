#!/bin/bash
set -e

# llvm-mingw.sh <dir>: install the llvm-mingw release redist.env pins
# (LLVM_MINGW_VERSION and LLVM_MINGW_SHA256) at <dir>, for a CI build to use
# as MINGW_DIR. The archive is downloaded into CACHE_DIR, the same directory
# bundle-wine.sh keeps its downloads in, and a copy there is reused while its
# checksum matches the pin. A fresh download that does not match is an
# error. <dir> must not exist, unless it holds this same release already
# (its .llvm-mingw-version file names the version), which is left alone.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# ROOT and PINS_DIR.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"
# shellcheck source=../../pins/redist.env
. "$PINS_DIR/redist.env"
CACHE_DIR="${CACHE_DIR:-$ROOT/cache}"

if [ $# -ne 1 ]; then
    echo "Usage: $0 <dir>"
    exit 1
fi
DEST="$1"
NAME="llvm-mingw-$LLVM_MINGW_VERSION-ucrt-macos-universal"
URL="https://github.com/mstorsjo/llvm-mingw/releases/download/$LLVM_MINGW_VERSION/$NAME.tar.xz"
ARCHIVE="$CACHE_DIR/$NAME.tar.xz"
MARKER=.llvm-mingw-version

if [ -e "$DEST" ]; then
    if [ "$(cat "$DEST/$MARKER" 2>/dev/null)" = "$LLVM_MINGW_VERSION" ]; then
        echo "==> llvm-mingw $LLVM_MINGW_VERSION is installed at $DEST already"
        exit 0
    fi
    echo "Error: $DEST exists and is not llvm-mingw $LLVM_MINGW_VERSION installed by this script"
    exit 1
fi

echo "==> llvm-mingw $LLVM_MINGW_VERSION"
if [ -f "$ARCHIVE" ] && [ "$(shasum -a 256 "$ARCHIVE" | cut -d' ' -f1)" = "$LLVM_MINGW_SHA256" ]; then
    echo "    $NAME.tar.xz (cached)"
else
    echo "    $NAME.tar.xz (downloading)"
    mkdir -p "$CACHE_DIR"
    rm -f "$ARCHIVE"
    curl -fsSL --retry 3 -o "$ARCHIVE.part" "$URL"
    got=$(shasum -a 256 "$ARCHIVE.part" | cut -d' ' -f1)
    if [ "$got" != "$LLVM_MINGW_SHA256" ]; then
        rm -f "$ARCHIVE.part"
        echo "Error: $NAME.tar.xz checksum mismatch"
        echo "  expected $LLVM_MINGW_SHA256"
        echo "  got      $got"
        echo "  from     $URL"
        exit 1
    fi
    mv "$ARCHIVE.part" "$ARCHIVE"
fi

# Unpacked next to <dir> and renamed into place, so an interrupted run never
# leaves a <dir> behind that looks complete.
part="$DEST.part"
rm -rf "$part"
mkdir -p "$part"
tar -C "$part" -xJf "$ARCHIVE"
if [ ! -d "$part/$NAME/bin" ]; then
    echo "Error: $NAME.tar.xz has no $NAME/bin"
    exit 1
fi
for triple in aarch64 arm64ec x86_64 i686; do
    if [ ! -x "$part/$NAME/bin/$triple-w64-mingw32-clang" ]; then
        echo "Error: $NAME has no $triple-w64-mingw32-clang"
        exit 1
    fi
done
echo "$LLVM_MINGW_VERSION" > "$part/$NAME/$MARKER"
mv "$part/$NAME" "$DEST"
rm -rf "$part"
echo "    installed at $DEST"
