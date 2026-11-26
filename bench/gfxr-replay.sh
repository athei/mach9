#!/bin/bash
# gfxr-replay.sh [<dir>] <capture.gfxr> [--rounds <n> | --screenshots <out>]
#     [--frames <first>-<end>] [-m <mode>] [--remove-unsupported]:
# replay a capture of compatdb's vulkan_capture with gfxrecon-replay on the
# Vulkan stack in <dir>, a full bundle (by default dist/wine) or a MESA_DIST
# of bin/build-mesa.sh, and print the time of the measured frames of each
# round, so two Mesa builds get comparable numbers for the frames a game
# recorded, without the game, Wine or FEX. With --screenshots it writes the
# frames as images instead, to see what a stack draws.
#
# - The replay draws into images of its own instead of a window
#   (--swapchain offscreen): nothing opens, and no display paces the frames.
# - --frames selects the frames, numbered from 1, <first> included and
#   <end> excluded; by default every frame after the first, which loads the
#   captured state. An <end> past the last frame stops at it.
# - Each of the --rounds rounds (5 by default) replays the whole capture in a
#   process of its own, measures the frames of --frames and prints one line:
#   the round, the frames measured, their duration in ms and the frames per
#   second. Compare two stacks by alternating invocations, as for kk.sh.
# - --screenshots <out> replays the capture once instead and writes each
#   frame of --frames into the directory <out>, made when missing, as
#   screenshot_frame_<n>.png, and prints the path of every image it wrote.
# - -m <mode> (gfxrecon-replay's memory translation: remap, realign or
#   rebind) and --remove-unsupported go to gfxrecon-replay in both modes. A
#   capture from another GPU can need them: its memory types and the
#   extensions and features it enabled are the capture GPU's.
#
# The loader is the stack's, found by its library name in lib/external or
# lib alone (DYLD_LIBRARY_PATH and DYLD_FALLBACK_LIBRARY_PATH), and the
# driver its share/vulkan/icd.d, with every layer disabled. gfxrecon-replay
# comes from GFXRECONSTRUCT_DIST of bin/build-gfxreconstruct.sh. The
# measurements and the replay's log go into a directory of mktemp -d, which
# it deletes; the log is printed when the replay fails. The exit status is
# gfxrecon-replay's first failure, or 2 when the stack, the capture or the
# replay is missing, <out> cannot be made, or a replay measured no frames or
# wrote no image.
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/../bin" && pwd)"
# shellcheck source=../bin/lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"
# shellcheck source=stack.sh
. "$REPO_DIR/bench/stack.sh"

path_setting GFXRECONSTRUCT_DIST "$BUILD_TOP/gfxreconstruct/dist"
replay="$GFXRECONSTRUCT_DIST/bin/gfxrecon-replay"

usage() {
    echo "Usage: $0 [<bundle or MESA_DIST>] <capture.gfxr> [--rounds <n> | --screenshots <out>]"
    echo "       [--frames <first>-<end>] [-m <mode>] [--remove-unsupported]"
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
rounds=""
screenshots=""
# Frame 1 of a replay loads the captured state.
first=2
end=1000000000
# The options both modes pass to gfxrecon-replay.
options=()
while [ $# -gt 0 ]; do
    case "$1" in
        --rounds) [ $# -gt 1 ] || usage; rounds="$2"; shift 2 ;;
        --screenshots) [ $# -gt 1 ] && [ -n "$2" ] || usage; screenshots="$2"; shift 2 ;;
        --frames) [ $# -gt 1 ] || usage; first="${2%%-*}"; end="${2#*-}"; shift 2 ;;
        -m) [ $# -gt 1 ] || usage; options+=(-m "$2"); shift 2 ;;
        --remove-unsupported) options+=(--remove-unsupported); shift ;;
        *) usage ;;
    esac
done
case "$first" in
    ''|*[!0-9]*) usage ;;
esac
case "$end" in
    ''|*[!0-9]*) usage ;;
esac
# gfxrecon-replay reads each bound as an int (std::stoi). The length check
# keeps the arithmetic below from overflowing.
[ ${#first} -le 10 ] && [ ${#end} -le 10 ] || usage
first=$((10#$first))
end=$((10#$end))
[ "$first" -ge 1 ] && [ "$end" -gt "$first" ] && [ "$end" -le 2147483647 ] || usage
if [ -n "$screenshots" ]; then
    [ -z "$rounds" ] || usage
else
    rounds="${rounds:-5}"
    case "$rounds" in
        ''|*[!0-9]*|0) usage ;;
    esac
fi
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

# run_replay <argument>...: run gfxrecon-replay on the stack with
# <argument>... ahead of the options and the capture; on a failure, print
# its log and exit with its status.
run_replay() {
    env DYLD_LIBRARY_PATH="$lib" DYLD_FALLBACK_LIBRARY_PATH="$lib" \
        VK_DRIVER_FILES="$stack/share/vulkan/icd.d" VK_LOADER_LAYERS_DISABLE='~all~' \
        MESA_SHADER_CACHE_DIR="$tmp/mesa-shader-cache" \
        "$replay" --swapchain offscreen "$@" "${options[@]}" \
        "$capture" > "$tmp/replay.log" 2>&1 || {
        status=$?
        cat "$tmp/replay.log"
        exit "$status"
    }
}

if [ -n "$screenshots" ]; then
    if ! mkdir -p "$screenshots" 2>/dev/null; then
        echo "Error: $screenshots is not a directory and cannot be made"
        exit 2
    fi
    # Images older than this marker are from an earlier run into <out>.
    touch "$tmp/start"
    # --screenshots takes the last frame, which is included.
    run_replay --screenshots "$first-$((end - 1))" --screenshot-format png \
        --screenshot-dir "$screenshots"
    find "$screenshots" -maxdepth 1 -name '*.png' -newer "$tmp/start" | sort -V > "$tmp/images"
    if [ ! -s "$tmp/images" ]; then
        echo "Error: the replay wrote no image into $screenshots:"
        cat "$tmp/replay.log"
        exit 2
    fi
    sed 's/^/gfxr-replay: image /' "$tmp/images"
    exit 0
fi

for round in $(seq 1 "$rounds"); do
    run_replay --measurement-frame-range "$first-$end" --measurement-file "$tmp/round.json"
    if [ ! -f "$tmp/round.json" ]; then
        echo "Error: round $round measured no frames:"
        cat "$tmp/replay.log"
        exit 2
    fi
    count=$(plutil -extract frame_range.frame_count raw -o - "$tmp/round.json")
    duration=$(plutil -extract frame_range.duration raw -o - "$tmp/round.json")
    fps=$(plutil -extract frame_range.fps raw -o - "$tmp/round.json")
    awk -v r="$round" -v c="$count" -v d="$duration" -v f="$fps" \
        'BEGIN { printf "gfxr-replay: round %d frames %d ms %.3f fps %.1f\n", r, c, d * 1000, f }'
    rm -f "$tmp/round.json"
done
