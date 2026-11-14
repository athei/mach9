# shellcheck shell=bash
# Sourced by kk.sh and zink.sh after bin/lib/profile.sh.
#
# find_stack <name> <dir>: set stack to the absolute path of <dir>, a full
# bundle or a MESA_DIST of bin/build-mesa.sh, and lib to the directory that
# holds its Vulkan loader and EGL (lib/external of a bundle, lib of a
# MESA_DIST), and print the Mesa and loader commits of the stack after
# "<name>: stack" (lib/wine/tests/COMPONENTS of a bundle, COMMIT and
# LOADER_COMMIT of a MESA_DIST). Exit 2 when <dir> is neither or has no
# driver manifest in share/vulkan/icd.d.
find_stack() {
    local name="$1" components
    stack="$2"
    if [ ! -d "$stack" ]; then
        echo "Error: $stack is not a directory"
        exit 2
    fi
    stack="$(cd "$stack" && pwd)"
    if [ -f "$stack/lib/external/libvulkan.1.dylib" ]; then
        lib="$stack/lib/external"
        components="$stack/lib/wine/tests/COMPONENTS"
        if [ ! -f "$components" ]; then
            echo "Error: $components is missing (a full bundle names its Mesa there)"
            exit 2
        fi
        grep -E '^(mesa|vulkan-loader)=' "$components" | sed "s/^/$name: stack /"
    elif [ -f "$stack/lib/libvulkan.1.dylib" ]; then
        lib="$stack/lib"
        echo "$name: stack mesa=$(cat "$stack/COMMIT")"
        echo "$name: stack vulkan-loader=$(cat "$stack/LOADER_COMMIT")"
    else
        echo "Error: $stack is neither a bundle nor a MESA_DIST: it has no" \
            "lib/external/libvulkan.1.dylib or lib/libvulkan.1.dylib"
        exit 2
    fi
    if [ ! -d "$stack/share/vulkan/icd.d" ]; then
        echo "Error: $stack/share/vulkan/icd.d is missing"
        exit 2
    fi
}
