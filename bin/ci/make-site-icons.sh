#!/bin/bash
set -euo pipefail

# make-site-icons.sh: regenerate the browser icons of the landing page from
# docs/images/mach9.png, the app icon:
#
#   site/favicon.ico           16, 32 and 48 px, each stored as a PNG
#   site/apple-touch-icon.png  180 px
#
# Both are committed, since sips exists only on macOS and pages.yml runs on
# Linux; assemble-site.sh copies them into the site. Run this whenever
# docs/images/mach9.png changes, and commit the result.
#
# macOS only: needs sips and python3.

die() {
    echo "Error: $*" >&2
    exit 1
}

[ $# -eq 0 ] || die "takes no arguments"
[ "$(uname)" = Darwin ] || die "needs macOS for sips"

REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="$REPO_DIR/docs/images/mach9.png"
SITE="$REPO_DIR/site"
[ -f "$SRC" ] || die "no icon at $SRC"

TMP_DIR="$(mktemp -d -t site-icons)"
trap 'rm -rf "$TMP_DIR"' EXIT

for size in 16 32 48; do
    sips -z "$size" "$size" "$SRC" --out "$TMP_DIR/$size.png" >/dev/null
done
sips -z 180 180 "$SRC" --out "$SITE/apple-touch-icon.png" >/dev/null

# An ICO file is a header, one directory entry per image and the images.
# Since Windows Vista an image may be a whole PNG file, which every browser
# reads.
python3 -I - "$SITE/favicon.ico" "$TMP_DIR/16.png" "$TMP_DIR/32.png" "$TMP_DIR/48.png" <<'EOF'
import struct
import sys

out, pngs = sys.argv[1], [open(p, "rb").read() for p in sys.argv[2:]]
header = struct.pack("<HHH", 0, 1, len(pngs))
entries, offset = b"", 6 + 16 * len(pngs)
for png in pngs:
    if png[:8] != b"\x89PNG\r\n\x1a\n" or png[12:16] != b"IHDR":
        sys.exit("not a PNG file")
    width, height = struct.unpack(">II", png[16:24])
    if not (0 < width <= 256 and 0 < height <= 256):
        sys.exit(f"{width}x{height} does not fit in an ICO entry")
    # 256 is stored as 0. Colour count 0, reserved 0, 1 plane, 32 bits.
    entries += struct.pack("<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(png), offset)
    offset += len(png)
with open(out, "wb") as f:
    f.write(header + entries + b"".join(pngs))
EOF

echo "Wrote $SITE/favicon.ico and $SITE/apple-touch-icon.png from $SRC"
