# Shared by the scripts in this directory. Sourced, never run:
#
#   source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
#
# Sets REPO_ROOT, LAUNCHER_DIR, BUILD_DIR, SPARKLE_DIR, SPARKLE_BIN, PLIST,
# INFO_REL and INFO_SRC and defines usage, die and require_macos. Written for
# the bash 3.2 that ships with macOS, like the scripts that source it.

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
