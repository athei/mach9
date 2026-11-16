# shellcheck shell=bash
# The Wine tests the Wine SDK and a full bundle carry, for build-wine.sh and
# bundle-wine.sh, which source this file after profile.sh. It is not part of
# profile.sh, whose hash is in the KEY of every staged dist and in the CI
# caches, so a change to the list rebuilds and refetches nothing else.

# Wine's graphics conformance tests, and dinput's, by the directory under
# dlls/ whose tests/ they are. The Wine SDK and a full bundle carry each as
# lib/wine/tests/<pe-dir>/<name>_test.exe, so the bundle's Direct3D, OpenGL
# and Vulkan stack and its DirectInput can be tested in place.
# - A plain name is built for TEST_PE_DIRS (profile.sh), the x86 code games
#   run.
# - A name with a :<pe-dir>[,<pe-dir>...] suffix is built for those
#   directories only. Wine builds no 64-bit d3d8.dll (enable_d3d8 in its
#   configure.ac), and so no 64-bit d3d8 test. dinput's tests also run as
#   native ARM64 code: their test drivers are built for the test's machine
#   and the 64-bit winedevice.exe that loads them is native ARM64, which
#   loads no x86_64 driver.
WINE_TESTS="ddraw d3d8:i386-windows d3d9 d3d10 d3d10_1 d3d10core d3d11 d3d12 dxgi d2d1 opengl32 vulkan-1 dinput:i386-windows,x86_64-windows,aarch64-windows"
# Every PE directory a test of WINE_TESTS is built for.
WINE_TEST_PE_DIRS="$TEST_PE_DIRS aarch64-windows"

# wine_tests <pe-dir>: print the name of every test of WINE_TESTS that is
# built for <pe-dir>, one per line.
wine_tests() {
    local t dirs
    for t in $WINE_TESTS; do
        case "$t" in
            *:*) dirs=${t#*:}; t=${t%%:*} ;;
            *) dirs=${TEST_PE_DIRS// /,} ;;
        esac
        case ",$dirs," in
            *",$1,"*) echo "$t" ;;
        esac
    done
}
