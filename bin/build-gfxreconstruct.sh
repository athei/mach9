#!/bin/bash
set -e

# Builds LunarG's GFXReconstruct for this Mac, native arm64 for the floor,
# and stages in GFXRECONSTRUCT_DIST what compatdb's vulkan_capture and the
# replay script use (docs/internals.md, Vulkan capture):
#
#   lib/libVkLayer_gfxreconstruct.dylib   the capture layer
#                   (VK_LAYER_LUNARG_gfxreconstruct), which the bundle step
#                   installs into lib/external
#   share/vulkan/explicit_layer.d/VkLayer_gfxreconstruct.json
#                   its manifest, which names the layer by a path relative
#                   to itself
#   bin/gfxrecon-replay, bin/gfxrecon-info
#                   the replay and the summary of a capture, which
#                   bench/gfxr-replay.sh runs; not in the bundle
#   share/licenses/gfxreconstruct.txt, gfxreconstruct-third-party.txt,
#   spirv-reflect.txt
#                   the licences of what the layer is built from, out of
#                   the pinned tree, which the bundle step installs
#   KEY             what the dist was built from (dist_key in profile.sh)
#   COMMIT          GFXReconstruct's source commit
#
# GFXReconstruct comes from the tag redist.env pins, the SDK release of the
# pinned Vulkan loader. The sources it builds on are the tag's submodules,
# so the tag fixes them too; only the three the build uses are fetched, and
# the script checks that their Vulkan-Headers are the ones redist.env pins.
# pins/gfxreconstruct-macos-exports.patch adds the functions win32u starts
# and stops a capture with to what the macOS layer exports, and the stage
# step checks that it exports them. Its compression libraries (LZ4,
# Zstandard, zlib) are the static archives the tag carries for macOS in
# external/precompiled/macos; the Homebrew prefix is kept out of CMake's
# search, and the stage step checks what the layer and the tools link.
# OpenXR is left out.
#
# The layer goes into the bundle, so it is built for the floor
# (MACOSX_DEPLOYMENT_TARGET, profile.sh) like every Mach-O there; the bundle
# step checks it again (check_minos). Nothing loads it unless compatdb.so
# names it for a process.
#
# Nothing is built when GFXRECONSTRUCT_DIST is complete and has the KEY
# this run would write (skip_current_dist in profile.sh). --clean builds the
# tree and the dist from scratch; the clone is kept while the pin is
# unchanged. --print-key prints that KEY and exits (print_dist_key).

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

# The tree: the clone and the CMake tree. Set here rather than in
# profile.sh, which every component's KEY hashes, as
# build-vulkan-validation-layers.sh does; bundle-wine.sh takes the same
# default.
GFXRECONSTRUCT_BUILD_DIR="${GFXRECONSTRUCT_BUILD_DIR:-$BUILD_TOP/gfxreconstruct}"
GFXRECONSTRUCT_DIST="${GFXRECONSTRUCT_DIST:-$GFXRECONSTRUCT_BUILD_DIR/dist}"
GFXR_SRC="$GFXRECONSTRUCT_BUILD_DIR/src"
GFXR_BUILD="$GFXRECONSTRUCT_BUILD_DIR/build"
LAYER=libVkLayer_gfxreconstruct.dylib
MANIFEST=VkLayer_gfxreconstruct.json
TOOLS="gfxrecon-replay gfxrecon-info"
# The submodules the build uses; OpenXR's are left out with OpenXR.
SUBMODULES="external/Vulkan-Headers external/SPIRV-Headers external/SPIRV-Reflect"
# The change to the pinned source, and the functions it exports.
PATCH="$PINS_DIR/gfxreconstruct-macos-exports.patch"
EXPORTS="GFXRSetTrimmingState GFXRGetCaptureMode"
# The licence texts of what the layer links, as <staged name>=<source in
# the tree>: GFXReconstruct's own, the third-party file that covers the
# static LZ4, Zstandard and zlib archives and the Vulkan-Headers it
# compiles in, and SPIRV-Reflect, which it links statically too.
LICENSES="gfxreconstruct.txt=LICENSES/LICENSE.txt
gfxreconstruct-third-party.txt=LICENSES/LICENSE_ThirdParty.txt
spirv-reflect.txt=external/SPIRV-Reflect/LICENSE"
# The sections the third-party file must have for the archives linked.
THIRD_PARTY_SECTIONS="LZ4 Library|Zstandard|zlib|Vulkan-Headers"

if [ -z "$GFXRECONSTRUCT_GIT_COMMIT" ]; then
    echo "Error: redist.env pins no GFXRECONSTRUCT_GIT_COMMIT"
    exit 1
fi
for tool in git cmake ninja; do
    if ! command -v "$tool" >/dev/null; then
        echo "Error: $tool not found (brew install $tool)"
        exit 1
    fi
done

# ── Dist ────────────────────────────────────────────────────────────────
# gfxr_key <commit>: the KEY of a dist built from <commit>. Besides the
# commit, which fixes the submodules and the precompiled archives, what
# builds it: Apple's clang and the macOS SDK, and CMake.
gfxr_key() {
    dist_key --hash "$PATCH" "$SCRIPT_DIR/build-gfxreconstruct.sh" "gfxreconstruct=$1" \
        "cc=$(cc_version /usr/bin/clang)" "$(sdk_key)" \
        "cmake=$(cmake --version | head -n 1)"
}
print_dist_key "$GFXRECONSTRUCT_GIT_COMMIT" redist.env gfxr_key

check_sdk_floor
claim_tree "$GFXRECONSTRUCT_BUILD_DIR" gfxreconstruct
if [ "$CLEAN" -eq 1 ]; then
    echo "==> Cleaning GFXReconstruct's tree and $GFXRECONSTRUCT_DIST..."
    remove_tree "$GFXR_BUILD"
fi
# The dist is inside the build directory unless overridden, so it is
# claimed on its own, and so is its .part below.
claim_tree "$GFXRECONSTRUCT_DIST" gfxreconstruct-dist "$CLEAN"
skip_current_dist "$GFXRECONSTRUCT_DIST" GFXReconstruct \
    "$GFXRECONSTRUCT_GIT_COMMIT" gfxr_key

# ── Source ──────────────────────────────────────────────────────────────
echo "==> GFXReconstruct source: $GFXRECONSTRUCT_GIT_URL $GFXRECONSTRUCT_GIT_TAG"
pinned_clone "$GFXRECONSTRUCT_GIT_URL" "$GFXRECONSTRUCT_GIT_TAG" \
    "$GFXRECONSTRUCT_GIT_COMMIT" "$GFXR_SRC" --no-submodules
# shellcheck disable=SC2086 # SUBMODULES is a list of paths without spaces
GIT_TERMINAL_PROMPT=0 git -C "$GFXR_SRC" submodule update --quiet --init --depth 1 -- $SUBMODULES
headers=$(git -C "$GFXR_SRC" rev-parse HEAD:external/Vulkan-Headers)
if [ "$headers" != "$VULKAN_HEADERS_GIT_COMMIT" ]; then
    echo "Error: GFXReconstruct $GFXRECONSTRUCT_GIT_TAG builds on Vulkan-Headers $headers,"
    echo "redist.env pins $VULKAN_HEADERS_GIT_COMMIT"
    exit 1
fi
# The macOS layer exports only the Vulkan entry points; PATCH adds the two
# functions win32u starts and stops a capture with, which the Linux layer
# exports already. The clone is reset first, so a kept clone gets the patch
# once, and a pin the patch no longer applies to stops here.
git -C "$GFXR_SRC" checkout --quiet -- .
if ! git -C "$GFXR_SRC" apply --check "$PATCH"; then
    echo "Error: $(basename "$PATCH") does not apply to GFXReconstruct $GFXRECONSTRUCT_GIT_TAG;"
    echo "drop it if the tag exports $EXPORTS, else update it"
    exit 1
fi
git -C "$GFXR_SRC" apply "$PATCH"

# ── Build ───────────────────────────────────────────────────────────────
# Apple's compilers named explicitly, through WINE_CCACHE when
# WINE_BUILD_CCACHE is set (profile.sh); the empty launcher clears one an
# earlier run set. The Homebrew prefix is ignored in every find_* call, so
# CMake takes the archives of external/precompiled/macos, which
# GFXReconstruct's CMakeLists.txt puts on its prefix path for Apple. The
# floor comes from MACOSX_DEPLOYMENT_TARGET, which profile.sh exports.
echo "==> Building GFXReconstruct..."
(
    unset CC CXX CFLAGS CXXFLAGS CPPFLAGS LDFLAGS
    GIT_TERMINAL_PROMPT=0 cmake -S "$GFXR_SRC" -B "$GFXR_BUILD" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_C_COMPILER=/usr/bin/clang -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
        -DCMAKE_OSX_ARCHITECTURES=arm64 \
        -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_FLOOR" \
        -DCMAKE_IGNORE_PREFIX_PATH="$DEPS_PREFIX;/usr/local" \
        -DCMAKE_C_COMPILER_LAUNCHER="$WINE_CCACHE" -DCMAKE_CXX_COMPILER_LAUNCHER="$WINE_CCACHE" \
        -DGFXRECON_ENABLE_OPENXR=OFF -DGFXRECON_TOCPP_SUPPORT=OFF \
        >/dev/null
    # shellcheck disable=SC2086 # TOOLS is a list of target names
    ninja -C "$GFXR_BUILD" VkLayer_gfxreconstruct VkLayer_gfxreconstruct_json $TOOLS
)

# ── Stage ───────────────────────────────────────────────────────────────
# Into the dist's .part, renamed into place once everything is checked, so
# an interrupted run never leaves a dist behind that looks complete.
echo "==> Staging GFXReconstruct into $GFXRECONSTRUCT_DIST..."
part="$GFXRECONSTRUCT_DIST.part"
claim_tree "$part" gfxreconstruct-dist 1
mkdir -p "$part/lib" "$part/bin" "$part/share/vulkan/explicit_layer.d"
cp "$GFXR_BUILD/layer/$LAYER" "$part/lib/"
for tool in $TOOLS; do
    cp "$GFXR_BUILD/tools/${tool#gfxrecon-}/$tool" "$part/bin/"
done
# The build tree's manifest names the layer next to it; the staged one names
# it relative to share/vulkan/explicit_layer.d.
sed "s|\"library_path\": \"[^\"]*\"|\"library_path\": \"../../../lib/$LAYER\"|" \
    "$GFXR_BUILD/layer/$MANIFEST" > "$part/share/vulkan/explicit_layer.d/$MANIFEST"
if ! grep -qF "\"library_path\": \"../../../lib/$LAYER\"" "$part/share/vulkan/explicit_layer.d/$MANIFEST"; then
    echo "Error: the staged $MANIFEST does not name lib/$LAYER"
    exit 1
fi

for file in "$part/lib/$LAYER" "$part"/bin/*; do
    deps=$(otool -L "$file" | tail -n +2 | awk '{print $1}' \
        | grep -vE "^(/usr/lib/|/System/Library/|@rpath/$LAYER)" || true)
    if [ -n "$deps" ]; then
        echo "Error: $(basename "$file") links libraries outside the system:"
        otool -L "$file" | tail -n +2
        exit 1
    fi
    echo "    $(basename "$file"): $(lipo -archs "$file"), links system libraries only"
done
for name in $EXPORTS; do
    if ! nm -gU "$part/lib/$LAYER" | grep -q " _$name\$"; then
        echo "Error: $LAYER does not export $name"
        exit 1
    fi
done
echo "    $LAYER exports $EXPORTS"

mkdir -p "$part/share/licenses"
staged=""
for entry in $LICENSES; do
    cp "$GFXR_SRC/${entry#*=}" "$part/share/licenses/${entry%%=*}"
    staged="$staged ${entry%%=*}"
done
IFS='|' read -r -a sections <<< "$THIRD_PARTY_SECTIONS"
for section in "${sections[@]}"; do
    if ! grep -qxF "## $section" "$part/share/licenses/gfxreconstruct-third-party.txt"; then
        echo "Error: GFXReconstruct's LICENSE_ThirdParty.txt has no '## $section' section"
        exit 1
    fi
done
echo "    licences:$staged"

gfxr_key "$GFXRECONSTRUCT_GIT_COMMIT" > "$part/KEY"
# Written last, so a staged directory without it is an incomplete one.
echo "$GFXRECONSTRUCT_GIT_COMMIT" > "$part/COMMIT"
remove_tree "$GFXRECONSTRUCT_DIST"
mv "$part" "$GFXRECONSTRUCT_DIST"
echo "==> GFXReconstruct $GFXRECONSTRUCT_GIT_TAG ($GFXRECONSTRUCT_GIT_COMMIT) staged in $GFXRECONSTRUCT_DIST."
