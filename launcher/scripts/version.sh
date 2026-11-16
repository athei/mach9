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
#   launcher/scripts/version.sh --test-build <commit> <latest>
#       Print the CFBundleShortVersionString of a test build of <commit>:
#       the workspace version, a dash and the commit abbreviated to 11
#       digits. <latest> is the version of the latest published release,
#       whose CFBundleVersion the workspace version's has to be. Above it,
#       the test build would carry the CFBundleVersion of a release that
#       is not published yet, and Sparkle would never offer that release
#       to it; below it, Sparkle would offer <latest> at once.
#   launcher/scripts/version.sh --stamp [--short <short>] <bundle> [<version>]
#       Write CFBundleShortVersionString and CFBundleVersion into the
#       Contents/Info.plist of <bundle>, an app or a plug-in. <version>
#       defaults to the workspace version. CFBundleShortVersionString is
#       <version>, or <short> when given, which has to be <version>, a dash
#       and a commit, as --test-build prints it; CFBundleVersion is always
#       <version>'s.
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

# Exit unless $1 is the release version $2, a dash and an abbreviated commit.
check_short_version() {
  local commit="${1#"$2"-}"
  [[ "$1" == "$2-"* && "$commit" =~ ^[0-9a-f]{7,40}$ ]] \
    || die "'$1' is not the version $2 with a commit (X.Y.Z-<commit>)"
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
  --test-build)
    [[ $# -eq 3 ]] || usage
    [[ "$2" =~ ^[0-9a-f]{11,40}$ ]] || die "'$2' is not a commit hash of 11 digits or more"
    check_version "$3"
    version="$(workspace_version)"
    check_version "$version"
    case "$(compare_bundle_versions "$(bundle_version "$version")" "$(bundle_version "$3")")" in
      1) die "the workspace version $version is above $3, the latest release; a test build would keep Sparkle from offering $version once it is published" ;;
      -1) die "the workspace version $version is below $3, the latest release; Sparkle would replace a test build with $3 at once" ;;
    esac
    printf '%s-%s\n' "$version" "${2:0:11}" ;;
  --stamp)
    shift
    short=""
    if [[ "${1:-}" == --short ]]; then
      [[ -n "${2:-}" ]] || die "--short needs a version"
      short="$2"
      shift 2
    fi
    [[ $# -eq 1 || $# -eq 2 ]] || usage
    plist="${1%/}/Contents/Info.plist"
    [[ -f "$plist" ]] || die "no Contents/Info.plist in $1"
    version="${2:-$(workspace_version)}"
    check_version "$version"
    if [[ -n "$short" ]]; then
      check_short_version "$short" "$version"
    else
      short="$version"
    fi
    plist_set CFBundleShortVersionString "$short" "$plist"
    plist_set CFBundleVersion "$(bundle_version "$version")" "$plist" ;;
  -h|--help) usage 0 ;;
  *) usage ;;
esac
