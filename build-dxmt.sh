#!/bin/bash
set -e

# Builds DXMT for the arm64 bundle from the upstream commit pinned in
# redist.env and stages it in DXMT_DIST, in the layout of DXMT's release
# tarball:
#
#   x86_64-windows/{dxgi,d3d10core,d3d11,winemetal}.dll
#   i386-windows/{dxgi,d3d10core,d3d11,winemetal}.dll
#   aarch64-unix/winemetal.so
#   COMMIT       the source commit
#   DESCRIBE     its `git describe`, the version string DXMT builds into d3d11.dll
#   WINE_COMMIT  the Wine source commit it was linked against
#
# Called by build-wine.sh --arm64, after the Wine build and the Wine SDK,
# which this links against. bundle-wine.sh --arm64 installs from DXMT_DIST.
#
# The Windows half is x86 code, x86_64 and i386, built with DXMT's own
# build-win64.txt and build-win32.txt cross files and llvm-mingw against the
# Wine SDK (-Dwine_install_path): its libwinecrt0.a, libntdll.a, libdbghelp.a
# and winebuild. Games are x86 programs and run under FEX, and these DLLs run
# in the same emulated code; an ARM64EC build of DXMT would be a different
# DXMT. Only the DLLs and the targets that mark them as Wine builtins are
# built there: the default targets include an x86_64 winemetal.so, which
# cannot link against this Wine's arm64 libraries.
#
# The unix half has to be an arm64 winemetal.so. DXMT builds it as a native
# target of its meson project, with the architecture taken from the cross
# file: -arch arm64 only under build-arm64ec.txt. So a third tree is
# configured with that cross file against the Wine build tree, and only
# winemetal.so is built there; none of its ARM64EC PE targets are. All three
# trees use one checkout, so the DLLs and the .so come from one commit, which
# their private call interface needs.
#
# winemetal.so needs LLVM 15: it links airconv_dep_darwin, the static airconv
# library (DXMT's DXBC to Metal shader converter) that
# src/airconv/darwin/meson.build builds natively against native_llvm_path.
# The PE side only takes airconv's headers (airconv_forward_dep) unless
# build_airconv_for_windows is set, which it is not, but its configure still
# requires native_llvm_path to exist, so all three trees point at DXMT_LLVM.
# There is no arm64 LLVM 15 to install, so its static libraries are built
# from the pinned tag into DXMT_LLVM, following Hadron's recipe, and reused
# while the pin stays.
#
# --clean wipes the DXMT checkout, trees and DXMT_DIST first (not DXMT_LLVM,
# which takes long to build and does not depend on anything here).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MINGW_DIR="${MINGW_DIR:-/opt/llvm-mingw}"

CLEAN=0
while [ $# -gt 0 ]; do
    case "$1" in
        --clean) CLEAN=1; shift ;;
        *) echo "Usage: $0 [--clean]"; exit 1 ;;
    esac
done

# DXMT is only built for the arm64 profile, which names the Wine build tree
# and SDK to link against, DXMT_BUILD_DIR, DXMT_LLVM, DXMT_DIST and the
# floor.
# shellcheck source=profile.sh
. "$SCRIPT_DIR/profile.sh"
set_profile arm64
# shellcheck source=redist.env
. "$SCRIPT_DIR/redist.env"

DXMT_SRC="$DXMT_BUILD_DIR/src"
PE_DLLS="dxgi d3d10core d3d11 winemetal"

# ── Preflight ───────────────────────────────────────────────────────────
for tool in git meson ninja cmake xxd; do
    if ! command -v "$tool" >/dev/null; then
        echo "Error: $tool not found (brew install $tool)"
        exit 1
    fi
done
if ! xcrun -sdk macosx -f metal >/dev/null 2>&1; then
    echo "Error: no Metal compiler (xcrun metal); install Xcode and its Metal toolchain"
    exit 1
fi
if [ ! -f "$BUILD_DIR/Makefile" ]; then
    echo "Error: no arm64 Wine build at $BUILD_DIR (run build-wine.sh --arm64 first)"
    exit 1
fi
# winemetal.so links the two .so files; its configure also looks up the PE
# link libraries for the ARM64EC winemetal.dll it never builds, and fails
# without them.
for f in dlls/winemac.drv/winemac.so dlls/ntdll/ntdll.so \
         "dlls/ntdll/$PE64_DIR/libntdll.a" "dlls/dbghelp/$PE64_DIR/libdbghelp.a"; do
    if [ ! -f "$BUILD_DIR/$f" ]; then
        echo "Error: $BUILD_DIR/$f not found (run build-wine.sh --arm64 first)"
        exit 1
    fi
done
# Upstream Wine builds winecrt0 under libs/, older trees under dlls/; DXMT's
# meson looks in both.
if [ ! -f "$BUILD_DIR/libs/winecrt0/$PE64_DIR/libwinecrt0.a" ] \
    && [ ! -f "$BUILD_DIR/dlls/winecrt0/$PE64_DIR/libwinecrt0.a" ]; then
    echo "Error: no $PE64_DIR/libwinecrt0.a under $BUILD_DIR/libs or dlls (run build-wine.sh --arm64 first)"
    exit 1
fi
# The SDK must be the one staged from this Wine source, or the DLLs would
# link against another Wine's unix call setup.
WINE_COMMIT=$(wine_src_commit)
if [ "$(cat "$WINE_SDK_DIR/WINE_COMMIT" 2>/dev/null)" != "$WINE_COMMIT" ]; then
    echo "Error: the Wine SDK at $WINE_SDK_DIR is for $(cat "$WINE_SDK_DIR/WINE_COMMIT" 2>/dev/null || echo nothing),"
    echo "but $WINE_SRC is at $WINE_COMMIT (run build-wine.sh --arm64)"
    exit 1
fi
for f in bin/winebuild lib/wine/x86_64-windows/libwinecrt0.a lib/wine/i386-windows/libwinecrt0.a; do
    if [ ! -f "$WINE_SDK_DIR/$f" ]; then
        echo "Error: the Wine SDK at $WINE_SDK_DIR has no $f (run build-wine.sh --arm64)"
        exit 1
    fi
done

# Appended, never prepended: llvm-mingw's bin/ has an unprefixed clang that
# would otherwise replace Apple's as the native compiler. DXMT's cross files
# name the tools without a path, so check that each resolves into
# llvm-mingw and not to some other mingw earlier in PATH.
export PATH="$PATH:$MINGW_DIR/bin"
for triple in x86_64-w64-mingw32 i686-w64-mingw32 arm64ec-w64-mingw32; do
    for tool in gcc g++ ar strip windres; do
        if [ "$(command -v "$triple-$tool")" != "$MINGW_DIR/bin/$triple-$tool" ]; then
            echo "Error: $triple-$tool does not resolve to $MINGW_DIR/bin"
            echo "  (found: $(command -v "$triple-$tool" || echo nothing))"
            exit 1
        fi
    done
done

# The checkouts and meson trees this script deletes and recreates are fixed
# paths in DXMT_BUILD_DIR. DXMT_LLVM and DXMT_DIST are inside it unless
# overridden, so they are claimed on their own, and so is DXMT_LLVM.part
# below.
claim_tree "$DXMT_BUILD_DIR" dxmt
claim_tree "$DXMT_LLVM" dxmt-llvm
if [ "$CLEAN" -eq 1 ]; then
    echo "==> Cleaning the DXMT trees..."
    remove_tree "$DXMT_SRC" "$DXMT_BUILD_DIR/win64" "$DXMT_BUILD_DIR/win32" \
                "$DXMT_BUILD_DIR/unix"
fi
claim_tree "$DXMT_DIST" dxmt-dist "$CLEAN"

# ── LLVM 15 ─────────────────────────────────────────────────────────────
# Only the libraries airconv links: no targets, no tools. The libc++ define
# and the other options are Hadron's, which builds LLVM 15 with the same
# SDK; zstd is off so the result needs nothing from Homebrew. DXMT_LLVM/COMMIT
# records the commit they were built from; libraries from another one (a
# bumped pin), or none, are rebuilt.
if [ "$(cat "$DXMT_LLVM/COMMIT" 2>/dev/null)" != "$LLVM15_GIT_COMMIT" ]; then
    echo "==> Building LLVM 15 static libraries into $DXMT_LLVM..."
    remove_tree "$DXMT_LLVM" "$DXMT_BUILD_DIR/llvm-build"
    pinned_clone "$LLVM15_GIT_URL" "$LLVM15_GIT_TAG" "$LLVM15_GIT_COMMIT" "$DXMT_BUILD_DIR/llvm-project"
    cmake -S "$DXMT_BUILD_DIR/llvm-project/llvm" -B "$DXMT_BUILD_DIR/llvm-build" -G Ninja \
        -DCMAKE_C_COMPILER=/usr/bin/clang -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
        -DCMAKE_INSTALL_PREFIX="$DXMT_LLVM" \
        -DCMAKE_OSX_ARCHITECTURES=arm64 \
        -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_FLOOR" \
        -DLLVM_HOST_TRIPLE=arm64-apple-darwin \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_CXX_FLAGS="-D_LIBCPP_KEEP_TRANSITIVE_INCLUDES_LLVM23" \
        -DLLVM_ENABLE_ASSERTIONS=On \
        -DLLVM_ENABLE_ZSTD=Off \
        -DLLVM_TARGETS_TO_BUILD="" \
        -DLLVM_BUILD_TOOLS=Off \
        -DLLVM_INCLUDE_BENCHMARKS=Off \
        -DLLVM_INCLUDE_TESTS=Off \
        -DLLVM_INCLUDE_EXAMPLES=Off \
        -DLLVM_VERSION_PRINTER_SHOW_HOST_TARGET_INFO=Off >/dev/null
    cmake --build "$DXMT_BUILD_DIR/llvm-build"
    # Installed next to DXMT_LLVM and renamed into place, so an interrupted
    # install never leaves a DXMT_LLVM behind.
    claim_tree "$DXMT_LLVM.part" dxmt-llvm 1
    cmake --install "$DXMT_BUILD_DIR/llvm-build" --prefix "$DXMT_LLVM.part" >/dev/null
    echo "$LLVM15_GIT_COMMIT" > "$DXMT_LLVM.part/COMMIT"
    mv "$DXMT_LLVM.part" "$DXMT_LLVM"
    # The source and build trees are large and no longer needed.
    remove_tree "$DXMT_BUILD_DIR/llvm-build" "$DXMT_BUILD_DIR/llvm-project"
fi

# ── DXMT source ─────────────────────────────────────────────────────────
# A full clone, not a shallow one: `git describe` needs the history back to
# the last tag. An existing clone at another commit (a bumped pin) is
# replaced; it is this script's own scratch copy.
echo "==> DXMT source: $DXMT_GIT_URL $DXMT_GIT_COMMIT"
if [ -e "$DXMT_SRC/.git" ] && [ "$(git -C "$DXMT_SRC" rev-parse HEAD)" != "$DXMT_GIT_COMMIT" ]; then
    echo "    $DXMT_SRC is at another commit; cloning again"
    remove_tree "$DXMT_SRC"
fi
if [ ! -e "$DXMT_SRC/.git" ]; then
    git clone --quiet "$DXMT_GIT_URL" "$DXMT_SRC"
    git -C "$DXMT_SRC" -c advice.detachedHead=false checkout --quiet --detach "$DXMT_GIT_COMMIT"
fi
git -C "$DXMT_SRC" submodule update --quiet --init --recursive
DXMT_DESCRIBE=$(git -C "$DXMT_SRC" describe --always)
echo "    $DXMT_DESCRIBE"

# ── Meson trees ─────────────────────────────────────────────────────────
# dxmt_setup <dir> <cross-file> <option>...: configure a DXMT tree, again
# from scratch whenever what it links against differs from what it was
# configured with: the DXMT commit, the Wine commit (the SDK archives and
# winemac.so and ntdll.so come from it), the LLVM commit, or the options. A
# fresh tree relinks everything, which ninja would not do on its own for a
# library that changed outside it. A Wine source with uncommitted changes is
# one state ("-dirty") however those changes move; rebuild with --clean
# after changing it.
dxmt_setup() {
    local dir="$1" cross="$2" key
    shift 2
    key="$DXMT_GIT_COMMIT wine=$WINE_COMMIT llvm=$(cat "$DXMT_LLVM/COMMIT") $cross $*"
    if [ "$(cat "$dir/.setup-key" 2>/dev/null)" != "$key" ]; then
        remove_tree "$dir"
        meson setup "$dir" "$DXMT_SRC" --cross-file "$DXMT_SRC/$cross" \
            --buildtype release -Dnative_llvm_path="$DXMT_LLVM" "$@" >/dev/null
        echo "$key" > "$dir/.setup-key"
    fi
}

# The PE DLLs and the custom targets that run `winebuild --builtin` on them.
PE_TARGETS=(src/dxgi/dxgi.dll.postproc src/d3d10/d3d10core.dll.postproc
            src/d3d11/d3d11.dll.postproc src/winemetal/winemetal.dll.postproc)

echo "==> Building DXMT's x86_64 PE DLLs..."
dxmt_setup "$DXMT_BUILD_DIR/win64" build-win64.txt -Dwine_install_path="$WINE_SDK_DIR"
ninja -C "$DXMT_BUILD_DIR/win64" "${PE_TARGETS[@]}"

echo "==> Building DXMT's i386 PE DLLs..."
dxmt_setup "$DXMT_BUILD_DIR/win32" build-win32.txt -Dwine_install_path="$WINE_SDK_DIR"
ninja -C "$DXMT_BUILD_DIR/win32" "${PE_TARGETS[@]}"

# The build-arm64ec.txt configure finds the ARM64EC link libraries in the
# arm64 Wine tree's aarch64-windows directories, and this one target pulls in
# only native ones (DXBCParserNative and the darwin airconv library).
echo "==> Building DXMT's arm64 winemetal.so..."
dxmt_setup "$DXMT_BUILD_DIR/unix" build-arm64ec.txt -Dwine_build_path="$BUILD_DIR"
ninja -C "$DXMT_BUILD_DIR/unix" src/winemetal/unix/winemetal.so

# ── Stage ───────────────────────────────────────────────────────────────
echo "==> Staging DXMT into $DXMT_DIST..."
claim_tree "$DXMT_DIST" dxmt-dist 1
mkdir -p "$DXMT_DIST/x86_64-windows" "$DXMT_DIST/i386-windows" "$DXMT_DIST/$UNIX_DIR"
for dll in $PE_DLLS; do
    case $dll in
        d3d10core) dir=d3d10 ;;
        *) dir=$dll ;;
    esac
    cp "$DXMT_BUILD_DIR/win64/src/$dir/$dll.dll" "$DXMT_DIST/x86_64-windows/"
    cp "$DXMT_BUILD_DIR/win32/src/$dir/$dll.dll" "$DXMT_DIST/i386-windows/"
done
cp "$DXMT_BUILD_DIR/unix/src/winemetal/unix/winemetal.so" "$DXMT_DIST/$UNIX_DIR/"

# A cross file that silently fell back to another compiler would still
# produce DLLs, only of the wrong machine; an ARM64EC DLL carries the x86_64
# machine too, which the tool check in the preflight rules out. Each DLL must
# also carry Wine's builtin marker, which the postproc targets put there.
for dll in $PE_DLLS; do
    for pe in x86_64-windows:8664 i386-windows:014c; do
        f="$DXMT_DIST/${pe%%:*}/$dll.dll"
        [ "$(pe_machine "$f")" = "${pe#*:}" ] \
            || { echo "Error: ${pe%%:*}/$dll.dll has the PE machine $(pe_machine "$f"), not ${pe#*:}"; exit 1; }
        head -c 128 "$f" | grep -qa "Wine builtin DLL" \
            || { echo "Error: ${pe%%:*}/$dll.dll is not marked as a Wine builtin"; exit 1; }
    done
done
if [ "$(lipo -archs "$DXMT_DIST/$UNIX_DIR/winemetal.so")" != arm64 ]; then
    echo "Error: $UNIX_DIR/winemetal.so is not an arm64-only Mach-O"
    exit 1
fi
# The DLLs carry the describe output as their version string; checked here so
# the record and the build cannot disagree.
if ! strings -a "$DXMT_DIST/x86_64-windows/d3d11.dll" | grep -qxF "$DXMT_DESCRIBE"; then
    echo "Error: d3d11.dll does not carry the version $DXMT_DESCRIBE"
    exit 1
fi
echo "$DXMT_DESCRIBE" > "$DXMT_DIST/DESCRIBE"
echo "$WINE_COMMIT" > "$DXMT_DIST/WINE_COMMIT"
# Written last, so a staged directory without it is an incomplete one.
git -C "$DXMT_SRC" rev-parse HEAD > "$DXMT_DIST/COMMIT"
echo "==> DXMT $DXMT_DESCRIBE staged."
