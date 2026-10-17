# shellcheck shell=bash
# The settings build-wine.sh, bundle-wine.sh, build-fex.sh, build-mesa.sh and
# build-dxmt.sh share, and the helpers they use. Source it with SCRIPT_DIR set
# to the directory of those scripts (bin/ of this repository); it sets
# everything as it is sourced. REPO_DIR is the repository itself, PINS_DIR
# its pins/ (redist.env and the *-src.ref files).
#
# The build is a native arm64 Wine (aarch64, arm64ec and i386 PE) with FEX as
# the x86 emulator.
#
# Everything lives under ROOT, the directory that holds this repository: the
# sources (the Wine checkout in src/, FEX in src-fex/, Mesa in src-mesa/),
# downloads (cache/), every intermediate under build/, one directory per
# component, and the bundle in dist/wine. WINE_BUILD_ROOT overrides ROOT,
# for a checkout kept elsewhere (a worktree) that should use the same trees.
#
# A path set below as "${NAME:-default}" is overridden by an environment
# variable of the same name. MACOSX_DEPLOYMENT_TARGET may raise the floor but
# not lower it.
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PINS_DIR="$REPO_DIR/pins"
ROOT="${WINE_BUILD_ROOT:-$(cd "$REPO_DIR/.." && pwd)}"
# Where the build trees go by default, and where claim_tree adopts an
# unmarked one.
BUILD_TOP="$ROOT/build"

# macos_newer <a> <b>: succeed when macOS version <a> is newer than <b>.
# Major, minor and patch are compared as numbers and a missing part counts as
# 0, so 26.4 is newer than 15.0 and 15 is not newer than 15.0.
macos_newer() {
    local a1 a2 a3 b1 b2 b3
    IFS=. read -r a1 a2 a3 <<< "$1"
    IFS=. read -r b1 b2 b3 <<< "$2"
    [ $(( ${a1:-0} * 10000 + ${a2:-0} * 100 + ${a3:-0} )) -gt \
      $(( ${b1:-0} * 10000 + ${b2:-0} * 100 + ${b3:-0} )) ]
}

# The Wine source: upstream Wine with our series, a checkout of athei/wine.
WINE_SRC="${WINE_SRC:-$ROOT/src}"
BUILD_DIR="${BUILD_DIR:-$BUILD_TOP/wine}"
# What bundle-wine.sh writes <dest>/wine into without --dest.
BUNDLE_DEST="$ROOT/dist"
HOST_TRIPLE=aarch64-apple-darwin
WINE_ARCHS=arm64ec,aarch64,i386
# The arm64 Homebrew. Configure finds the libraries here and the bundle
# copies them from here.
DEPS_PREFIX=/opt/homebrew
DEPS_FORMULAE="freetype gnutls sdl2-compat sdl3 bison pkgconf"
# Mesa's repository (upstream Mesa plus our topic commits, for its Vulkan
# driver KosmicKrisp), the local clone build-mesa.sh checks out mesa-src.ref
# in, its trees, and the Vulkan loader and driver as build-mesa.sh stages
# them for the bundle. Set ahead of the configure flags, which name
# MESA_DIST.
MESA_REPO="${MESA_REPO:-https://github.com/athei/mesa}"
MESA_SRC="${MESA_SRC:-$ROOT/src-mesa}"
MESA_BUILD_DIR="${MESA_BUILD_DIR:-$BUILD_TOP/mesa}"
MESA_DIST="${MESA_DIST:-$MESA_BUILD_DIR/dist}"
# Configure arguments on top of the allowlist in build-wine.sh: the
# toolchain and where the dependencies are. Tests are left out: nothing runs
# them on this build. macOS ships bison 2.3, too old for Wine. The include
# path goes into CPPFLAGS so configure keeps its default -g -O2 CFLAGS.
# Putting $DEPS_PREFIX first in both search paths keeps clang and ld from
# taking an x86_64 header or library from /usr/local, which they search by
# default. Ahead of it, $MESA_DIST/lib holds the Vulkan loader build-mesa.sh
# built, so configure's -lvulkan test links that one: it succeeds without
# Homebrew's vulkan-loader and does not link Homebrew's libvulkan.dylib where
# it is installed. The soname define is the bare file name either way; that
# the bundle carries this loader is the bundle step's doing.
CONFIGURE_TOOLCHAIN=(CC="clang -arch arm64"
                     --host="$HOST_TRIPLE"
                     --disable-tests
                     BISON="$DEPS_PREFIX/opt/bison/bin/bison"
                     PKG_CONFIG_PATH="$DEPS_PREFIX/lib/pkgconfig"
                     CPPFLAGS="-I$DEPS_PREFIX/include"
                     LDFLAGS="-L$MESA_DIST/lib -L$DEPS_PREFIX/lib")
# Path prefixes no bundled Mach-O may still load from. An x86_64 library
# from /usr/local cannot load into an arm64 process, so a reference to it is
# as much a leak as one to the arm64 Homebrew.
LEAK_PREFIXES="/usr/local/ /opt/homebrew/"
# The floor every Mach-O is held to. The loader's cross-architecture
# entitlement needs macOS 26.4, so nothing in this bundle aims lower.
MACOS_FLOOR=26.4
if [ -n "$MACOSX_DEPLOYMENT_TARGET" ]; then
    if macos_newer "$MACOS_FLOOR" "$MACOSX_DEPLOYMENT_TARGET"; then
        echo "Error: MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET is below the floor $MACOS_FLOOR"
        exit 1
    fi
    MACOS_FLOOR="$MACOSX_DEPLOYMENT_TARGET"
fi
# Functions Wine's configure checks for that the macOS SDK declares, but
# that macOS only has from a release above the floor, as <function>:<macOS
# version>. configure finds them in the SDK, Wine calls them without an
# availability check, and so the linker weak-imports them and on an older
# macOS the call jumps to address 0 (pipe2 is what ntdll's server_pipe()
# calls first in every process). While the floor is below a function's
# version, configure is told the function is missing, and Wine uses the older
# calls it has for that case. The macOS 27 SDK declares pipe2 as available
# from macOS 27 on, above this floor; an older SDK has no pipe2, so there the
# override changes nothing.
FLOOR_NEWER_FUNCS="pipe2:27.0"
for floor_func in $FLOOR_NEWER_FUNCS; do
    if macos_newer "${floor_func#*:}" "$MACOS_FLOOR"; then
        CONFIGURE_TOOLCHAIN+=("ac_cv_func_${floor_func%%:*}=no")
    fi
done
unset floor_func
export MACOSX_DEPLOYMENT_TARGET="$MACOS_FLOOR"
UNIX_DIR=aarch64-unix
# An x64 process runs as ARM64EC, and Wine looks up its builtins in
# aarch64-windows only, so the x86_64 PE modules of DXMT and mtld3d go there
# too.
PE64_DIR=aarch64-windows
RUST_TARGET=aarch64-apple-darwin
# The Direct3D trees, in the order a default-dir marker prefers them. Each
# name is a tree under lib/wine/dxgi or lib/wine/d3d9.
DXGI_IMPLS="dxmt wined3d"
D3D9_IMPLS="mtld3d wined3d"
# The link-library tree for the x86_64 PE archives and the Wine SDK
# build-wine.sh stages from it and the main build.
X86LIBS_BUILD_DIR="${X86LIBS_BUILD_DIR:-$BUILD_TOP/x86libs}"
WINE_SDK_DIR="${WINE_SDK_DIR:-$BUILD_TOP/sdk}"
# DXMT's checkout and trees, the LLVM libraries it links, and DXMT as
# build-dxmt.sh stages it for the bundle.
DXMT_BUILD_DIR="${DXMT_BUILD_DIR:-$BUILD_TOP/dxmt}"
DXMT_LLVM="${DXMT_LLVM:-$DXMT_BUILD_DIR/llvm15}"
DXMT_DIST="${DXMT_DIST:-$DXMT_BUILD_DIR/dist}"
# FEX's repository, the local clone build-fex.sh checks out fex-src.ref in,
# and its build trees.
FEX_REPO="${FEX_REPO:-https://github.com/athei/FEX}"
FEX_SRC="${FEX_SRC:-$ROOT/src-fex}"
FEX_BUILD_DIR="${FEX_BUILD_DIR:-$BUILD_TOP/fex}"
LOADER_PROFILE="${LOADER_PROFILE:-$REPO_DIR/signing/loader.provisionprofile}"
# The llvm-mingw targets the smoke test builds smoke/hello.c for and runs:
# native, and the two x86 ones FEX emulates.
SMOKE_PE_ARCHS="aarch64 x86_64 i686"
# Where `make install` puts the tree under DESTDIR. Wine's default --prefix,
# which configure is never given; the bundle step moves this directory to
# <dest>/wine.
INSTALL_PREFIX=/usr/local

# The file that marks a directory as a tree these scripts made and may
# delete. It holds the tree's name.
TREE_MARKER=.wine-build-tree

# canonical_path <dir>: <dir> with symlinks, . and .. resolved, or as given
# when it does not exist.
canonical_path() {
    if [ -d "$1" ]; then
        (cd "$1" && pwd -P)
    else
        echo "$1"
    fi
}

# claim_tree <dir> <name> [<wipe>]: make <dir> the tree <name>, emptied first
# when <wipe> is 1, and mark it. Every tree a script fills or deletes goes
# through here first, since an override can point one anywhere. Exit unless
# <dir> is missing, empty, or marked as that same tree. A marker that says
# "arm64 <name>" is that same tree too: it was written while these scripts
# also built an x86_64 Wine and named the profile. A directory with no marker
# is also taken when its parent is BUILD_TOP or another tree: the default
# places of trees made before there were markers. A git checkout never is.
claim_tree() {
    local dir="$1" mark="$2" found parent
    if [ -z "$dir" ]; then
        echo "Error: no directory for the tree '$mark'"
        exit 1
    elif [ -f "$dir/$TREE_MARKER" ]; then
        found=$(cat "$dir/$TREE_MARKER")
        if [ "$found" != "$mark" ] && [ "$found" != "arm64 $mark" ]; then
            echo "Error: $dir is the tree '$found', not '$mark'"
            exit 1
        fi
    elif [ -e "$dir" ] && [ -n "$(ls -A "$dir")" ]; then
        parent=$(dirname "$(canonical_path "$dir")")
        if [ -e "$dir/.git" ] || { [ "$parent" != "$(canonical_path "$BUILD_TOP")" ] \
            && [ ! -f "$parent/$TREE_MARKER" ]; }; then
            echo "Error: $dir is not empty and has no $TREE_MARKER, so these scripts did not make it."
            echo "Refusing to use it; remove it or point the override elsewhere."
            exit 1
        fi
    fi
    if [ "${3:-0}" -eq 1 ]; then
        remove_tree "$dir"
    fi
    mkdir -p "$dir"
    echo "$mark" > "$dir/$TREE_MARKER"
}

# check_sdk_floor: exit unless the macOS SDK is at least MACOS_FLOOR. Code
# that needs a newer macOS than the SDK knows cannot be compiled against it.
check_sdk_floor() {
    local sdk
    sdk=$(xcrun --show-sdk-version 2>/dev/null) || {
        echo "Error: xcrun cannot report the macOS SDK version (install the Xcode command line tools)"
        exit 1
    }
    if macos_newer "$MACOS_FLOOR" "$sdk"; then
        echo "Error: the macOS SDK is $sdk, older than the floor $MACOS_FLOOR"
        exit 1
    fi
}

# remove_tree <path>...: rm -rf, tried again when it fails. macOS can write a
# .DS_Store into a directory (a Finder window showing it is enough) while rm
# is emptying it, and rm then stops with "Directory not empty"; a second
# attempt removes the latecomer. A third failure is real and exits.
remove_tree() {
    local try
    for try in 1 2; do
        if rm -rf "$@" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done
    rm -rf "$@"
}

# pe_machine <file>: print the COFF Machine field of a PE image as four hex
# digits: 8664 for x86_64, 014c for i386, aa64 for ARM64 (an ARM64X image
# says aa64 too). The PE header starts at the offset stored at 0x3c, and the
# field follows its four-byte signature. od reads both little-endian, as the
# host is.
pe_machine() {
    local off
    off=$(od -An -tu4 -j60 -N4 "$1" | tr -d ' ')
    od -An -tx2 -j$((off + 4)) -N2 "$1" | tr -d ' '
}

# wine_src_commit: print the commit WINE_SRC is at, with "-dirty" appended
# when tracked files differ from it, so a Wine SDK and what links against it
# can be matched to one source state.
wine_src_commit() {
    local commit
    commit=$(git -C "$WINE_SRC" rev-parse HEAD)
    if [ -n "$(git -C "$WINE_SRC" status --porcelain --untracked-files=no)" ]; then
        commit="$commit-dirty"
    fi
    echo "$commit"
}

# src_ref_commit <ref-file> <repo>: print the commit the ref in <ref-file>
# (a file in PINS_DIR, such as fex-src.ref) names in <repo>: the ref itself
# when it is a full commit hash, else the tag (peeled, for an annotated one)
# or branch of that name, preferring a tag as git does. Prints nothing when
# the ref cannot be resolved, also when <repo> wants credentials: git is not
# allowed to ask for them.
src_ref_commit() {
    local ref
    ref=$(tr -d '[:space:]' < "$PINS_DIR/$1")
    if [[ "$ref" =~ ^[0-9a-f]{40}$ ]]; then
        echo "$ref"
        return 0
    fi
    GIT_TERMINAL_PROMPT=0 git ls-remote "$2" "$ref" 2>/dev/null | awk -v ref="$ref" '
        $2 == "refs/tags/" ref "^{}" { peeled = $1 }
        $2 == "refs/tags/" ref { tag = $1 }
        $2 == "refs/heads/" ref { head = $1 }
        END { if (peeled != "") print peeled; else if (tag != "") print tag; else if (head != "") print head }'
}

# pinned_clone <url> <tag> <commit> <dir>: make <dir> a shallow clone of
# <tag> with its submodules, and exit unless the tag is at <commit>. An
# existing clone at another commit (a bumped pin) is replaced; it is the
# calling script's own scratch copy, never somebody's work tree. The
# submodules are brought to the recorded commits every time, so an
# interrupted earlier run cannot leave them missing.
pinned_clone() {
    local url="$1" tag="$2" commit="$3" dir="$4" head
    if [ -e "$dir/.git" ]; then
        head=$(git -C "$dir" rev-parse HEAD)
        if [ "$head" != "$commit" ]; then
            echo "    $dir is at $head, not $commit; cloning again"
            remove_tree "$dir"
        fi
    fi
    if [ ! -e "$dir/.git" ]; then
        echo "    cloning $url $tag"
        git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" "$url" "$dir"
        head=$(git -C "$dir" rev-parse HEAD)
        if [ "$head" != "$commit" ]; then
            echo "Error: $url $tag is at $head, redist.env pins $commit"
            exit 1
        fi
    fi
    git -C "$dir" submodule update --quiet --init --recursive --depth 1
}

# check_deps_formulae: exit unless every formula in DEPS_FORMULAE is
# installed in DEPS_PREFIX.
check_deps_formulae() {
    local f missing=""
    for f in $DEPS_FORMULAE; do
        [ -d "$DEPS_PREFIX/opt/$f" ] || missing="$missing $f"
    done
    if [ -n "$missing" ]; then
        echo "Error: not installed in $DEPS_PREFIX:$missing"
        echo "Run: $DEPS_PREFIX/bin/brew install$missing"
        exit 1
    fi
}
