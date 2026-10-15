# shellcheck shell=bash
# The arch profile shared by build-wine.sh and bundle-wine.sh. Source it with
# SCRIPT_DIR set (this repository), then call set_profile with the profile
# name. Every difference between the two builds is a value or a flag here;
# the steps in the scripts test the flags, never the profile name.
#
# intel  x86_64 Wine (i386 and x86_64 PE) from the CrossOver tree, run under
#        Rosetta. What the releases ship.
# arm64  Native arm64 Wine (aarch64, arm64ec and i386 PE) with FEX as the x86
#        emulator, from the arm64 worktree of the Wine source. Local only.
#
# Everything lives under ROOT, the directory that holds this repository: the
# sources (the Wine checkout of each profile under src/<profile>/, FEX in
# src-fex/), downloads (cache/), every intermediate of a profile under
# build/<profile>/ and its bundle under dist/<profile>/wine.
# WINE_BUILD_ROOT overrides ROOT, for a checkout kept elsewhere (a worktree)
# that should use the same trees.
#
# Environment overrides of single paths are per profile, so a value exported
# for one build cannot redirect the other: WINE_SRC and BUILD_DIR for intel;
# ARM64_WINE_SRC, ARM64_BUILD_DIR, ARM64_X86LIBS_BUILD_DIR and ARM64_WINE_SDK
# for arm64 (whose FEX and DXMT paths exist only there and keep their own
# names).
# MACOSX_DEPLOYMENT_TARGET overrides the intel floor, and may only raise the
# arm64 one.
ROOT="${WINE_BUILD_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"

# set_profile <intel|arm64>: set the profile's variables, or exit on an
# unknown name.
set_profile() {
    PROFILE="$1"
    case "$PROFILE" in
        intel)
            # The checkout that holds the Wine repository.
            WINE_SRC="${WINE_SRC:-$ROOT/src/intel}"
            BUILD_DIR="${BUILD_DIR:-$ROOT/build/intel/wine}"
            # What bundle-wine.sh writes <dest>/wine into without --dest.
            BUNDLE_DEST="$ROOT/dist/intel"
            # Wine's configure triple, and what runs configure and
            # `make install`: both probe the host, so they run translated.
            HOST_TRIPLE=x86_64-apple-darwin
            CONFIGURE_WRAPPER=(arch -x86_64)
            WINE_ARCHS=i386,x86_64
            # The x86_64 Homebrew (or the deps-* tarball CI unpacks there).
            # Configure finds the libraries here and the bundle copies them
            # from here.
            DEPS_PREFIX=/usr/local
            # Not checked: the formulae are ROOTS in package-deps.sh, and CI
            # unpacks a tarball rather than installing them.
            DEPS_FORMULAE=""
            # Configure arguments on top of the shared allowlist in
            # build-wine.sh: the options only this profile's Wine tree has,
            # then the toolchain and where the dependencies are. A CFLAGS
            # here replaces configure's default -g -O2 for the unix side;
            # kept as it is because it is what every release was built with.
            CONFIGURE_FEATURES_EXTRA=(--with-unwind)
            CONFIGURE_TOOLCHAIN=(CC="clang -arch x86_64"
                                 CROSSCC="clang -arch x86_64"
                                 --host="$HOST_TRIPLE"
                                 PKG_CONFIG_PATH="$DEPS_PREFIX/lib/pkgconfig"
                                 CFLAGS="-I$DEPS_PREFIX/include"
                                 LDFLAGS="-L$DEPS_PREFIX/lib")
            # Path prefixes no bundled Mach-O may still load from.
            LEAK_PREFIXES="/usr/local/"
            # install_name_tool leaves the relinked dylibs with a broken
            # signature, which x86_64 code under Rosetta does not check.
            SIGN_RELINKED=0
            # The floor every Mach-O is held to. Without the pin clang takes
            # the deployment target from whatever host builds, so the floor
            # was an accident of the runner image: cx-26.3.0-3 shipped minos
            # 15.0 off the macos-15 runner, while a local build on macOS 27
            # produced minos 26.0. GPTK's D3DMetal, the default for 64-bit
            # D3D10 to D3D12, needs macOS 26.4 because it links
            # libdxccontainer.dylib; below that compatdb.so gives those
            # processes DXMT instead. .cargo/config.toml carries the same
            # value for a plain cargo build of compatdb.
            MACOS_FLOOR="${MACOSX_DEPLOYMENT_TARGET:-15.0}"
            # See after the profiles. The macOS 27 SDK declares pipe2 as
            # available from macOS 27 on, above this floor. An older SDK
            # has no pipe2, so there the override changes nothing.
            FLOOR_NEWER_FUNCS="pipe2:27.0"
            UNIX_DIR=x86_64-unix
            PE64_DIR=x86_64-windows
            RUST_TARGET=x86_64-apple-darwin
            # The Direct3D trees, in the order a default-dir marker prefers
            # them. Each name is a tree under lib/wine/dxgi or lib/wine/d3d9.
            DXGI_IMPLS="gptk dxmt wined3d"
            D3D9_IMPLS="mtld3d wined3d"
            # All of DXMT, winemetal.so included, comes from the release
            # tarball pinned in redist.env.
            DXMT_BUILD_DIR=""
            DXMT_LLVM=""
            DXMT_DIST=""
            X86LIBS_BUILD_DIR=""
            WINE_SDK_DIR=""
            FEX_BUILD_DIR=""
            LOADER_PROFILE=""
            SMOKE_TEST=smoke_test
            SMOKE_PE_ARCHS=""
            WITH_X87SIDECAR=1
            WITH_D3D9_TESTS=1
            WITH_WINE_SDK=0
            WITH_DXMT_BUILD=0
            WITH_FEX=0
            WITH_LOADER_APP=0
            ;;
        arm64)
            # The arm64 Wine is a linked worktree of the same repository.
            WINE_SRC="${ARM64_WINE_SRC:-$ROOT/src/arm64}"
            BUILD_DIR="${ARM64_BUILD_DIR:-$ROOT/build/arm64/wine}"
            BUNDLE_DEST="$ROOT/dist/arm64"
            HOST_TRIPLE=aarch64-apple-darwin
            CONFIGURE_WRAPPER=()
            WINE_ARCHS=arm64ec,aarch64,i386
            DEPS_PREFIX=/opt/homebrew
            DEPS_FORMULAE="freetype gnutls sdl2-compat sdl3 bison pkgconf"
            # Upstream Wine has VA-API and no libunwind option.
            CONFIGURE_FEATURES_EXTRA=(--without-va)
            # Tests are left out: nothing runs them on this build. macOS
            # ships bison 2.3, too old for Wine. The include path goes into
            # CPPFLAGS so configure keeps its default -g -O2 CFLAGS. Putting
            # $DEPS_PREFIX first in both search paths keeps clang and ld from
            # taking an x86_64 header or library from /usr/local, which they
            # search by default.
            CONFIGURE_TOOLCHAIN=(CC="clang -arch arm64"
                                 --host="$HOST_TRIPLE"
                                 --disable-tests
                                 BISON="$DEPS_PREFIX/opt/bison/bin/bison"
                                 PKG_CONFIG_PATH="$DEPS_PREFIX/lib/pkgconfig"
                                 CPPFLAGS="-I$DEPS_PREFIX/include"
                                 LDFLAGS="-L$DEPS_PREFIX/lib")
            # An x86_64 library from /usr/local cannot load into an arm64
            # process, so a reference to it is as much a leak as one to the
            # arm64 Homebrew.
            LEAK_PREFIXES="/usr/local/ /opt/homebrew/"
            # arm64 code only runs with a valid signature, and
            # install_name_tool invalidates the one Homebrew's linker made.
            SIGN_RELINKED=1
            # The loader's cross-architecture entitlement needs macOS 26.4,
            # so nothing in this bundle aims lower.
            MACOS_FLOOR=26.4
            if [ -n "$MACOSX_DEPLOYMENT_TARGET" ]; then
                if macos_newer "$MACOS_FLOOR" "$MACOSX_DEPLOYMENT_TARGET"; then
                    echo "Error: MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET is below the arm64 floor $MACOS_FLOOR"
                    exit 1
                fi
                MACOS_FLOOR="$MACOSX_DEPLOYMENT_TARGET"
            fi
            # See after the profiles. The macOS 27 SDK declares pipe2 as
            # available from macOS 27 on, above this floor.
            FLOOR_NEWER_FUNCS="pipe2:27.0"
            UNIX_DIR=aarch64-unix
            # An x64 process runs as ARM64EC, and Wine looks up its builtins
            # in aarch64-windows only, so the x86_64 PE modules of DXMT and
            # mtld3d go there too.
            PE64_DIR=aarch64-windows
            RUST_TARGET=aarch64-apple-darwin
            # No gptk: Apple's D3DMetal exists for x86_64 only.
            DXGI_IMPLS="dxmt wined3d"
            D3D9_IMPLS="mtld3d wined3d"
            # The link-library tree for the x86_64 PE archives and the Wine
            # SDK build-wine.sh stages from it and the main build.
            X86LIBS_BUILD_DIR="${ARM64_X86LIBS_BUILD_DIR:-$ROOT/build/arm64/x86libs}"
            WINE_SDK_DIR="${ARM64_WINE_SDK:-$ROOT/build/arm64/sdk}"
            # DXMT's checkout and trees, the LLVM libraries it links, and DXMT
            # as build-dxmt.sh stages it for the bundle.
            DXMT_BUILD_DIR="${DXMT_BUILD_DIR:-$ROOT/build/arm64/dxmt}"
            DXMT_LLVM="${DXMT_LLVM:-$DXMT_BUILD_DIR/llvm15}"
            DXMT_DIST="${DXMT_DIST:-$DXMT_BUILD_DIR/dist}"
            # FEX's repository, the local clone build-fex.sh checks out
            # fex-src.ref in, and its build trees.
            FEX_REPO="${FEX_REPO:-https://github.com/athei/FEX}"
            FEX_SRC="${FEX_SRC:-$ROOT/src-fex}"
            FEX_BUILD_DIR="${FEX_BUILD_DIR:-$ROOT/build/arm64/fex}"
            LOADER_PROFILE="${LOADER_PROFILE:-$SCRIPT_DIR/signing/loader.provisionprofile}"
            SMOKE_TEST=smoke_test_arm64
            # The llvm-mingw targets the smoke test builds smoke/hello.c for
            # and runs: native, and the two x86 ones FEX emulates.
            SMOKE_PE_ARCHS="aarch64 x86_64 i686"
            WITH_X87SIDECAR=0
            WITH_D3D9_TESTS=0
            WITH_WINE_SDK=1
            WITH_DXMT_BUILD=1
            WITH_FEX=1
            WITH_LOADER_APP=1
            ;;
        *)
            echo "Error: unknown profile '$PROFILE'"
            exit 1
            ;;
    esac
    # FLOOR_NEWER_FUNCS: functions Wine's configure checks for that the
    # macOS SDK declares, but that macOS only has from a release above the
    # floor, as <function>:<macOS version>. configure finds them in the SDK,
    # Wine calls them without an availability check, and so the linker
    # weak-imports them and on an older macOS the call jumps to address 0
    # (pipe2 is what ntdll's server_pipe() calls first in every process).
    # While the floor is below a function's version, configure is told the
    # function is missing, and Wine uses the older calls it has for that
    # case.
    local func
    for func in $FLOOR_NEWER_FUNCS; do
        if macos_newer "${func#*:}" "$MACOS_FLOOR"; then
            CONFIGURE_TOOLCHAIN+=("ac_cv_func_${func%%:*}=no")
        fi
    done
    # Where `make install` puts the tree under DESTDIR. Wine's default
    # --prefix, which configure is never given; the bundle step moves this
    # directory to <dest>/wine.
    INSTALL_PREFIX=/usr/local
    export MACOSX_DEPLOYMENT_TARGET="$MACOS_FLOOR"
}

# The file that marks a directory as a tree these scripts made and may
# delete. It holds "<profile> <name>", the tree's profile and what it is.
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

# claim_tree <dir> <name> [<wipe>]: make <dir> the profile's tree <name>,
# emptied first when <wipe> is 1, and mark it. Every tree a script fills or
# deletes goes through here first, since an override can point one anywhere.
# Exit unless <dir> is missing, empty, or marked as that same tree. A
# directory with no marker is also taken when its parent is build/<profile>
# or another tree of the profile: the default places of trees made before
# there were markers. A git checkout never is.
claim_tree() {
    local dir="$1" mark="$PROFILE $2" found parent
    if [ -z "$dir" ]; then
        echo "Error: no directory for the tree '$mark'"
        exit 1
    elif [ -f "$dir/$TREE_MARKER" ]; then
        found=$(cat "$dir/$TREE_MARKER")
        if [ "$found" != "$mark" ]; then
            echo "Error: $dir is the tree '$found', not '$mark'"
            exit 1
        fi
    elif [ -e "$dir" ] && [ -n "$(ls -A "$dir")" ]; then
        parent=$(dirname "$(canonical_path "$dir")")
        found=$(cat "$parent/$TREE_MARKER" 2>/dev/null || true)
        if [ -e "$dir/.git" ] || { [ "$parent" != "$(canonical_path "$ROOT/build/$PROFILE")" ] \
            && [ "${found%% *}" != "$PROFILE" ]; }; then
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

# fex_ref_commit: print the commit fex-src.ref names in FEX_REPO: the ref
# itself when it is a full commit hash, else the tag (peeled, for an
# annotated one) or branch of that name, preferring a tag as git does.
# Prints nothing when the ref cannot be resolved.
fex_ref_commit() {
    local ref
    ref=$(tr -d '[:space:]' < "$SCRIPT_DIR/fex-src.ref")
    if [[ "$ref" =~ ^[0-9a-f]{40}$ ]]; then
        echo "$ref"
        return 0
    fi
    git ls-remote "$FEX_REPO" "$ref" 2>/dev/null | awk -v ref="$ref" '
        $2 == "refs/tags/" ref "^{}" { peeled = $1 }
        $2 == "refs/tags/" ref { tag = $1 }
        $2 == "refs/heads/" ref { head = $1 }
        END { if (peeled != "") print peeled; else if (tag != "") print tag; else if (head != "") print head }'
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
