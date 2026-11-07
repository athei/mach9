# Shared by the scripts in this directory. Sourced, never run:
#
#   source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
#
# Sets REPO_ROOT, LAUNCHER_DIR, BUILD_DIR, SPARKLE_DIR, SPARKLE_BIN, PLIST,
# INFO_REL and INFO_SRC and defines usage, die, require_macos and the version
# helpers. Written for the bash 3.2 that ships with macOS, like the scripts
# that source it.

LAUNCHER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$LAUNCHER_DIR/.." && pwd)"
PLIST=/usr/libexec/PlistBuddy

# The app's Info.plist, which carries the identifier, the feed URL and the
# public key.
INFO_REL=launcher/app/Info.plist
INFO_SRC="$REPO_ROOT/$INFO_REL"

# Where the Makefile and these scripts put what they build and download,
# under the repository's ignored target/. SPARKLE_DIR in the environment
# overrides where the pinned Sparkle release goes (fetch-sparkle.sh), and is
# the same directory the launcher's build.rs links against.
BUILD_DIR="$REPO_ROOT/target/launcher"
SPARKLE_DIR="${SPARKLE_DIR:-$BUILD_DIR/sparkle}"
SPARKLE_BIN="$SPARKLE_DIR/bin"

# The script that sourced this file, whose header usage prints.
LIB_CALLER="${BASH_SOURCE[1]}"

# Print the comment block at the top of the calling script (everything after
# the shebang up to the first line that is not a comment) and exit. With no
# argument it is the answer to a usage mistake: printed to stderr, exit 64.
# `usage 0` is the answer to --help: printed to stdout, exit 0.
usage() {
  local status="${1:-64}"
  local header
  header="$(awk 'NR > 1 && !/^#/ { exit } NR > 1 { sub(/^# ?/, ""); print }' "$LIB_CALLER")"
  if [[ "$status" -eq 0 ]]; then
    printf '%s\n' "$header"
  else
    printf '%s\n' "$header" >&2
  fi
  exit "$status"
}

die() {
  echo "error: $*" >&2
  exit 1
}

# Exit unless running on macOS. The optional argument names what the script
# needs from it, for the message.
require_macos() {
  [[ "$(uname)" == "Darwin" ]] || die "this script requires macOS${1:+ ($1)}"
}

# --- versions -------------------------------------------------------------------
#
# A release version is X.Y.Z, or X.Y.Z-rc.N for a release candidate of X.Y.Z,
# and is the tag vX.Y.Z or vX.Y.Z-rc.N without the v. Its one source is the
# workspace version in the root Cargo.toml, which the release commit sets to
# X.Y.Z; a release candidate of X.Y.Z is cut while it says X.Y.Z.
#
# The bundle carries the release version as CFBundleShortVersionString and
# bundle_version's form of it as CFBundleVersion, which is what Sparkle
# compares.

# Exit unless $1 is a release version.
check_version() {
  [[ "$1" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-rc\.[1-9][0-9]*)?$ ]] \
    || die "'$1' is not a release version (X.Y.Z or X.Y.Z-rc.N)"
}

# The X.Y.Z a release version is, or is a release candidate of.
base_version() {
  printf '%s' "${1%-rc.*}"
}

# The CFBundleVersion of a release version: X.Y.Z as it is, X.Y.Z-rc.N as
# X.Y.ZrcN. Sparkle's comparator (SUStandardVersionComparator) stops reading
# a version at its first dash, so X.Y.Z-rc.N would compare equal to X.Y.Z.
# In X.Y.ZrcN, "rc" is a string part after the numbers, and a version with
# such a part sorts below the same numbers without it, so
# X.Y.Z-rc.N < X.Y.Z and earlier releases sort below both.
bundle_version() {
  case "$1" in
    *-rc.*) printf '%src%s' "${1%-rc.*}" "${1##*-rc.}" ;;
    *) printf '%s' "$1" ;;
  esac
}

# Compare two CFBundleVersions the way Sparkle does and print 1, 0 or -1 for
# $1 above, equal to or below $2. Only the forms bundle_version produces, and
# any count of numbers, are accepted (N(.N)* with an optional rcN), for which
# this matches SUStandardVersionComparator: missing numbers count as 0, and
# the same numbers with rcN sort below them without it.
compare_bundle_versions() {
  local a="$1" b="$2" a_rc="" b_rc="" i x y n
  local -a an bn
  [[ "$a" =~ ^[0-9]+(\.[0-9]+)*(rc[0-9]+)?$ ]] || die "cannot compare the version '$a'"
  [[ "$b" =~ ^[0-9]+(\.[0-9]+)*(rc[0-9]+)?$ ]] || die "cannot compare the version '$b'"
  if [[ "$a" == *rc* ]]; then a_rc="${a##*rc}"; a="${a%rc*}"; fi
  if [[ "$b" == *rc* ]]; then b_rc="${b##*rc}"; b="${b%rc*}"; fi
  IFS=. read -r -a an <<<"$a"
  IFS=. read -r -a bn <<<"$b"
  n="${#an[@]}"
  if (( ${#bn[@]} > n )); then n="${#bn[@]}"; fi
  for ((i = 0; i < n; i++)); do
    x="$((10#${an[i]:-0}))"
    y="$((10#${bn[i]:-0}))"
    if (( x > y )); then echo 1; return; fi
    if (( x < y )); then echo -1; return; fi
  done
  if [[ -z "$a_rc" && -z "$b_rc" ]]; then echo 0
  elif [[ -z "$a_rc" ]]; then echo 1
  elif [[ -z "$b_rc" ]]; then echo -1
  elif (( 10#$a_rc > 10#$b_rc )); then echo 1
  elif (( 10#$a_rc < 10#$b_rc )); then echo -1
  else echo 0
  fi
}

# The workspace version in the root Cargo.toml, which every crate inherits,
# read through cargo so that whatever cargo accepts is read correctly.
workspace_version() {
  local json version
  json="$(cargo metadata --no-deps --format-version 1 --manifest-path "$REPO_ROOT/Cargo.toml")" \
    || die "cargo metadata failed in $REPO_ROOT"
  version="$(jq -r '.packages[] | select(.name == "gamelauncher") | .version' <<<"$json")"
  [[ -n "$version" ]] || die "cargo metadata names no gamelauncher package"
  printf '%s' "$version"
}
