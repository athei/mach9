#!/bin/bash
# zink.sh [<dir>] [<argument>...]: build zink.c for the host and run it on the
# OpenGL stack in <dir>, a full bundle (lib/external/libEGL.1.dylib, by
# default dist/wine) or a MESA_DIST of bin/build-mesa.sh (lib/libEGL.1.dylib).
# The arguments after <dir> go to the program; the header of zink.c lists
# them and what it times.
#
# Zink loads the Vulkan loader next to the gallium library, and the loader
# takes its driver from VK_DRIVER_FILES, the stack's share/vulkan/icd.d, with
# every layer disabled; Mesa's shader cache is a directory of this run. Other
# variables, ZINK_DEBUG and MESA_KK_DEBUG among them, reach it from the
# caller. The first lines name the Mesa and loader commits of the stack
# (lib/wine/tests/COMPONENTS of a bundle, COMMIT and LOADER_COMMIT of a
# MESA_DIST).
#
# The binary and the shader cache go into a directory of mktemp -d, which it
# deletes when it exits; it writes nothing else. The exit status is the
# program's, or 2 when the stack or the build is missing.
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/../bin" && pwd)"
# profile.sh says where the bundle is.
# shellcheck source=../bin/lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"
# shellcheck source=stack.sh
. "$REPO_DIR/bench/stack.sh"

stack="$BUNDLE_DEST/wine"
if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then
    stack="$1"
    shift
fi
find_stack zink "$stack"
if [ ! -f "$lib/libEGL.1.dylib" ]; then
    echo "Error: $lib/libEGL.1.dylib is missing"
    exit 2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
if ! xcrun clang -std=c11 -O2 -Wall -Wextra -o "$tmp/zink" "$REPO_DIR/bench/zink.c"; then
    exit 2
fi
status=0
env VK_DRIVER_FILES="$stack/share/vulkan/icd.d" VK_LOADER_LAYERS_DISABLE='~all~' \
    MESA_SHADER_CACHE_DIR="$tmp/mesa-shader-cache" "$tmp/zink" "$lib/libEGL.1.dylib" "$@" || status=$?
exit "$status"
