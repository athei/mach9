#!/usr/bin/env bash
#
# The app's version, whose one source is the workspace version in the root
# Cargo.toml (lib.sh says how a release version looks).
#
# Usage:
#   launcher/scripts/version.sh
#       Print the workspace version.
#   launcher/scripts/version.sh --bundle <version>
#       Print the CFBundleVersion of a release version.
#   launcher/scripts/version.sh --stamp <bundle> [<version>]
#       Write CFBundleShortVersionString and CFBundleVersion into the
#       Contents/Info.plist of <bundle>, an app or a plug-in. <version>
#       defaults to the workspace version.
#
# The Makefile stamps every bundle it assembles, so launcher/app/Info.plist
# and launcher/docktile/Info.plist carry no version of their own.

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# Set a string key, whether the plist has it or not.
plist_set() {
  $PLIST -c "Delete :$1" "$3" >/dev/null 2>&1 || true
  $PLIST -c "Add :$1 string $2" "$3"
}

case "${1:-}" in
  "")
    workspace_version
    echo ;;
  --bundle)
    [[ $# -eq 2 ]] || usage
    check_version "$2"
    bundle_version "$2"
    echo ;;
  --stamp)
    [[ $# -eq 2 || $# -eq 3 ]] || usage
    plist="${2%/}/Contents/Info.plist"
    [[ -f "$plist" ]] || die "no Contents/Info.plist in $2"
    version="${3:-$(workspace_version)}"
    check_version "$version"
    plist_set CFBundleShortVersionString "$version" "$plist"
    plist_set CFBundleVersion "$(bundle_version "$version")" "$plist" ;;
  -h|--help) usage 0 ;;
  *) usage ;;
esac
