#!/bin/bash
# gfxr-replay.sh [<dir>] <capture.gfxr> [--rounds <n>] [--frames <first>-<end>]:
# replay a capture of compatdb's vulkan_capture with gfxrecon-replay on the
# Vulkan stack in <dir>, a full bundle (by default dist/wine) or a MESA_DIST
# of bin/build-mesa.sh, and print the time of the measured frames of each
# round, so two Mesa builds get comparable numbers for the frames a game
# recorded, without the game, Wine or FEX.
#
# - The replay draws into images of its own instead of a window
#   (--swapchain offscreen): nothing opens, and no display paces the frames.
# - It measures the frames of --frames, the range gfxrecon-replay's
#   --measurement-frame-range takes, first frame included and end excluded;
#   by default every frame after the first, which loads the captured state.
# - Each of the --rounds rounds (5 by default) replays the whole capture in a
#   process of its own and prints one line: the round, the frames measured,
#   their duration in ms and the frames per second. Compare two stacks by
#   alternating invocations, as for kk.sh.
#
# The loader is the stack's, found by its library name in lib/external or
# lib alone (DYLD_LIBRARY_PATH and DYLD_FALLBACK_LIBRARY_PATH), and the
# driver its share/vulkan/icd.d, with every layer disabled. gfxrecon-replay
# comes from GFXRECONSTRUCT_DIST of bin/build-gfxreconstruct.sh. The
# measurements go into a directory of mktemp -d, which it deletes. The exit
# status is gfxrecon-replay's first failure, or 2 when the stack, the
# capture or the replay is missing.
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/../bin" && pwd)"
# shellcheck source=../bin/lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"
# shellcheck source=stack.sh
. "$REPO_DIR/bench/stack.sh"

GFXRECONSTRUCT_DIST="${GFXRECONSTRUCT_DIST:-$BUILD_TOP/gfxreconstruct/dist}"
replay="$GFXRECONSTRUCT_DIST/bin/gfxrecon-replay"

usage() {
    echo "Usage: $0 [<bundle or MESA_DIST>] <capture.gfxr> [--rounds <n>] [--frames <first>-<end>]"
    exit 2
}

stack="$BUNDLE_DEST/wine"
if [ $# -gt 1 ] && [ "${2#-}" = "$2" ]; then
    stack="$1"
    shift
fi
[ $# -gt 0 ] || usage
capture="$1"
shift
rounds=5
# Frame 1 of a replay loads the captured state; an end past the last frame
# stops at it.
frames=(--measurement-frame-range 2-1000000000)
while [ $# -gt 0 ]; do
    case "$1" in
        --rounds) [ $# -gt 1 ] || usage; rounds="$2"; shift 2 ;;
        --frames) [ $# -gt 1 ] || usage; frames=(--measurement-frame-range "$2"); shift 2 ;;
        *) usage ;;
    esac
done
case "$rounds" in
    ''|*[!0-9]*|0) usage ;;
esac
if [ ! -f "$capture" ]; then
    echo "Error: no capture at $capture"
    exit 2
fi
if [ ! -x "$replay" ]; then
    echo "Error: no gfxrecon-replay in $GFXRECONSTRUCT_DIST/bin; bin/build-gfxreconstruct.sh stages it"
    exit 2
fi
find_stack gfxr-replay "$stack"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for round in $(seq 1 "$rounds"); do
    env DYLD_LIBRARY_PATH="$lib" DYLD_FALLBACK_LIBRARY_PATH="$lib" \
        VK_DRIVER_FILES="$stack/share/vulkan/icd.d" VK_LOADER_LAYERS_DISABLE='~all~' \
        MESA_SHADER_CACHE_DIR="$tmp/mesa-shader-cache" \
        "$replay" --swapchain offscreen "${frames[@]}" --measurement-file "$tmp/round.json" \
        "$capture" > "$tmp/round.log" 2>&1 || {
        status=$?
        cat "$tmp/round.log"
        exit "$status"
    }
    if [ ! -f "$tmp/round.json" ]; then
        echo "Error: round $round measured no frames:"
        cat "$tmp/round.log"
        exit 2
    fi
    count=$(plutil -extract frame_range.frame_count raw -o - "$tmp/round.json")
    duration=$(plutil -extract frame_range.duration raw -o - "$tmp/round.json")
    fps=$(plutil -extract frame_range.fps raw -o - "$tmp/round.json")
    awk -v r="$round" -v c="$count" -v d="$duration" -v f="$fps" \
        'BEGIN { printf "gfxr-replay: round %d frames %d ms %.3f fps %.1f\n", r, c, d * 1000, f }'
    rm -f "$tmp/round.json"
done
