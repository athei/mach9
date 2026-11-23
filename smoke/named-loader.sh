#!/bin/bash
# Native lease/exec test, optionally followed by a launch in a booted smoke prefix.
# Usage: bash named-loader.sh <Wine source> <configured Wine build> [bundled bin/wine]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src="${1:?Wine source required}/dlls/ntdll/unix"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mach9-named-loader.XXXXXXXX")
pid=
cleanup() {
    if [[ -n $pid ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    rm -rf "$tmp"
}
trap cleanup EXIT
mkdir -p "$tmp/wine.app/Contents/MacOS"
probe="$tmp/wine.app/Contents/MacOS/wine"
xcrun clang -Wall -Wextra -Werror -I"${2:?Wine build required}/include" -I"$src" \
    "$here/named-loader.c" "$src/named_loader.c" -o "$probe"
"$probe" "$tmp"
[[ $# -ge 3 ]] || exit 0

# Run a real Windows program through the installed loader and compatdb. Keep
# cmd reading until we've verified the running bundle. A silent fallback to
# wine.app or an intermediate launcher with a different PID fails this check.
: "${WINEPREFIX:?use a booted throwaway prefix}"
mkfifo "$tmp/input"
exec 3<>"$tmp/input"
WINE_COMPATDB='exe=cmd.exe;display_name=Mach9 Smoke' "$3" 'C:\windows\system32\cmd.exe' \
    /d /q /c 'echo named-loader-ready&&set /p reply=' <&3 3>&- >"$tmp/windows.log" 2>&1 &
pid=$!
for ((i=0; i<600; ++i)); do
    if grep -q '^named-loader-ready' "$tmp/windows.log"; then break; fi
    if ! kill -0 "$pid" 2>/dev/null; then break; fi
    sleep 0.1
done
if ! grep -q '^named-loader-ready' "$tmp/windows.log"; then
    cat "$tmp/windows.log" >&2
    echo 'Named Windows process did not reach its ready marker' >&2
    exit 1
fi
image=$("$probe" --image "$pid")
case "$image" in
    */'Mach9 Smoke.app/Contents/MacOS/wine') ;;
    *) echo "Expected a named signed loader, got: $image" >&2; exit 1 ;;
esac
/usr/bin/codesign --verify --strict "${image%/Contents/MacOS/wine}"
printf 'done\r\n' >&3
wait "$pid"
pid=
echo 'named-loader: signed bundle ran cmd.exe in the launched process'
