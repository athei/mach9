#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MINGW_DIR="${MINGW_DIR:-/opt/llvm-mingw}"

# ARM64X link libraries for linking ARM64X PE builtins, built and staged but
# never installed or run.
ARM64X_BUILD_DIR="${ARM64X_BUILD_DIR:-$ROOT/build-arm64x}"
ARM64X_LIB_DIR="${ARM64X_LIB_DIR:-$ROOT/dist/wine-arm64x}"

usage() {
    echo "Usage: $0 [--clean] [--arm64]"
    exit 1
}

# Parse flags. --arm64 selects the native arm64 profile; without it the
# x86_64 Wine is built, as every release is.
CLEAN=0
ARCH_PROFILE=intel
while [ $# -gt 0 ]; do
    case "$1" in
        --clean) CLEAN=1; shift ;;
        --arm64) ARCH_PROFILE=arm64; shift ;;
        *) usage ;;
    esac
done

# shellcheck source=profile.sh
. "$SCRIPT_DIR/profile.sh"
set_profile "$ARCH_PROFILE"

if [ ! -f "$WINE_SRC/configure" ]; then
    echo "Error: Wine source not found at $WINE_SRC"
    exit 1
fi

if [ ! -x "$MINGW_DIR/bin/x86_64-w64-mingw32-clang" ]; then
    echo "Error: llvm-mingw not found at $MINGW_DIR"
    exit 1
fi
export PATH="$PATH:$MINGW_DIR/bin"

if [ ! -x "$MINGW_DIR/bin/arm64ec-w64-mingw32-clang" ]; then
    echo "Error: llvm-mingw at $MINGW_DIR has no arm64ec toolchain"
    exit 1
fi

# The floor is pinned in profile.sh, which exports it as
# MACOSX_DEPLOYMENT_TARGET for everything built below. The SDK stays whatever
# the host has; only the minimum is fixed.
check_sdk_floor
check_deps_formulae

# Clean if requested
if [ "$CLEAN" -eq 1 ]; then
    echo "==> Cleaning build directory..."
    remove_tree "$BUILD_DIR"
fi
mkdir -p "$BUILD_DIR"

# Every optional dependency is switched off unless it is switched back on
# here, so a package missing from the dependency prefix fails configure
# instead of silently changing the feature set. A profile adds the options
# only its Wine tree knows through CONFIGURE_FEATURES_EXTRA; together they
# cover every --with-*/--without-* package of `configure --help`, except:
# opengl, which stays at configure's default (never --with-opengl: on macOS
# it starts an EGL probe that always fails, and the Mac driver links
# -framework OpenGL on its own); pthread, which is required; gettextpo,
# which is off unless asked for (--without-gettextpo would only skip a header
# probe and change config.h for nothing); and the options that name a path
# rather than a package. The X extensions are moot under --without-x and
# denied anyway, so nothing depends on that.
CONFIGURE_ALLOWLIST=(
    --with-coreaudio
    --with-freetype
    --with-gnutls
    --with-mingw
    --with-opencl
    --with-sdl
    --without-alsa
    --without-capi
    --without-cups
    --without-dbus
    --without-ffmpeg
    --without-fontconfig
    --without-gettext
    --without-gphoto
    --without-gssapi
    --without-gstreamer
    --without-hwloc
    --without-inotify
    --without-krb5
    --without-netapi
    --without-oss
    --without-pcap
    --without-pcsclite
    --without-pulse
    --without-sane
    --without-udev
    --without-usb
    --without-v4l2
    --without-vulkan
    --without-wayland
    --without-x
    --without-xcomposite
    --without-xcursor
    --without-xfixes
    --without-xinerama
    --without-xinput
    --without-xinput2
    --without-xrandr
    --without-xrender
    --without-xshape
    --without-xshm
    --without-xxf86vm
)

# Configure (skip if already configured unless --clean was passed)
cd "$BUILD_DIR"
if [ "$CLEAN" -eq 1 ] || [ ! -f "$BUILD_DIR/Makefile" ]; then
    echo "==> Configuring Wine ($PROFILE)..."
    "${CONFIGURE_WRAPPER[@]}" "$WINE_SRC/configure" \
        --enable-archs="$WINE_ARCHS" \
        "${CONFIGURE_ALLOWLIST[@]}" \
        "${CONFIGURE_FEATURES_EXTRA[@]}" \
        "${CONFIGURE_TOOLCHAIN[@]}"
else
    echo "==> Skipping configure (already configured, use --clean to reconfigure)"
fi

# Patch sonames for relocatable bundle. config.status regenerates config.h
# whenever configure changes (e.g. after a source update), silently reverting
# the patch, so check before every build, not just on --clean, and re-check
# after make in case the build itself triggered a regeneration.
#
# Patched are the defines of the libraries the --with-* packages above load
# by soname, and only those, so the result does not depend on what else the
# host has installed. configure also probes a few libraries it has no
# --without-* for; when it does not find one it writes a placeholder define
# (FALLBACK_SONAMES), which stays as it is. Any other define means configure
# found a library the allowlist does not account for, and stops the build.
BUNDLED_SONAMES="SONAME_LIBFREETYPE SONAME_LIBGNUTLS SONAME_LIBSDL2"
FALLBACK_SONAMES="SONAME_LIBODBC=libodbc.dylib"
CONFIG_H="$BUILD_DIR/include/config.h"
soname_value() { # <define>: its string value in config.h, or nothing
    sed -n "s|^#define $1 \"\\(.*\\)\"\$|\\1|p" "$CONFIG_H"
}
check_soname_defines() {
    local def value
    for def in $BUNDLED_SONAMES; do
        if [ -z "$(soname_value "$def")" ]; then
            echo "Error: config.h has no $def; configure did not find the library"
            exit 1
        fi
    done
    for def in $(sed -n 's|^#define \(SONAME_LIB[A-Z0-9_]*\) .*|\1|p' "$CONFIG_H"); do
        case " $BUNDLED_SONAMES " in *" $def "*) continue ;; esac
        value=$(soname_value "$def")
        case " $FALLBACK_SONAMES " in *" $def=$value "*) continue ;; esac
        echo "Error: configure found $value for $def, which the allowlist does not"
        echo "account for; uninstall it from $DEPS_PREFIX or add it to the allowlist"
        exit 1
    done
}
unpatched_sonames() {
    local def value
    for def in $BUNDLED_SONAMES; do
        value=$(soname_value "$def")
        case "$value" in
            lib*) echo "$value" ;;
        esac
    done
}
patch_sonames() {
    local name
    echo "==> Patching sonames in config.h for @loader_path relocation..."
    for name in $(unpatched_sonames); do
        echo "    $name"
        sed -i '' "s|\"${name//./\\.}\"|\"@loader_path/../../external/$name\"|" "$CONFIG_H"
    done
    if [ -n "$(unpatched_sonames)" ]; then
        echo "Error: config.h still names these libraries bare after patching:"
        unpatched_sonames
        exit 1
    fi
}
check_soname_defines
if [ -n "$(unpatched_sonames)" ]; then
    patch_sonames
fi

# Build. make runs natively even for the x86_64 profile: clang is a universal
# binary and CC/CROSSCC already pin `-arch x86_64`, so putting make under
# `arch -x86_64` only means translating every compiler invocation. A clean
# build measured 642s that way against 208s this way, producing the same 2843
# artifacts at identical sizes. The tools the build generates for itself
# (winebuild, widl, wrc, makedep) are x86_64 there and get exec'd through
# Rosetta from this arm64 make, which works and is a small share of the time.
# Configure stays translated; that is where the host probing happens.
echo "==> Building Wine..."
make -j$(sysctl -n hw.ncpu)

if [ -n "$(unpatched_sonames)" ]; then
    echo "==> config.h was regenerated during the build, re-patching and rebuilding..."
    check_soname_defines
    patch_sonames
    make -j$(sysctl -n hw.ncpu)
fi

# The d3d9 test binaries, which bundle-wine.sh publishes so a consumer can run
# Wine's de-facto D3D9 conformance suite against its own d3d9 builtin without a
# Wine build tree of its own. The toplevel `all` happens to produce them today,
# but only as a side effect of programs/winetest embedding every test binary as
# a resource; naming the target keeps a bundle input a stated dependency instead
# of a by-product, and costs nothing when they are already built.
# `dlls/d3d9/tests/all` depends on exactly the two per-arch `d3d9_test.exe`.
if [ "$WITH_D3D9_TESTS" -eq 1 ]; then
    echo "==> Building the d3d9 test binaries..."
    make -j$(sysctl -n hw.ncpu) dlls/d3d9/tests/all
fi

# Verify
echo "==> Build complete."
file "$BUILD_DIR/loader/wine"
"$BUILD_DIR/loader/wine" --version 2>/dev/null || true

# ── Link-library trees and the Wine SDK ──────────────────────────────────
# PE code built outside Wine (DXMT, mtld3d) links against Wine's
# libwinecrt0.a, for the unix_lib.o its unix calls go through, and imports
# from libntdll.a (DXMT also from libdbghelp.a), each for the architecture of
# that code. A link-library tree is a second configure of the Wine source for
# just the PE architectures a build lacks, of which only those archives are
# built: no unix dependency is wanted, so every package is switched off.
# Configure still probes the host side, but runs natively: the tree is arm64
# in host and target.
LINK_LIB_WITHOUT=(
    --without-alsa --without-capi --without-coreaudio --without-cups
    --without-dbus --without-ffmpeg --without-fontconfig --without-freetype
    --without-gettext --without-gnutls --without-gphoto --without-gssapi
    --without-gstreamer --without-hwloc --without-inotify --without-krb5
    --without-netapi --without-opencl --without-oss --without-pcap
    --without-pcsclite --without-pulse --without-sane --without-sdl
    --without-udev --without-usb --without-v4l2
    --without-vulkan --without-wayland --without-x
)

# link_lib_tree <dir> <archs> <configure-arg>...: configure <dir> as a
# link-library tree for the PE architectures <archs>, once (again after
# --clean). The arguments add the options only this Wine source has and the
# toolchain.
link_lib_tree() {
    local dir="$1" archs="$2"
    shift 2
    if [ "$CLEAN" -eq 1 ]; then
        remove_tree "$dir"
    fi
    mkdir -p "$dir"
    if [ ! -f "$dir/Makefile" ]; then
        echo "==> Configuring the link library tree $dir ($archs)..."
        (cd "$dir" && "$WINE_SRC/configure" --enable-archs="$archs" --with-mingw \
            "${LINK_LIB_WITHOUT[@]}" "$@")
    else
        echo "==> Skipping configure of $dir (already configured, use --clean to reconfigure)"
    fi
}

# link_libs <pe-dir>: print the paths, relative to a build tree, of the three
# archives for <pe-dir>. Upstream Wine builds winecrt0 under libs/, the
# CrossOver tree under dlls/.
link_libs() {
    local crt0=dlls
    if [ -d "$WINE_SRC/libs/winecrt0" ]; then
        crt0=libs
    fi
    echo "$crt0/winecrt0/$1/libwinecrt0.a dlls/ntdll/$1/libntdll.a dlls/dbghelp/$1/libdbghelp.a"
}

# check_arm64x_archive <libwinecrt0.a>: exit unless the archive carries both
# halves an ARM64X link takes from it. A missing EC half means the ARM64X
# pairing did not happen and the archive holds ARM64 code only; a missing
# ARM64 half means the archive is not the paired one at all.
check_arm64x_archive() {
    local members member
    members="$("$MINGW_DIR/bin/llvm-ar" t "$1")"
    for member in arm64ec-windows/unix_lib.o aarch64-windows/unix_lib.o; do
        grep -q "$member" <<<"$members" \
            || { echo "Error: $1 carries no $member"; exit 1; }
    done
}

# The Wine SDK of the arm64 build: one directory, in the layout of an
# installed Wine, that DXMT's PE build (-Dwine_install_path) and mtld3d
# (WINE_SDK, and WINE_SDK_ARM64X for its ARM64X build) read. bin/winebuild
# and the aarch64-windows and i386-windows archives come from the arm64 build
# itself: its aarch64-windows archives are ARM64X, since arm64ec is paired
# with aarch64 there, which is why the arm64 profile needs no ARM64X side
# tree. The x86_64-windows archives are the one thing that build cannot make
# (ARM64EC is not x64, and an x64 object does not link against it), so a
# link-library tree configured for x86_64 alone, reusing the build's tools,
# makes them. WINE_COMMIT names the source commit, which build-dxmt.sh and
# bundle-wine.sh compare.
if [ "$WITH_WINE_SDK" -eq 1 ]; then
    link_lib_tree "$X86LIBS_BUILD_DIR" x86_64 "${CONFIGURE_FEATURES_EXTRA[@]}" \
        --host="$HOST_TRIPLE" --with-wine-tools="$BUILD_DIR" \
        CC="clang -arch arm64" --disable-tests BISON="$DEPS_PREFIX/opt/bison/bin/bison"
    # shellcheck disable=SC2046 # one word per target
    make -C "$X86LIBS_BUILD_DIR" -j"$(sysctl -n hw.ncpu)" $(link_libs x86_64-windows)

    echo "==> Staging the Wine SDK into $WINE_SDK_DIR..."
    remove_tree "$WINE_SDK_DIR"
    mkdir -p "$WINE_SDK_DIR/bin"
    cp "$BUILD_DIR/tools/winebuild/winebuild" "$WINE_SDK_DIR/bin/"
    for pe in "$PE64_DIR" i386-windows x86_64-windows; do
        tree="$BUILD_DIR"
        if [ "$pe" = x86_64-windows ]; then
            tree="$X86LIBS_BUILD_DIR"
        fi
        mkdir -p "$WINE_SDK_DIR/lib/wine/$pe"
        for lib in $(link_libs "$pe"); do
            cp "$tree/$lib" "$WINE_SDK_DIR/lib/wine/$pe/"
        done
    done
    check_arm64x_archive "$WINE_SDK_DIR/lib/wine/$PE64_DIR/libwinecrt0.a"
    wine_src_commit > "$WINE_SDK_DIR/WINE_COMMIT"
    echo "==> Wine SDK staged for $(cat "$WINE_SDK_DIR/WINE_COMMIT")."
fi

# FEX, the x86 emulator of the arm64 Wine, and DXMT, whose unix half links
# against this Wine's winemac.so and ntdll.so and whose PE half links against
# the SDK, so both are built after them. Both scripts read the same profile
# and take --clean the same way.
SUB_FLAGS=()
if [ "$CLEAN" -eq 1 ]; then
    SUB_FLAGS=(--clean)
fi
if [ "$WITH_FEX" -eq 1 ]; then
    "$SCRIPT_DIR/build-fex.sh" "${SUB_FLAGS[@]}"
fi
if [ "$WITH_DXMT_BUILD" -eq 1 ]; then
    "$SCRIPT_DIR/build-dxmt.sh" "${SUB_FLAGS[@]}"
fi

# The ARM64X link libraries for the x86_64 profile, whose Wine has no arm64
# PE side: a link-library tree for arm64ec,aarch64. The output is two link
# archives, not a Wine: nothing here is runnable and none of it is
# installed. An ARM64X PE builtin needs a `libwinecrt0.a` to take its
# `unix_lib.o` from and a `libntdll.a` to import from, each with an ARM64 and
# an ARM64EC member, and that is all this produces. The Wine that eventually
# loads such a builtin is CrossOver's, not this one, and it ships no link
# archives of its own.
#
# arm64ec is paired with aarch64 so makedep sets up ARM64X (`native_archs` /
# `hybrid_archs`): the pair emits ONE set of libraries under `aarch64-windows`
# carrying both arches' objects, which is also where the loader looks, since
# `get_pe_dir` knows no arm64ec directory and redirects a hybrid module
# requested as AMD64 to the ARM64 one. arm64ec alone would build a standalone
# `arm64ec-windows` tree that no loader ever searches.
if [ "$WITH_ARM64X" -eq 1 ]; then
    echo "==> Building the ARM64X link libraries..."
    link_lib_tree "$ARM64X_BUILD_DIR" arm64ec,aarch64 --without-unwind --host=aarch64-apple-darwin
    make -C "$ARM64X_BUILD_DIR" -j"$(sysctl -n hw.ncpu)" \
        dlls/winecrt0/aarch64-windows/libwinecrt0.a \
        dlls/ntdll/aarch64-windows/libntdll.a

    # Staged in the layout of an installed Wine, so a consumer can point its
    # WINE_SDK at this directory and find the libraries where it expects them.
    echo "==> Staging the ARM64X link libraries into $ARM64X_LIB_DIR ..."
    mkdir -p "$ARM64X_LIB_DIR/lib/wine/aarch64-windows"
    cp "$ARM64X_BUILD_DIR/dlls/winecrt0/aarch64-windows/libwinecrt0.a" \
       "$ARM64X_BUILD_DIR/dlls/ntdll/aarch64-windows/libntdll.a" \
       "$ARM64X_LIB_DIR/lib/wine/aarch64-windows/"
    check_arm64x_archive "$ARM64X_LIB_DIR/lib/wine/aarch64-windows/libwinecrt0.a"
    echo "==> ARM64X link libraries staged."
fi
