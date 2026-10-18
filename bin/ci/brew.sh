#!/bin/bash
set -e

# The Homebrew formulae of a CI build, kept between runs. A hosted runner
# starts from its image every time, and installing the formulae in
# pins/brew-formulae with their dependencies takes long, so the first run on
# an image installs them and writes the kegs that added to a directory the
# workflow caches, and later runs on the same image unpack them instead:
#
#   brew.sh install <dir>   install the formulae, then write to <dir>:
#                             kegs.tar  every keg the install added, as
#                                       Cellar/<name>/<version>, with the
#                                       links in opt/ that point at it, and
#                                       the files the install added to etc/
#                                       and var/ or changed in etc/
#                             kegs      those kegs, as <name>/<version>
#                             linked    the names of those brew linked into
#                                       the prefix
#                             files     those files in etc/ and var/
#   brew.sh restore <dir>   unpack what install wrote into the prefix and
#                           link what it had linked
#   brew.sh check           exit unless pins/brew-formulae names every formula
#                           of DEPS_FORMULAE and MESA_FORMULAE (profile.sh),
#                           every formula it names is installed, and the
#                           tools the build takes from the prefix run
#
# install and restore end with check. A keg the install added is a version
# directory in the Cellar that was not there before it, whichever formula it
# belongs to, so the dependencies come along and the image's own kegs stay
# out. When the install upgraded a formula the image had, restore unlinks the
# image's version and links the cached one, as the install did; the old
# version stays in the Cellar unused, as brew install leaves it with
# HOMEBREW_NO_INSTALL_CLEANUP. A keg-only formula (llvm) gets its opt link
# and nothing in the prefix's bin/ or lib/, as with brew install.
#
# A cache only fits the runner image it was made on, whose Homebrew it
# extends, so the workflow keys it on the image version, the list and this
# script. It never touches a prefix other than DEPS_PREFIX, and it is meant
# for a runner, not for a Mac whose Homebrew somebody maintains.

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
# DEPS_PREFIX, DEPS_FORMULAE, MESA_FORMULAE and PINS_DIR.
# shellcheck source=../lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"

BREW="$DEPS_PREFIX/bin/brew"
LIST="$PINS_DIR/brew-formulae"
# No update of brew's formula data, which would make the result depend on
# the day instead of the image; no cleanup of old versions; no check of the
# formulae that depend on an upgraded one, which would reinstall the image's.
BREW_ENV=(env HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1
          HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK=1 HOMEBREW_NO_ENV_HINTS=1)

usage() {
    echo "Usage: $0 install <dir> | restore <dir> | check"
    exit 1
}

# formulae: print the formulae LIST names, one per line, without its
# comments.
formulae() {
    awk '{ sub(/#.*/, ""); for (i = 1; i <= NF; i++) print $i }' "$LIST"
}

# kegs: print every keg in the Cellar as <name>/<version>, sorted.
kegs() {
    local k
    for k in "$DEPS_PREFIX"/Cellar/*/*; do
        if [ -d "$k" ]; then echo "${k#"$DEPS_PREFIX"/Cellar/}"; fi
    done | LC_ALL=C sort
}

# prefix_files [<stamp>]: print every file and symlink under the prefix's
# etc/ and var/, brew's own var/homebrew left out, relative to the prefix and
# sorted; with <stamp>, only those under etc/ changed after it. Bottles
# install their configuration there, and post-install steps write there (the
# certificate bundle of ca-certificates, which openssl@3's and gnutls's
# cert.pem point at), outside the kegs. A changed file is taken from etc/
# only, where an upgrade rewrites one the image had; var/ holds logs and
# state that anything on the runner may touch, so only its new files count.
prefix_files() {
    local d dirs=() top="etc var"
    if [ -n "${1:-}" ]; then top=etc; fi
    for d in $top; do
        if [ -d "$DEPS_PREFIX/$d" ]; then dirs+=("$d"); fi
    done
    [ ${#dirs[@]} -gt 0 ] || return 0
    (cd "$DEPS_PREFIX" && find "${dirs[@]}" -path var/homebrew -prune -o \
        \( -type f -o -type l \) ${1:+-newer "$1"} -print) | LC_ALL=C sort
}

# check_tools: run each tool the build takes from the prefix, as the scripts
# find it, and ask pkg-config for the packages Wine's configure and
# build-mesa.sh look up, so a keg that is in place but does not work fails
# here and not hours into the build.
check_tools() {
    local t
    for t in meson cmake ninja pkg-config "$DEPS_PREFIX/opt/bison/bin/bison" \
             "$DEPS_PREFIX/opt/llvm/bin/llvm-config"; do
        if ! "$t" --version >/dev/null 2>&1; then
            echo "Error: $t --version fails:"
            "$t" --version || true
            exit 1
        fi
    done
    if ! PKG_CONFIG_PATH="$DEPS_PREFIX/lib/pkgconfig" pkg-config --exists \
            gnutls freetype2 sdl2 LLVMSPIRVLib SPIRV-Tools; then
        echo "Error: pkg-config does not find every package of gnutls freetype2 sdl2"
        echo "LLVMSPIRVLib SPIRV-Tools in $DEPS_PREFIX/lib/pkgconfig"
        exit 1
    fi
    echo "  meson, cmake, ninja, pkg-config, bison and llvm-config run, and pkg-config finds the packages."
}

check_brew() {
    if [ ! -x "$BREW" ]; then
        echo "Error: no Homebrew at $DEPS_PREFIX"
        exit 1
    fi
}

cmd_check() {
    local f absent="" missing=""
    for f in $DEPS_FORMULAE $MESA_FORMULAE; do
        formulae | grep -qxF "$f" || absent="$absent $f"
    done
    if [ -n "$absent" ]; then
        echo "Error: $LIST does not name$absent, which profile.sh lists"
        exit 1
    fi
    for f in $(formulae); do
        [ -d "$DEPS_PREFIX/opt/$f" ] || missing="$missing $f"
    done
    if [ -n "$missing" ]; then
        echo "Error: not installed in $DEPS_PREFIX:$missing"
        exit 1
    fi
    echo "  Every formula of $LIST is installed in $DEPS_PREFIX."
    check_tools
}

cmd_install() {
    local dir="$1" before before_files stamp added files f k name link paths=() list=()
    check_brew
    mkdir -p "$dir"
    if [ -n "$(ls -A "$dir")" ]; then
        echo "Error: $dir is not empty"
        exit 1
    fi
    read -ra list <<< "$(formulae | tr '\n' ' ')"
    if [ ${#list[@]} -eq 0 ]; then
        echo "Error: $LIST names no formula"
        exit 1
    fi
    before=$(kegs)
    before_files=$(prefix_files)
    stamp="$dir/.install-started"
    touch "$stamp"
    # A file the install writes gets a newer time than this; one it copies
    # from a bottle may keep the bottle's, and is found by its name instead.
    sleep 1
    echo "==> brew install ${list[*]}"
    "${BREW_ENV[@]}" "$BREW" install --formula "${list[@]}"
    added=$(LC_ALL=C comm -13 <(echo "$before") <(kegs))
    files=$({ LC_ALL=C comm -13 <(echo "$before_files") <(prefix_files)
              prefix_files "$stamp"; } | LC_ALL=C sort -u)
    rm -f "$stamp"

    echo "==> Writing the kegs the install added to $dir"
    : > "$dir/kegs"
    : > "$dir/linked"
    : > "$dir/files"
    while IFS= read -r k; do
        [ -n "$k" ] || continue
        name=${k%%/*}
        echo "    $k"
        echo "$k" >> "$dir/kegs"
        paths+=("Cellar/$k")
        # The opt link of the formula, and those of its aliases.
        for link in "$DEPS_PREFIX"/opt/*; do
            if [ -L "$link" ] && [ "$(readlink "$link")" = "../Cellar/$k" ]; then
                paths+=("opt/${link##*/}")
            fi
        done
        if [ "$(readlink "$DEPS_PREFIX/var/homebrew/linked/$name" 2>/dev/null)" = "../../../Cellar/$k" ]; then
            echo "$name" >> "$dir/linked"
        fi
    done <<< "$added"
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        echo "    $f"
        echo "$f" >> "$dir/files"
        paths+=("$f")
    done <<< "$files"
    if [ ${#paths[@]} -eq 0 ]; then
        echo "    (none: the image has every formula already)"
        tar -cf "$dir/kegs.tar" -T /dev/null
    else
        tar --no-mac-metadata -C "$DEPS_PREFIX" -cf "$dir/kegs.tar" "${paths[@]}"
    fi
    cmd_check
}

cmd_restore() {
    local dir="$1" f k name
    check_brew
    for f in kegs.tar kegs linked; do
        if [ ! -f "$dir/$f" ]; then
            echo "Error: no $f in $dir (write it with: $0 install $dir)"
            exit 1
        fi
    done
    echo "==> Restoring the kegs in $dir"
    while IFS= read -r k; do
        [ -n "$k" ] || continue
        name=${k%%/*}
        echo "    $k"
        if [ -e "$DEPS_PREFIX/Cellar/$k" ]; then
            echo "Error: $DEPS_PREFIX/Cellar/$k exists already, so the cache was not made on this image"
            exit 1
        fi
        # Another version of it is linked: the image's, which the install
        # upgraded.
        if [ -L "$DEPS_PREFIX/var/homebrew/linked/$name" ]; then
            "${BREW_ENV[@]}" "$BREW" unlink "$name"
        fi
    done < "$dir/kegs"
    tar -C "$DEPS_PREFIX" -xpf "$dir/kegs.tar"
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        "${BREW_ENV[@]}" "$BREW" link --overwrite "$name"
    done < "$dir/linked"
    cmd_check
}

case "${1:-}" in
    install|restore) [ $# -eq 2 ] || usage; "cmd_$1" "$2" ;;
    check) [ $# -eq 1 ] || usage; cmd_check ;;
    *) usage ;;
esac
