#!/bin/bash
set -eo pipefail

# pins-key.sh <name-prefix>...: print the SHA-256 of the lines of redist.env
# that set a variable whose name starts with one of the <name-prefix>es, such
# as MTLD3D_. A cache key made from it changes with those pins only, not with
# the rest of the file. Fails when no line matches, so a renamed pin cannot
# leave a key that never changes again.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# PINS_DIR.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

if [ $# -eq 0 ]; then
    echo "Usage: $0 <name-prefix>..." >&2
    exit 1
fi
pattern="^($(IFS='|'; echo "$*"))[A-Za-z0-9_]*="
if ! lines=$(grep -E "$pattern" "$PINS_DIR/redist.env"); then
    echo "Error: no pin in $PINS_DIR/redist.env starts with $*" >&2
    exit 1
fi
printf '%s\n' "$lines" | shasum -a 256 | cut -d' ' -f1
