#!/bin/bash
set -e

# keychain.sh setup|cleanup: give a CI runner the code signing identity the
# loader is signed with (bundle-wine.sh, loader_preflight), and take it away
# again.
#
# setup imports the PKCS#12 file in MACOS_SIGNING_P12 (base64), whose
# password is MACOS_SIGNING_P12_PASSWORD, into a keychain of its own in
# RUNNER_TEMP with a random password, unlocks it for six hours, lets codesign
# use the key without asking, and puts the keychain first in the user's
# search list, ahead of the keychains already there. It then checks that the
# keychain holds a valid code signing identity of the team in
# LOADER_PROFILE. Where
# the runner lacks the intermediate certificate of a Developer ID
# certificate, the identity is not valid until Apple's Developer ID G2
# intermediate is imported, which setup does then.
#
# cleanup deletes that keychain, which also takes it out of the search list.
# It is fine to run when setup did not run or failed.
#
# Nothing here prints a secret, and the decoded file is deleted as soon as it
# is imported.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# LOADER_PROFILE.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

if [ -z "${RUNNER_TEMP:-}" ]; then
    echo "Error: RUNNER_TEMP is not set; this script is for a CI runner"
    exit 1
fi
KEYCHAIN="$RUNNER_TEMP/wine-build-signing.keychain-db"
DEVID_G2_URL=https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer

usage() {
    echo "Usage: $0 setup|cleanup"
    exit 1
}

# search_list: print the user's keychain search list, one path per line.
search_list() {
    security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//'
}

# team_identities <team>: print the SHA-1 of every valid code signing
# identity of <team> in KEYCHAIN.
team_identities() {
    security find-identity -v -p codesigning "$KEYCHAIN" \
        | awk -v team="($1)\"" 'index($0, team) { print $2 }'
}

cmd_setup() {
    local pw p12 team plist existing=() k
    if [ -z "${MACOS_SIGNING_P12:-}" ] || [ -z "${MACOS_SIGNING_P12_PASSWORD:-}" ]; then
        echo "Error: MACOS_SIGNING_P12 and MACOS_SIGNING_P12_PASSWORD must both be set"
        echo "(the secrets of the release environment)"
        exit 1
    fi
    if [ -e "$KEYCHAIN" ]; then
        echo "Error: $KEYCHAIN exists already; run $0 cleanup first"
        exit 1
    fi
    plist="$RUNNER_TEMP/loader-profile.plist"
    security cms -D -i "$LOADER_PROFILE" > "$plist" 2>/dev/null || {
        echo "Error: $LOADER_PROFILE is not a signed provisioning profile"
        exit 1
    }
    team=$(/usr/libexec/PlistBuddy -c 'Print :TeamIdentifier:0' "$plist" 2>/dev/null) || {
        echo "Error: the provisioning profile names no TeamIdentifier"
        exit 1
    }
    rm -f "$plist"

    umask 077
    p12="$RUNNER_TEMP/signing.p12"
    trap 'rm -f "$p12"' EXIT
    printf '%s' "$MACOS_SIGNING_P12" | tr -d '[:space:]' | base64 --decode > "$p12"
    pw=$(openssl rand -hex 32)

    echo "==> Keychain $KEYCHAIN"
    security create-keychain -p "$pw" "$KEYCHAIN"
    security set-keychain-settings -lut 21600 "$KEYCHAIN"
    security unlock-keychain -p "$pw" "$KEYCHAIN"
    security import "$p12" -k "$KEYCHAIN" -f pkcs12 -P "$MACOS_SIGNING_P12_PASSWORD" \
        -T /usr/bin/codesign >/dev/null
    rm -f "$p12"
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$pw" "$KEYCHAIN" >/dev/null

    while IFS= read -r k; do
        if [ -n "$k" ] && [ "$k" != "$KEYCHAIN" ]; then existing+=("$k"); fi
    done < <(search_list)
    security list-keychains -d user -s "$KEYCHAIN" "${existing[@]}"

    if [ -z "$(team_identities "$team")" ]; then
        echo "    No valid identity of team $team yet; importing the Developer ID G2 intermediate"
        curl -fsSL --retry 3 -o "$RUNNER_TEMP/DeveloperIDG2CA.cer" "$DEVID_G2_URL"
        security import "$RUNNER_TEMP/DeveloperIDG2CA.cer" -k "$KEYCHAIN" >/dev/null
        rm -f "$RUNNER_TEMP/DeveloperIDG2CA.cer"
    fi
    if [ -z "$(team_identities "$team")" ]; then
        echo "Error: no valid code signing identity of team $team after the import:"
        security find-identity -p codesigning "$KEYCHAIN"
        exit 1
    fi
    echo "    Valid code signing identities of team $team: $(team_identities "$team" | wc -l | tr -d ' ')"
}

cmd_cleanup() {
    local k rest=()
    if [ -e "$KEYCHAIN" ]; then
        echo "==> Deleting $KEYCHAIN"
        security delete-keychain "$KEYCHAIN" || true
    fi
    # delete-keychain takes it out of the search list; a keychain file that
    # is gone may still be named there.
    if search_list | grep -qxF "$KEYCHAIN"; then
        while IFS= read -r k; do
            if [ -n "$k" ] && [ "$k" != "$KEYCHAIN" ]; then rest+=("$k"); fi
        done < <(search_list)
        if [ ${#rest[@]} -gt 0 ]; then
            security list-keychains -d user -s "${rest[@]}"
        fi
    fi
}

case "${1:-}" in
    setup) [ $# -eq 1 ] || usage; cmd_setup ;;
    cleanup) [ $# -eq 1 ] || usage; cmd_cleanup ;;
    *) usage ;;
esac
