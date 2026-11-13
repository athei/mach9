#!/bin/bash
# kk.sh [<dir>] [<argument>...]: build kk.c for the host and run it on the
# Vulkan stack in <dir>, a full bundle (lib/external/libvulkan.1.dylib, by
# default dist/wine) or a MESA_DIST of bin/build-mesa.sh
# (lib/libvulkan.1.dylib). The arguments after <dir> go to the program; the
# header of kk.c lists them and the cases.
#
# The program runs as the deqp-vk suite runs deqp-vk: the loader takes its
# driver from VK_DRIVER_FILES, the stack's share/vulkan/icd.d, with every
# layer disabled, and Mesa's shader cache is a directory of this run. Other
# variables, MESA_KK_DEBUG among them, reach it from the caller. The first
# lines name the Mesa and loader commits of the stack (lib/wine/tests/
# COMPONENTS of a bundle, COMMIT and LOADER_COMMIT of a MESA_DIST).
#
# It compiles against the Vulkan headers build-mesa.sh builds the loader
# with, in MESA_BUILD_DIR. The binary and the shader cache go into a
# directory of mktemp -d, which it deletes when it exits; it writes nothing
# else. The exit status is the program's, or 2 when the stack, the headers
# or the build is missing.
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/../bin" && pwd)"
# profile.sh says where the bundle and the Mesa trees are.
# shellcheck source=../bin/lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

stack="$BUNDLE_DEST/wine"
if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then
    stack="$1"
    shift
fi
if [ ! -d "$stack" ]; then
    echo "Error: $stack is not a directory"
    exit 2
fi
stack="$(cd "$stack" && pwd)"
if [ -f "$stack/lib/external/libvulkan.1.dylib" ]; then
    loader="$stack/lib/external/libvulkan.1.dylib"
    components="$stack/lib/wine/tests/COMPONENTS"
    if [ ! -f "$components" ]; then
        echo "Error: $components is missing (a full bundle names its Mesa there)"
        exit 2
    fi
    grep -E '^(mesa|vulkan-loader)=' "$components" | sed 's/^/kk: stack /'
elif [ -f "$stack/lib/libvulkan.1.dylib" ]; then
    loader="$stack/lib/libvulkan.1.dylib"
    echo "kk: stack mesa=$(cat "$stack/COMMIT")"
    echo "kk: stack vulkan-loader=$(cat "$stack/LOADER_COMMIT")"
else
    echo "Error: $stack is neither a bundle nor a MESA_DIST: it has no" \
        "lib/external/libvulkan.1.dylib or lib/libvulkan.1.dylib"
    exit 2
fi
if [ ! -d "$stack/share/vulkan/icd.d" ]; then
    echo "Error: $stack/share/vulkan/icd.d is missing"
    exit 2
fi

# HEADERS_PREFIX of build-mesa.sh.
headers="$MESA_BUILD_DIR/vulkan-headers-install/include"
if [ ! -f "$headers/vulkan/vulkan_core.h" ]; then
    echo "Error: no Vulkan headers in $headers; bin/build-mesa.sh installs them"
    exit 2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
if ! xcrun clang -std=c11 -O2 -Wall -Wextra -I "$headers" -o "$tmp/kk" "$REPO_DIR/bench/kk.c"; then
    exit 2
fi
status=0
env VK_DRIVER_FILES="$stack/share/vulkan/icd.d" VK_LOADER_LAYERS_DISABLE='~all~' \
    MESA_SHADER_CACHE_DIR="$tmp/mesa-shader-cache" "$tmp/kk" "$loader" "$@" || status=$?
exit "$status"
