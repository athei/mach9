#!/bin/bash
set -e

# Builds deqp-vk, the Vulkan conformance test program of Khronos' VK-GL-CTS,
# for this Mac, and stages it in VK_GL_CTS_DIST for the deqp-vk suite of the
# conformance runner (docs/internals.md, deqp-vk suite):
#
#   deqp-vk         the test program
#   vulkan/         its data archive, which it reads relative to the
#                   directory --deqp-archive-dir names
#   mustpass.txt    every case of the vk-default mustpass, one per line, the
#                   case list the runner selects its groups from
#   KEY             what the dist was built from (dist_key in profile.sh)
#   COMMIT          VK-GL-CTS's source commit
#
# VK-GL-CTS comes from the tag redist.env pins, the Vulkan CTS release Mesa's
# CI builds deqp-vk from. Its own script, external/fetch_sources.py, fetches
# the sources it builds on (SPIRV-Tools, glslang, zlib, libpng and others) at
# the commits and checksums that script pins, so the tag fixes them too.
#
# deqp-vk is built for the host, not the floor: it runs here and is never
# bundled. It loads the Vulkan loader at run time, by the path the runner
# passes with --deqp-vk-library-path, and links nothing from Homebrew; the
# Homebrew prefix is kept out of CMake's search, and the stage step checks
# what it links. Not part of build-wine.sh, and the bundle does not carry
# it: the suite runs it against a bundle's or MESA_DIST's KosmicKrisp.
#
# Nothing is built when VK_GL_CTS_DIST is complete and has the KEY this run
# would write (skip_current_dist in profile.sh). --clean builds the tree and
# VK_GL_CTS_DIST from scratch; the clone and the fetched sources are kept
# while the pin is unchanged. --print-key prints that KEY and exits
# (print_dist_key).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

CLEAN=0
PRINT_KEY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --clean) CLEAN=1; shift ;;
        --print-key) PRINT_KEY=1; shift ;;
        *) echo "Usage: $0 [--clean | --print-key]"; exit 1 ;;
    esac
done

# shellcheck source=lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"
# shellcheck source=../pins/redist.env
. "$PINS_DIR/redist.env"

# The tree: the clone with the sources fetch_sources.py adds, and the CMake
# tree. Set here rather than in profile.sh, which every component's KEY
# hashes, since only this script and the runner use them; the runner takes
# the same default.
VK_GL_CTS_BUILD_DIR="${VK_GL_CTS_BUILD_DIR:-$BUILD_TOP/vk-gl-cts}"
VK_GL_CTS_DIST="${VK_GL_CTS_DIST:-$VK_GL_CTS_BUILD_DIR/dist}"
CTS_SRC="$VK_GL_CTS_BUILD_DIR/src"
CTS_BUILD="$VK_GL_CTS_BUILD_DIR/build"
MODULE_DIR=external/vulkancts/modules/vulkan
MUSTPASS_DIR=external/vulkancts/mustpass/main

if [ -z "$VK_GL_CTS_GIT_COMMIT" ]; then
    echo "Error: redist.env pins no VK_GL_CTS_GIT_COMMIT"
    exit 1
fi
for tool in git cmake ninja python3; do
    if ! command -v "$tool" >/dev/null; then
        echo "Error: $tool not found (brew install $tool)"
        exit 1
    fi
done

# ── Dist ────────────────────────────────────────────────────────────────
# vk_gl_cts_key <commit>: the KEY of a VK_GL_CTS_DIST built from <commit>.
# Besides the commit, which fixes what fetch_sources.py fetches, what builds
# it: Apple's clang and the macOS SDK, CMake, and the Python that runs the
# CTS's generators.
vk_gl_cts_key() {
    dist_key "$SCRIPT_DIR/build-vk-gl-cts.sh" "vk-gl-cts=$1" \
        "cc=$(cc_version /usr/bin/clang)" "$(sdk_key)" \
        "cmake=$(cmake --version | head -n 1)" \
        "python=$(python3 -c 'import platform; print(platform.python_version())')"
}
print_dist_key "$VK_GL_CTS_GIT_COMMIT" redist.env vk_gl_cts_key

claim_tree "$VK_GL_CTS_BUILD_DIR" vk-gl-cts
if [ "$CLEAN" -eq 1 ]; then
    echo "==> Cleaning the deqp-vk tree and $VK_GL_CTS_DIST..."
    remove_tree "$CTS_BUILD"
fi
# VK_GL_CTS_DIST is inside VK_GL_CTS_BUILD_DIR unless overridden, so it is
# claimed on its own, and so is VK_GL_CTS_DIST.part below.
claim_tree "$VK_GL_CTS_DIST" vk-gl-cts-dist "$CLEAN"
skip_current_dist "$VK_GL_CTS_DIST" VK-GL-CTS "$VK_GL_CTS_GIT_COMMIT" vk_gl_cts_key

# ── Source ──────────────────────────────────────────────────────────────
# VK-GL-CTS has no submodules; fetch_sources.py puts each source it builds
# on into external/<name>/src, and fetches only what is not there at its
# pinned commit or checksum yet.
echo "==> VK-GL-CTS source: $VK_GL_CTS_GIT_URL $VK_GL_CTS_GIT_TAG"
pinned_clone "$VK_GL_CTS_GIT_URL" "$VK_GL_CTS_GIT_TAG" "$VK_GL_CTS_GIT_COMMIT" "$CTS_SRC" --no-submodules
echo "==> Fetching the sources VK-GL-CTS builds on..."
GIT_TERMINAL_PROMPT=0 python3 "$CTS_SRC/external/fetch_sources.py"

# ── Build ───────────────────────────────────────────────────────────────
# The osx target, deqp-vk only. Apple's compilers named explicitly, through
# WINE_CCACHE when WINE_BUILD_CCACHE is set (profile.sh); the empty launcher
# clears one an earlier run set. The Homebrew prefix is ignored in every
# find_* call, so the build takes zlib and libpng from fetch_sources.py or
# the SDK and never a Homebrew library.
echo "==> Building deqp-vk..."
(
    unset MACOSX_DEPLOYMENT_TARGET CC CXX CFLAGS CXXFLAGS CPPFLAGS LDFLAGS
    cmake -S "$CTS_SRC" -B "$CTS_BUILD" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DDEQP_TARGET=osx \
        -DSELECTED_BUILD_TARGETS=deqp-vk \
        -DCMAKE_C_COMPILER=/usr/bin/clang -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
        -DCMAKE_OSX_ARCHITECTURES=arm64 \
        -DCMAKE_IGNORE_PREFIX_PATH="$DEPS_PREFIX;/usr/local" \
        -DCMAKE_C_COMPILER_LAUNCHER="$WINE_CCACHE" -DCMAKE_CXX_COMPILER_LAUNCHER="$WINE_CCACHE" \
        >/dev/null
    ninja -C "$CTS_BUILD" deqp-vk
)

# ── Stage ───────────────────────────────────────────────────────────────
# Into VK_GL_CTS_DIST.part, renamed into place once everything is checked,
# so an interrupted run never leaves a dist behind that looks complete.
echo "==> Staging deqp-vk into $VK_GL_CTS_DIST..."
part="$VK_GL_CTS_DIST.part"
claim_tree "$part" vk-gl-cts-dist 1
cp "$CTS_BUILD/$MODULE_DIR/deqp-vk" "$part/"
cp -R "$CTS_BUILD/$MODULE_DIR/vulkan" "$part/"
# vk-default.txt lists the files of the mustpass, one per line, in the order
# the CTS runs them.
while read -r list; do
    [ -n "$list" ] || continue
    cat "$CTS_SRC/$MUSTPASS_DIR/$list"
done < "$CTS_SRC/$MUSTPASS_DIR/vk-default.txt" > "$part/mustpass.txt"
if ! grep -q '^dEQP-VK\.draw\.' "$part/mustpass.txt"; then
    echo "Error: the staged mustpass.txt has no dEQP-VK.draw case"
    exit 1
fi

# deqp-vk opens the Vulkan loader itself, by the path the runner gives it,
# so it must link no loader, and nothing outside the system either.
deps=$(otool -L "$part/deqp-vk" | tail -n +2 | awk '{print $1}' \
    | grep -vE '^(/usr/lib/|/System/Library/)' || true)
if [ -n "$deps" ] || otool -L "$part/deqp-vk" | grep -q libvulkan; then
    echo "Error: deqp-vk links a Vulkan loader or libraries outside the system:"
    otool -L "$part/deqp-vk" | tail -n +2
    exit 1
fi
echo "    deqp-vk: $(lipo -archs "$part/deqp-vk"), links system libraries only," \
     "$(wc -l < "$part/mustpass.txt" | tr -d ' ') mustpass cases"

vk_gl_cts_key "$VK_GL_CTS_GIT_COMMIT" > "$part/KEY"
# Written last, so a staged directory without it is an incomplete one.
echo "$VK_GL_CTS_GIT_COMMIT" > "$part/COMMIT"
remove_tree "$VK_GL_CTS_DIST"
mv "$part" "$VK_GL_CTS_DIST"
echo "==> VK-GL-CTS $VK_GL_CTS_GIT_TAG ($VK_GL_CTS_GIT_COMMIT) staged in $VK_GL_CTS_DIST."
