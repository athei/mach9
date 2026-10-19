#!/bin/bash
set -eo pipefail

# pins-key.sh <name-prefix>...: print the SHA-256 of the lines of redist.env
# that set a variable whose name starts with one of the <name-prefix>es, such
# as MTLD3D_. A cache key made from it changes with those pins only, not with
# the rest of the file. Fails when one of the prefixes matches no line, so a
# renamed pin cannot drop out of a key and leave it unchanged when that pin
# changes.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# PINS_DIR.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

if [ $# -eq 0 ]; then
    echo "Usage: $0 <name-prefix>..." >&2
    exit 1
fi
for prefix in "$@"; do
    if ! grep -qE "^${prefix}[A-Za-z0-9_]*=" "$PINS_DIR/redist.env"; then
        echo "Error: no pin in $PINS_DIR/redist.env starts with $prefix" >&2
        exit 1
    fi
done
pattern="^($(IFS='|'; echo "$*"))[A-Za-z0-9_]*="
lines=$(grep -E "$pattern" "$PINS_DIR/redist.env")
printf '%s\n' "$lines" | shasum -a 256 | cut -d' ' -f1
