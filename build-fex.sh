#!/bin/bash
set -e

# Builds FEX, the x86 emulator of the arm64 Wine, from athei/FEX (upstream FEX
# with our commits on top; FEX_REPO names another repository) at the ref in
# fex-src.ref (a branch, tag or commit). Called by build-wine.sh --arm64;
# running it alone rebuilds FEX only. Three CMake trees under FEX_BUILD_DIR,
# following Hadron's recipe:
#
#   wow64/    libwow64fex.dll, an aarch64 PE that runs i386 code under WoW64
#             (Wine loads it as xtajit.dll)
#   arm64ec/  libarm64ecfex.dll, an ARM64EC PE that runs x86_64 code
#             (Wine loads it as xtajit64.dll)
#   unixlib/  libwow64fex.so and libarm64ecfex.so, their macOS halves
#
# Nothing is installed here: bundle-wine.sh --arm64 installs from these trees.
#
# --clean wipes FEX_BUILD_DIR first.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MINGW_DIR="${MINGW_DIR:-/opt/llvm-mingw}"

CLEAN=0
while [ $# -gt 0 ]; do
    case "$1" in
        --clean) CLEAN=1; shift ;;
        *) echo "Usage: $0 [--clean]"; exit 1 ;;
    esac
done

# FEX only exists in the arm64 profile, which also names its repository and
# clone, says where the trees go and pins the floor of the unix half.
# shellcheck source=profile.sh
. "$SCRIPT_DIR/profile.sh"
set_profile arm64

FEX_REF="$(tr -d '[:space:]' < "$SCRIPT_DIR/fex-src.ref")"
if [ -z "$FEX_REF" ]; then
    echo "Error: $SCRIPT_DIR/fex-src.ref is empty"
    exit 1
fi

for tool in cmake ninja git; do
    if ! command -v "$tool" >/dev/null; then
        echo "Error: $tool not found (brew install $tool)"
        exit 1
    fi
done
for triple in aarch64-w64-mingw32 arm64ec-w64-mingw32; do
    if [ ! -x "$MINGW_DIR/bin/$triple-clang" ]; then
        echo "Error: llvm-mingw at $MINGW_DIR has no $triple toolchain"
        exit 1
    fi
done
# Appended, never prepended: llvm-mingw's bin/ has an unprefixed clang that
# would otherwise stand in for Apple's in the unixlib build.
export PATH="$PATH:$MINGW_DIR/bin"

# ── Source ──────────────────────────────────────────────────────────────
# The ref is fetched from FEX_REPO by URL, so this works the same whatever
# the local clone calls its remotes. A tree with local changes is refused
# rather than reset: those changes are somebody's work, and building from
# them would not be building the ref. Submodules are left out of that check
# because the update below moves them to the commits the ref records. git
# is kept from asking for credentials, so an unattended run fails instead of
# waiting.
echo "==> FEX source: $FEX_REPO $FEX_REF"
if [ ! -e "$FEX_SRC/.git" ]; then
    GIT_TERMINAL_PROMPT=0 git clone "$FEX_REPO" "$FEX_SRC"
fi
if [ -n "$(git -C "$FEX_SRC" status --porcelain --untracked-files=no --ignore-submodules=all)" ]; then
    echo "Error: $FEX_SRC has local changes; commit or stash them first"
    git -C "$FEX_SRC" status --short --untracked-files=no --ignore-submodules=all
    exit 1
fi
GIT_TERMINAL_PROMPT=0 git -C "$FEX_SRC" fetch --quiet "$FEX_REPO" "$FEX_REF"
git -C "$FEX_SRC" checkout --quiet --detach FETCH_HEAD
git -C "$FEX_SRC" submodule update --init --recursive
# A leading '+', '-' or 'U' marks a submodule that is not at the recorded
# commit, not checked out, or in conflict.
if git -C "$FEX_SRC" submodule status --recursive | grep -q '^[-+U]'; then
    echo "Error: FEX submodules do not match $FEX_REF:"
    git -C "$FEX_SRC" submodule status --recursive | grep '^[-+U]'
    exit 1
fi
echo "    at $(git -C "$FEX_SRC" rev-parse HEAD)"

if [ "$CLEAN" -eq 1 ]; then
    echo "==> Cleaning $FEX_BUILD_DIR..."
fi
claim_tree "$FEX_BUILD_DIR" fex "$CLEAN"
# Stale until the build below finishes.
rm -f "$FEX_BUILD_DIR/COMMIT"

# ── PE halves ───────────────────────────────────────────────────────────
# FEX's own mingw toolchain file picks <triple>-clang from PATH. The install
# directory is where Wine looks for an aarch64 or ARM64EC builtin.
for target in wow64 arm64ec; do
    case $target in
        wow64)   triple=aarch64-w64-mingw32 ;;
        arm64ec) triple=arm64ec-w64-mingw32 ;;
    esac
    echo "==> Building FEX $target ($triple)..."
    cmake -S "$FEX_SRC" -B "$FEX_BUILD_DIR/$target" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_TOOLCHAIN_FILE="$FEX_SRC/Data/CMake/toolchain_mingw.cmake" \
        -DMINGW_TRIPLE=$triple \
        -DCMAKE_INSTALL_LIBDIR="lib/wine/$PE64_DIR" \
        -DENABLE_LTO=False -DENABLE_ASSERTIONS=False -DENABLE_JEMALLOC_GLIBC_ALLOC=False \
        -DBUILD_TESTING=False -DTUNE_ARCH=generic -DTUNE_CPU=none -DRANGES_NATIVE=OFF \
        >/dev/null
    cmake --build "$FEX_BUILD_DIR/$target"
done

# ── Unix half ───────────────────────────────────────────────────────────
# Apple's compilers named explicitly, held to the profile's floor like the
# rest of the bundle.
echo "==> Building the FEX unixlib..."
cmake -S "$FEX_SRC/Source/Windows/UnixLib" -B "$FEX_BUILD_DIR/unixlib" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER=/usr/bin/clang -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
    -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_FLOOR" \
    -DCMAKE_INSTALL_LIBDIR="lib/wine/$UNIX_DIR" \
    >/dev/null
cmake --build "$FEX_BUILD_DIR/unixlib"

# The commit the trees were built from, which bundle-wine.sh compares with
# what fex-src.ref names.
git -C "$FEX_SRC" rev-parse HEAD > "$FEX_BUILD_DIR/COMMIT"
echo "==> FEX $(cat "$FEX_BUILD_DIR/COMMIT") built in $FEX_BUILD_DIR."
