# shellcheck shell=bash
# How the licence texts of what the bundle ships get into its share/licenses,
# for the build scripts that stage them into their dists (build-fex.sh,
# build-mesa.sh, build-gstreamer.sh, build-dxmt.sh, build-vkd3d-proton.sh)
# and for bundle-wine.sh, which source this file after profile.sh. Each build
# script hashes it into its KEY (dist_key --hash). It is not part of
# profile.sh, whose hash is in the KEY of every staged dist, so a change here
# rebuilds only the dists that stage licences.
#
# A build script stages the texts of its component, and of what the component
# links statically, out of the tree it builds from into
# <dist>/share/licenses/<component>/, and the bundle step copies that
# directory into the bundle's share/licenses. The texts come from the pinned
# tree on every build, so a pin whose tree moved or dropped a text stops the
# build instead of shipping a stale copy.

# license_notice <file>: print the comment block <file> starts with, for code
# whose licence is only in its file headers: a /* */ block up to the line that
# closes it, or the run of // lines. Blank lines before it are skipped and
# carriage returns dropped. Prints nothing when <file> does not start with a
# comment.
license_notice() {
    awk '
        { sub(/\r$/, "") }
        !started && /^[[:space:]]*$/ { next }
        !started { started = 1; if ($0 ~ /^\/\*/) block = 1 }
        block { print; if ($0 ~ /\*\//) exit; next }
        /^\/\// { print; next }
        { exit }
    ' "$1"
}

# stage_licenses <tree> <dest> <list>: stage the licence texts <list> names
# from <tree> into <dest>. <list> has one entry per line, leading blanks
# ignored:
#   <name>=<path>          the file <path> as <dest>/<name>, or, when <path>
#                          is a directory, its files under <dest>/<name>/
#   <name>=notice:<path>   the comment block the source file <path> starts
#                          with (license_notice), which has to name a
#                          copyright
# <path> is relative to <tree>. Exits when a text is missing or empty.
stage_licenses() {
    local tree="$1" dest="$2" entry name path
    mkdir -p "$dest"
    while IFS= read -r entry; do
        entry="${entry#"${entry%%[![:space:]]*}"}"
        [ -n "$entry" ] || continue
        name=${entry%%=*}
        path=${entry#*=}
        case "$path" in
            notice:*)
                path=${path#notice:}
                if [ ! -f "$tree/$path" ]; then
                    echo "Error: no $tree/$path, whose header holds a licence"
                    exit 1
                fi
                license_notice "$tree/$path" > "$dest/$name"
                if ! grep -qi copyright "$dest/$name"; then
                    echo "Error: $tree/$path does not start with a comment that names a copyright"
                    exit 1
                fi
                ;;
            *)
                if [ -d "$tree/$path" ]; then
                    mkdir -p "$dest/$name"
                    cp -R "$tree/$path/." "$dest/$name/"
                    if [ -z "$(find "$dest/$name" -type f -size +0)" ]; then
                        echo "Error: the licence directory $tree/$path holds no text"
                        exit 1
                    fi
                elif [ -s "$tree/$path" ]; then
                    cp "$tree/$path" "$dest/$name"
                else
                    echo "Error: no licence text $tree/$path"
                    exit 1
                fi
                ;;
        esac
    done <<< "$3"
}
