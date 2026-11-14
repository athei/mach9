#!/bin/bash
set -e

# Builds Khronos' Vulkan validation layers (VK_LAYER_KHRONOS_validation) for
# this Mac and stages them in VULKAN_VALIDATION_LAYERS_DIST, for the checks
# that run Zink or KosmicKrisp under them (docs/internals.md, what the bundle
# step checks):
#
#   lib/libVkLayer_khronos_validation.dylib   the layer
#   share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json
#                   its manifest, which names the layer by a path relative
#                   to itself, so VK_LAYER_PATH can point at the directory
#                   wherever the dist is
#   DEPS            the commit of each source the layers were built on
#   KEY             what the dist was built from (dist_key in profile.sh)
#   COMMIT          the validation layers' source commit
#
# The layers come from the tag redist.env pins, the SDK release of the
# pinned Vulkan loader. CMake's UPDATE_DEPS runs the tag's
# scripts/update_deps.py, which fetches the sources the layers build on
# (Vulkan-Headers, Vulkan-Utility-Libraries, SPIRV-Headers, SPIRV-Tools) at
# what the tag's scripts/known_good.json names, so the tag fixes them too.
# The stage step checks that the Vulkan-Headers are the ones redist.env pins.
#
# The layers are built for the host, not the floor: they run here and are
# never bundled. They link nothing from Homebrew; the Homebrew prefix is
# kept out of CMake's search, and the stage step checks what they link. Not
# part of build-wine.sh, and neither the bundle nor Mach9.app carries them.
#
# Nothing is built when VULKAN_VALIDATION_LAYERS_DIST is complete and has
# the KEY this run would write (skip_current_dist in profile.sh). --clean
# builds the tree and the dist from scratch; the clone and the fetched
# sources are kept while the pin is unchanged. --print-key prints that KEY
# and exits (print_dist_key).

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

# The tree: the clone, the sources update_deps.py fetches, and the CMake
# tree. Set here rather than in profile.sh, which every component's KEY
# hashes, as build-vk-gl-cts.sh does; bundle-wine.sh takes the same default.
VULKAN_VALIDATION_LAYERS_BUILD_DIR="${VULKAN_VALIDATION_LAYERS_BUILD_DIR:-$BUILD_TOP/vulkan-validation-layers}"
VULKAN_VALIDATION_LAYERS_DIST="${VULKAN_VALIDATION_LAYERS_DIST:-$VULKAN_VALIDATION_LAYERS_BUILD_DIR/dist}"
VVL_SRC="$VULKAN_VALIDATION_LAYERS_BUILD_DIR/src"
VVL_DEPS="$VULKAN_VALIDATION_LAYERS_BUILD_DIR/external"
VVL_BUILD="$VULKAN_VALIDATION_LAYERS_BUILD_DIR/build"
LAYER=libVkLayer_khronos_validation.dylib
MANIFEST=VkLayer_khronos_validation.json

if [ -z "$VULKAN_VALIDATION_LAYERS_GIT_COMMIT" ]; then
    echo "Error: redist.env pins no VULKAN_VALIDATION_LAYERS_GIT_COMMIT"
    exit 1
fi
for tool in git cmake ninja python3; do
    if ! command -v "$tool" >/dev/null; then
        echo "Error: $tool not found (brew install $tool)"
        exit 1
    fi
done

# ── Dist ────────────────────────────────────────────────────────────────
# vvl_key <commit>: the KEY of a dist built from <commit>. Besides the
# commit, which fixes what update_deps.py fetches, what builds it: Apple's
# clang and the macOS SDK, CMake, and the Python that runs update_deps.py.
vvl_key() {
    dist_key "$SCRIPT_DIR/build-vulkan-validation-layers.sh" "vulkan-validation-layers=$1" \
        "cc=$(cc_version /usr/bin/clang)" "$(sdk_key)" \
        "cmake=$(cmake --version | head -n 1)" \
        "python=$(python3 -c 'import platform; print(platform.python_version())')"
}
print_dist_key "$VULKAN_VALIDATION_LAYERS_GIT_COMMIT" redist.env vvl_key

claim_tree "$VULKAN_VALIDATION_LAYERS_BUILD_DIR" vulkan-validation-layers
if [ "$CLEAN" -eq 1 ]; then
    echo "==> Cleaning the validation layers' tree and $VULKAN_VALIDATION_LAYERS_DIST..."
    remove_tree "$VVL_BUILD"
fi
# The dist is inside the build directory unless overridden, so it is
# claimed on its own, and so is its .part below.
claim_tree "$VULKAN_VALIDATION_LAYERS_DIST" vulkan-validation-layers-dist "$CLEAN"
skip_current_dist "$VULKAN_VALIDATION_LAYERS_DIST" "Vulkan validation layers" \
    "$VULKAN_VALIDATION_LAYERS_GIT_COMMIT" vvl_key

# ── Source ──────────────────────────────────────────────────────────────
echo "==> Validation layers source: $VULKAN_VALIDATION_LAYERS_GIT_URL $VULKAN_VALIDATION_LAYERS_GIT_TAG"
pinned_clone "$VULKAN_VALIDATION_LAYERS_GIT_URL" "$VULKAN_VALIDATION_LAYERS_GIT_TAG" \
    "$VULKAN_VALIDATION_LAYERS_GIT_COMMIT" "$VVL_SRC" --no-submodules

# ── Build ───────────────────────────────────────────────────────────────
# UPDATE_DEPS fetches and builds the dependencies into VVL_DEPS. Apple's
# compilers named explicitly, through WINE_CCACHE when WINE_BUILD_CCACHE is
# set (profile.sh); the empty launcher clears one an earlier run set. The
# Homebrew prefix is ignored in every find_* call, so no Homebrew SPIRV-Tools
# or headers stand in for the pinned ones.
echo "==> Building the validation layers..."
(
    unset MACOSX_DEPLOYMENT_TARGET CC CXX CFLAGS CXXFLAGS CPPFLAGS LDFLAGS
    GIT_TERMINAL_PROMPT=0 cmake -S "$VVL_SRC" -B "$VVL_BUILD" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DUPDATE_DEPS=ON -DUPDATE_DEPS_DIR="$VVL_DEPS" \
        -DBUILD_TESTS=OFF \
        -DCMAKE_C_COMPILER=/usr/bin/clang -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
        -DCMAKE_OSX_ARCHITECTURES=arm64 \
        -DCMAKE_IGNORE_PREFIX_PATH="$DEPS_PREFIX;/usr/local" \
        -DCMAKE_C_COMPILER_LAUNCHER="$WINE_CCACHE" -DCMAKE_CXX_COMPILER_LAUNCHER="$WINE_CCACHE" \
        >/dev/null
    ninja -C "$VVL_BUILD"
)

# ── Stage ───────────────────────────────────────────────────────────────
# Into the dist's .part, renamed into place once everything is checked, so
# an interrupted run never leaves a dist behind that looks complete.
echo "==> Staging the validation layers into $VULKAN_VALIDATION_LAYERS_DIST..."
part="$VULKAN_VALIDATION_LAYERS_DIST.part"
claim_tree "$part" vulkan-validation-layers-dist 1
mkdir -p "$part/lib" "$part/share/vulkan/explicit_layer.d"
cp "$VVL_BUILD/layers/$LAYER" "$part/lib/"
# The build tree's manifest names the layer next to it; the staged one names
# it relative to share/vulkan/explicit_layer.d.
sed "s|\"library_path\": \"[^\"]*\"|\"library_path\": \"../../../lib/$LAYER\"|" \
    "$VVL_BUILD/layers/$MANIFEST" > "$part/share/vulkan/explicit_layer.d/$MANIFEST"
if ! grep -qF "\"library_path\": \"../../../lib/$LAYER\"" "$part/share/vulkan/explicit_layer.d/$MANIFEST"; then
    echo "Error: the staged $MANIFEST does not name lib/$LAYER"
    exit 1
fi

# The commit of each source update_deps.py fetched, and the check that the
# headers are the pinned ones the loader was built with.
for dir in "$VVL_DEPS"/*/; do
    if [ -e "$dir/.git" ]; then
        echo "$(basename "$dir")=$(git -C "$dir" rev-parse HEAD)"
    fi
done > "$part/DEPS"
if ! grep -qx "Vulkan-Headers=$VULKAN_HEADERS_GIT_COMMIT" "$part/DEPS"; then
    echo "Error: the validation layers were built on other Vulkan-Headers than redist.env pins:"
    cat "$part/DEPS"
    exit 1
fi

deps=$(otool -L "$part/lib/$LAYER" | tail -n +2 | awk '{print $1}' \
    | grep -vE '^(/usr/lib/|/System/Library/|@rpath/libVkLayer_khronos_validation)' || true)
if [ -n "$deps" ]; then
    echo "Error: $LAYER links libraries outside the system:"
    otool -L "$part/lib/$LAYER" | tail -n +2
    exit 1
fi
echo "    $LAYER: $(lipo -archs "$part/lib/$LAYER"), links system libraries only"
sed 's/^/    built on /' "$part/DEPS"

vvl_key "$VULKAN_VALIDATION_LAYERS_GIT_COMMIT" > "$part/KEY"
# Written last, so a staged directory without it is an incomplete one.
echo "$VULKAN_VALIDATION_LAYERS_GIT_COMMIT" > "$part/COMMIT"
remove_tree "$VULKAN_VALIDATION_LAYERS_DIST"
mv "$part" "$VULKAN_VALIDATION_LAYERS_DIST"
echo "==> Vulkan validation layers $VULKAN_VALIDATION_LAYERS_GIT_TAG ($VULKAN_VALIDATION_LAYERS_GIT_COMMIT) staged in $VULKAN_VALIDATION_LAYERS_DIST."
