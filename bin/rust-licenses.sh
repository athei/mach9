#!/bin/bash
set -e

# Stages the licence texts of what the Rust packages given compile into their
# binaries for <target>, into <dest>:
#
#   rust-std/                 the notice of Rust's standard library
#                             (COPYRIGHT-library.html) and the licence texts
#                             it names, from the toolchain's share/doc/rust
#   <crate>-<version>/        each crate's licence files, as the crate
#                             publishes them (LICENSE*, LICENCE*, COPYING*,
#                             COPYRIGHT*, NOTICE*, UNLICENSE*)
#
# The crates are what `cargo tree` lists for the target along normal
# dependencies, without proc macros: build and dev dependencies and proc
# macros run at build time and leave nothing in the binary. Packages of this
# workspace are Mach9's own, under its LICENSE, which the caller installs.
# A crate that publishes no licence file takes the text WITHOUT_FILES names
# from this repository's licenses/; any other crate without one, and a crate
# whose sources are not in cargo's registry, stops the script. Run it after
# a build of the packages, which unpacked their crates; it does not fetch.
#
# Usage: bin/rust-licenses.sh <dest> <target> <package>...
# Called by bundle-wine.sh for compatdb.so and by launcher/Makefile for the
# launcher and its Dock tile plug-in.

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# The crates that publish no licence file, as <crate>:<file in licenses/>.
# The objc2 crates name their licence in Cargo.toml only; licenses/objc2.txt
# has the terms their repository gives.
WITHOUT_FILES="objc2:objc2.txt objc2-encode:objc2.txt block2:objc2.txt
objc2-foundation:objc2.txt objc2-core-foundation:objc2.txt objc2-app-kit:objc2.txt"

if [ $# -lt 3 ]; then
    echo "Usage: $0 <dest> <target> <package>..."
    exit 1
fi
DEST="$1"
TARGET="$2"
shift 2
PACKAGES=()
for p in "$@"; do
    PACKAGES+=(-p "$p")
done
REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry/src"

mkdir -p "$DEST"
# The toolchain cargo builds with in the workspace.
docs="$(cd "$REPO_DIR" && rustc --print sysroot)/share/doc/rust"
if [ ! -s "$docs/COPYRIGHT-library.html" ] || [ ! -d "$docs/licenses" ]; then
    echo "Error: the Rust toolchain has no $docs/COPYRIGHT-library.html and licenses/"
    exit 1
fi
mkdir -p "$DEST/rust-std"
cp "$docs/COPYRIGHT-library.html" "$DEST/rust-std/"
cp -R "$docs/licenses" "$DEST/rust-std/"

# One "<name> v<version>" line per crate; path packages carry their path in
# parentheses, and a crate listed before is marked (*). cargo runs on its
# own, so its failure stops the script instead of leaving an empty list.
if ! tree=$(cd "$REPO_DIR" && cargo tree --quiet --locked --offline -e normal,no-proc-macro \
        --target "$TARGET" "${PACKAGES[@]}" --prefix none --format '{p}'); then
    echo "Error: cargo tree cannot list the crates of $* for $TARGET"
    exit 1
fi
crates=$(sed 's/ (\*)$//' <<< "$tree" | grep -v -e '^$' -e ' (/' | LC_ALL=C sort -u)
if [ -z "$crates" ]; then
    echo "Error: cargo tree lists no crate for $* on $TARGET"
    exit 1
fi
count=0
while read -r name version; do
    [ -n "$name" ] || continue
    version=${version#v}
    src=""
    for dir in "$REGISTRY"/*/"$name-$version"; do
        if [ -f "$dir/Cargo.toml" ]; then
            src="$dir"
            break
        fi
    done
    if [ -z "$src" ]; then
        echo "Error: $name $version is not unpacked in $REGISTRY (build the packages first)"
        exit 1
    fi
    out="$DEST/$name-$version"
    mkdir -p "$out"
    found=0
    for f in "$src"/*; do
        case "$(basename "$f" | tr '[:upper:]' '[:lower:]')" in
            licen[cs]e*|copying*|copyright*|notice*|unlicense*)
                if [ -f "$f" ] && [ -s "$f" ]; then
                    cp "$f" "$out/"
                    found=1
                fi
                ;;
        esac
    done
    if [ "$found" -eq 0 ]; then
        for entry in $WITHOUT_FILES; do
            if [ "${entry%%:*}" = "$name" ]; then
                cp "$REPO_DIR/licenses/${entry#*:}" "$out/"
                found=1
            fi
        done
    fi
    if [ "$found" -eq 0 ]; then
        echo "Error: the crate $name $version publishes no licence file, and WITHOUT_FILES in"
        echo "$0 names no text for it"
        exit 1
    fi
    count=$((count + 1))
done <<< "$crates"
echo "    licences: Rust's standard library and $count crates in $DEST"
