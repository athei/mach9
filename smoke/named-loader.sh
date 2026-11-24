#!/bin/bash
# Native lease/exec test, optionally followed by a launch in a booted smoke prefix.
# Usage: bash named-loader.sh <Wine source> <configured Wine build> [bundled bin/wine]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
wine_src=${1:?Wine source required}
src="$wine_src/dlls/ntdll/unix"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mach9-named-loader.XXXXXXXX")
pid=
cleanup() {
    # Do not wait on a stuck process from the EXIT trap. Removing hold-child
    # releases the PE child, which Wine spawns as a native grandchild.
    if [[ -n $pid ]]; then kill -KILL "$pid" 2>/dev/null || true; fi
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' INT TERM HUP
mkdir -p "$tmp/wine.app/Contents/MacOS"
probe="$tmp/wine.app/Contents/MacOS/wine"
xcrun clang -Wall -Wextra -Werror -I"${2:?Wine build required}/include" -I"$src" \
    "$here/named-loader.c" "$src/named_loader.c" -o "$probe"
"$probe" "$tmp"
[[ $# -ge 3 ]] || exit 0

: "${WINEPREFIX:?use a booted throwaway prefix}"
wine=$3
[[ $wine = /* ]] || wine="$PWD/$wine"

wait_for_ready() {
    local log=$1 i marker
    shift
    for ((i=0; i<300; ++i)); do
        for marker; do
            if ! grep -q "^$marker" "$log"; then break; fi
            marker=
        done
        if [[ -z $marker ]]; then return 0; fi
        if ! kill -0 "$pid" 2>/dev/null; then break; fi
        sleep 0.1
    done
    cat "$log" >&2
    echo "Named loader test did not reach its ready marker: $marker" >&2
    return 1
}

wait_for_exit() {
    local log=$1 i status=0
    for ((i=0; i<100; ++i)); do
        if ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid" || status=$?
            pid=
            if ((status == 0)); then return 0; fi
            cat "$log" >&2
            echo "Named loader test exited with status $status" >&2
            return 1
        fi
        sleep 0.1
    done
    cat "$log" >&2
    echo 'Named loader test did not exit after being released' >&2
    return 1
}

# Run a real Windows program through the installed loader and compatdb. Keep
# cmd reading until we've verified the running bundle. A silent fallback to
# wine.app or an intermediate launcher with a different PID fails this check.
mkfifo "$tmp/input"
exec 3<>"$tmp/input"
WINE_COMPATDB='exe=cmd.exe;display_name=Mach9 Smoke' "$wine" 'C:\windows\system32\cmd.exe' \
    /d /q /c 'echo named-loader-ready&&set /p reply=' <&3 3>&- >"$tmp/windows.log" 2>&1 &
pid=$!
wait_for_ready "$tmp/windows.log" named-loader-ready
image=$("$probe" --image "$pid")
case "$image" in
    */'Mach9 Smoke.app/Contents/MacOS/wine') ;;
    *) echo "Expected a named signed loader, got: $image" >&2; exit 1 ;;
esac
/usr/bin/codesign --verify --strict "${image%/Contents/MacOS/wine}"
printf 'done\r\n' >&3
exec 3>&-
wait_for_exit "$tmp/windows.log"
echo 'named-loader: signed bundle ran cmd.exe in the launched process'

# Unlike the direct launch above, this rule can only match the environment
# block supplied to CreateProcessW. Parent and child have the same PE name.
cc="${MINGW_DIR:-/opt/llvm-mingw}/bin/aarch64-w64-mingw32-clang"
"$cc" -O2 -Wall -Wextra -Werror -municode -D__WINESRC__ -isystem "$wine_src/include" \
    -o "$tmp/named-loader-child.exe" "$here/named-loader-child.c"
: >"$tmp/hold-child"
(
    cd "$tmp"
    exec env -u SteamAppId -u WINE_COMPATDB_FILE \
        WINE_COMPATDB='exe=steam:4294967294;display_name=Mach9 Steam Child' "$wine" "$PWD/named-loader-child.exe"
) >"$tmp/child.log" 2>&1 &
pid=$!
wait_for_ready "$tmp/child.log" 'named-loader-child-created ' 'named-loader-child-ready '
created=$(tr -d '\r' <"$tmp/child.log" | sed -n 's/^named-loader-child-created //p')
ready=$(tr -d '\r' <"$tmp/child.log" | sed -n 's/^named-loader-child-ready //p')
if [[ ! $ready =~ ^([1-9][0-9]*)\ ([1-9][0-9]*)$ || ${BASH_REMATCH[1]} != "$created" || ${BASH_REMATCH[2]} == "$pid" ]]; then
    cat "$tmp/child.log" >&2
    echo 'Could not identify the native PID of the child that CreateProcessW launched' >&2
    exit 1
fi
child_pid=${BASH_REMATCH[2]}
child_image=$("$probe" --image "$child_pid")
case "$child_image" in
    */'Mach9 Steam Child.app/Contents/MacOS/wine') ;;
    *) echo "Expected a named loader for child PID $child_pid, got: $child_image" >&2; exit 1 ;;
esac
parent_image=$("$probe" --image "$pid")
if [[ $parent_image != */wine.app/Contents/MacOS/wine || ! $parent_image -ef $wine ]]; then
    echo "Expected the packaged wine.app for parent PID $pid, got: $parent_image" >&2
    exit 1
fi
/usr/bin/codesign --verify --strict "${child_image%/Contents/MacOS/wine}"
rm "$tmp/hold-child"
wait_for_exit "$tmp/child.log"
echo "named-loader: SteamAppId named child PID $child_pid without renaming the parent"
