#!/bin/bash
set -e

# host.sh [--metal] [--xcode-choice <file>]: report what a CI runner is and
# prepare its Xcode.
#
# Prints the machine, macOS, the disk space of the home directory, the
# workspace and the runner's temporary directory, Xcode, the SDK and
# the tools the build uses, so a run's log documents the runner. Exits unless
# macOS is at least MACOS_FLOOR: the bundle step's smoke test runs the
# signed loader, whose cross-architecture entitlement needs that macOS.
#
# Selects the newest Xcode in /Applications, betas left out, whose macOS SDK
# is at least the floor, or the one XCODE_APP names, with xcode-select, which
# needs sudo when it changes the selection (a hosted runner has it without a
# password). With --metal it makes sure Xcode's Metal toolchain is installed,
# which DXMT's shaders need, and downloads it when it is not.
#
# Asking every Xcode for its SDK is slow on a hosted runner, which has many.
# With --xcode-choice, <file> holds the path of the Xcode that search chose:
# when it names an Xcode that still exists, that one is taken and the Xcodes
# are only listed, without asking each for its SDK; otherwise the search
# runs and writes its choice to <file>. XCODE_APP still overrides either.
# The workflows cache <file> per runner image and version of this script and
# profile.sh, which decide the choice.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# MACOS_FLOOR, macos_newer and check_sdk_floor.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

usage() {
    echo "Usage: $0 [--metal] [--xcode-choice <file>]"
    exit 1
}

METAL=0
XCODE_CHOICE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --metal) METAL=1; shift ;;
        --xcode-choice) [ $# -ge 2 ] || usage; XCODE_CHOICE="$2"; shift 2 ;;
        *) usage ;;
    esac
done

# show <label> <command>...: print <label> and the first line the command
# prints, or that it failed.
show() {
    local label="$1" out
    shift
    out=$("$@" 2>&1 | head -n 1) || true
    printf '  %-22s %s\n' "$label" "${out:-(nothing)}"
}

echo "==> Runner"
show "macOS" sw_vers -productVersion
show "build" sw_vers -buildVersion
show "architecture" uname -m
show "model" sysctl -n hw.model
show "CPUs" sysctl -n hw.ncpu
show "memory (bytes)" sysctl -n hw.memsize
show "virtual machine" sysctl -n kern.hv_vmm_present
show "page size" pagesize
# The workspace holds the sources, the trees and the bundle, the runner's
# temporary directory the caches; on a hosted runner both are usually on one
# volume.
for d in "$HOME" "${GITHUB_WORKSPACE:-}" "${RUNNER_TEMP:-}"; do
    if [ -n "$d" ]; then
        echo "  disk of $d:"
        df -h "$d" | sed 's/^/    /'
    fi
done
show "image" printenv ImageOS
show "image version" printenv ImageVersion
show "Homebrew" brew --version
show "rustc" rustc --version
show "cargo" cargo --version
show "python3" python3 --version
show "git" git --version

macos=$(sw_vers -productVersion)
if macos_newer "$MACOS_FLOOR" "$macos"; then
    echo "Error: the runner runs macOS $macos, below the floor $MACOS_FLOOR; the smoke"
    echo "test cannot run the loader, whose entitlement needs the floor"
    exit 1
fi

# ── Xcode ───────────────────────────────────────────────────────────────
echo "==> Xcode"
best=""
chosen_by=""
if [ -n "$XCODE_CHOICE" ] && [ -s "$XCODE_CHOICE" ]; then
    best=$(head -n 1 "$XCODE_CHOICE")
    if [ -d "$best/Contents/Developer" ] && [ ! -L "$best" ]; then
        chosen_by="from the cache in $XCODE_CHOICE"
        # The Xcodes the runner has, for the log, without asking each for
        # its SDK, which is the slow part of the search.
        for app in /Applications/Xcode*.app; do
            if [ -d "$app" ] && [ ! -L "$app" ]; then
                version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
                    "$app/Contents/Info.plist" 2>/dev/null) || version="(unknown)"
                printf '  %-34s Xcode %-8s SDK not asked\n' "$app" "$version"
            fi
        done
    else
        echo "  $XCODE_CHOICE names $best, which is not an Xcode here; searching"
        best=""
    fi
fi
if [ -z "$best" ]; then
    best_version=""
    for app in /Applications/Xcode*.app; do
        if [ ! -d "$app" ] || [ -L "$app" ]; then
            continue
        fi
        version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
            "$app/Contents/Info.plist" 2>/dev/null) || continue
        sdk=$(DEVELOPER_DIR="$app/Contents/Developer" xcrun --sdk macosx --show-sdk-version 2>/dev/null) || sdk=""
        printf '  %-34s Xcode %-8s SDK %s\n' "$app" "$version" "${sdk:-(none)}"
        case "$app" in
            *[Bb]eta*) continue ;;
        esac
        if [ -n "$sdk" ] && ! macos_newer "$MACOS_FLOOR" "$sdk" \
            && { [ -z "$best" ] || macos_newer "$version" "$best_version"; }; then
            best="$app"
            best_version="$version"
        fi
    done
    chosen_by="by searching /Applications"
    if [ -n "$best" ] && [ -n "$XCODE_CHOICE" ]; then
        mkdir -p "$(dirname "$XCODE_CHOICE")"
        echo "$best" > "$XCODE_CHOICE"
    fi
fi
if [ -n "${XCODE_APP:-}" ]; then
    best="$XCODE_APP"
    chosen_by="by XCODE_APP"
fi
if [ -z "$best" ]; then
    echo "Error: no Xcode in /Applications has a macOS SDK of at least $MACOS_FLOOR"
    exit 1
fi
printf '  %-22s %s (%s)\n' "chosen" "$best" "$chosen_by"
if [ "$(xcode-select -p)" != "$best/Contents/Developer" ]; then
    echo "  Selecting $best"
    sudo xcode-select -s "$best/Contents/Developer"
fi
show "selected" xcode-select -p
show "xcodebuild" xcodebuild -version
show "SDK" xcrun --sdk macosx --show-sdk-version
check_sdk_floor

if [ "$METAL" -eq 1 ]; then
    echo "==> Metal toolchain"
    if ! xcrun -sdk macosx metal --version >/dev/null 2>&1; then
        echo "  Not installed; downloading it"
        xcodebuild -downloadComponent MetalToolchain
    fi
    if ! xcrun -sdk macosx metal --version >/dev/null 2>&1; then
        echo "Error: xcrun cannot run the Metal compiler"
        xcrun -sdk macosx metal --version || true
        exit 1
    fi
    show "metal" xcrun -sdk macosx metal --version
    show "path" xcrun -sdk macosx -f metal
fi
