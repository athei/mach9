#!/bin/bash
set -o pipefail

# tag-forks.sh [--check] [--sources <file>] [--checkout <dir>]... <tag>
#
# Tags each fork the release <tag> (vX.Y.Z or vX.Y.Z-rc.N) was built from
# as mach9-<tag>, at the commit the release names, so that commit can still
# be fetched after the fork's branch is rebased and force-pushed. A release
# candidate's forks get a tag of their own, since its DMG is published like
# a release's.
#
# The commits come from the release's sources.txt (bin/ci/package.sh),
# which the tag run attaches to the draft: one "<name> <repository>
# <commit>" line per source the bundle was built from, in CI. A record with
# a line of another form (sources_line_ok in sources.sh), or with a name or
# a repository twice, is refused. A fork is a source on GitHub whose owner
# is the owner of the Mach9 line's repository, other than that repository
# itself. The other sources are pinned in redist.env and get no tag. The
# Mach9 repository has to have <tag> at the Mach9 commit, or the record is
# not the release's and nothing is tagged.
#
# For a fork without the tag, the commit is pushed as the tag from the
# local checkout whose origin is the fork, with the owner's credentials:
# one of the --checkout directories, or of ROOT/src and ROOT/src-*
# (profile.sh), searched in that order. One whose origin pushes elsewhere
# (remote.origin.pushurl, pushInsteadOf) is refused. A checkout that lacks
# the commit fetches it from its origin first, which changes only its
# FETCH_HEAD, and one that still has a commit the fork lost pushes it back
# with the tag.
# A tag at the commit already is left as it is. A tag at another commit is
# never moved: that is an error (docs/maintenance.md, cutting a release).
# The tags are then checked as --check does.
#
# --check changes nothing: it checks that the release tag and every fork's
# tag are at the commits of the record, reading the repositories it names.
# --sources <file> reads the record from <file> instead of downloading the
# release's sources.txt with gh from this repository's origin.
#
# Every problem is reported, then the script exits 1 when there was one,
# and 2 on a usage error.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# ROOT and REPO_DIR.
# shellcheck source=lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"
# github_name, mach9_repo and sources_line_ok.
# shellcheck source=lib/sources.sh
. "$SCRIPT_DIR/lib/sources.sh"

usage() {
    echo "Usage: $0 [--check] [--sources <file>] [--checkout <dir>]... <tag>" >&2
    exit 2
}

CHECK_ONLY=0
SOURCES_FILE=""
CHECKOUTS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --check) CHECK_ONLY=1; shift ;;
        --sources|--checkout)
            [ $# -ge 2 ] || usage
            if [ "$1" = --sources ]; then SOURCES_FILE=$2; else CHECKOUTS+=("$2"); fi
            shift 2
            ;;
        -*) usage ;;
        *) break ;;
    esac
done
[ $# -eq 1 ] || usage
TAG=$1
# The tags a tag run builds a release of (release.yml, Check the tag).
if ! [[ "$TAG" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-rc\.[1-9][0-9]*)?$ ]]; then
    echo "Error: $TAG is neither vX.Y.Z nor vX.Y.Z-rc.N" >&2
    exit 2
fi
FORK_TAG="mach9-$TAG"
for d in "$ROOT/src" "$ROOT"/src-*; do
    [ -d "$d" ] && CHECKOUTS+=("$d")
done

# short <commit>: the commit abbreviated as the release notes write it.
short() {
    echo "${1:0:11}"
}

# repo_key <url>: <url> in the form two names of one repository share:
# owner/name of a GitHub URL in any of its forms, else the URL without a
# trailing .git, in lower case.
repo_key() {
    github_name "$1" | tr '[:upper:]' '[:lower:]'
}

# github_owner <url>: the owner of a GitHub repository URL in lower case,
# or nothing for another host.
github_owner() {
    local n
    n=$(github_name "$1")
    [ "$n" != "${1%.git}" ] || return 0
    echo "${n%%/*}" | tr '[:upper:]' '[:lower:]'
}

# remote_tag <url> <tag>: print the commit the tag <tag> of the repository
# <url> names (peeled, for an annotated one), or nothing when it has no such
# tag. Fails when the repository cannot be read; git is not allowed to ask
# for credentials. ls-remote prints the peeled line of an annotated tag only
# for a pattern that matches its name with the ^{}.
remote_tag() {
    local out
    out=$(GIT_TERMINAL_PROMPT=0 git ls-remote "$1" "refs/tags/$2" "refs/tags/$2^{}") || return 1
    awk -v t="refs/tags/$2" '
        $2 == t "^{}" { peeled = $1 }
        $2 == t { tag = $1 }
        END { if (peeled != "") print peeled; else print tag }' <<< "$out"
}

# find_checkout <url>: print the first of CHECKOUTS that is the top level of
# a git work tree whose remote origin is the repository <url>, as the
# configuration names it, before any insteadOf rewriting.
find_checkout() {
    local d url key
    key=$(repo_key "$1")
    for d in "${CHECKOUTS[@]}"; do
        [ "$(git -C "$d" rev-parse --show-toplevel 2>/dev/null)" -ef "$d" ] || continue
        url=$(git -C "$d" config --get remote.origin.url) || continue
        if [ "$(repo_key "$url")" = "$key" ]; then
            echo "$d"
            return 0
        fi
    done
    return 1
}

# check_tag <name> <url> <tag> <commit>: report whether the repository <url>
# of the source <name> has <tag> at <commit>; fail when it does not.
check_tag() {
    local name="$1" url="$2" tag="$3" commit="$4" have
    if ! have=$(remote_tag "$url" "$tag"); then
        echo "  ERROR: $name: cannot read the tags of $url"
        return 1
    fi
    if [ -z "$have" ]; then
        echo "  ERROR: $name: $url has no tag $tag"
        return 1
    fi
    if [ "$have" != "$commit" ]; then
        echo "  ERROR: $name: $url has $tag at $have, not at $commit"
        return 1
    fi
    echo "  $name: $tag at $(short "$commit")"
}

# tag_fork <name> <url> <commit>: give the fork <url> the tag FORK_TAG at
# <commit> (header); fail when it cannot.
tag_fork() {
    local name="$1" url="$2" commit="$3" have co push
    if ! have=$(remote_tag "$url" "$FORK_TAG"); then
        echo "  ERROR: $name: cannot read the tags of $url"
        return 1
    fi
    if [ "$have" = "$commit" ]; then
        echo "  $name: $FORK_TAG is at $(short "$commit") already"
        return 0
    fi
    if [ -n "$have" ]; then
        echo "  ERROR: $name: $url has $FORK_TAG at $have, not at $commit;"
        echo "  a tag is never moved (docs/maintenance.md, cutting a release)"
        return 1
    fi
    if ! co=$(find_checkout "$url"); then
        echo "  ERROR: $name: no checkout of $url to push the tag from among"
        echo "  $ROOT/src and $ROOT/src-* (clone it there, or pass --checkout <dir>)"
        return 1
    fi
    # The push goes where origin pushes to, after remote.origin.pushurl and
    # pushInsteadOf: it has to be the fork, as <url> names it or as the
    # checkout's insteadOf rewrites <url>.
    push=$(git -C "$co" remote get-url --push origin)
    if [ "$(repo_key "$push")" != "$(repo_key "$url")" ] &&
       [ "$(repo_key "$push")" != "$(repo_key "$(git -C "$co" ls-remote --get-url "$url")")" ]; then
        echo "  ERROR: $name: origin of $co pushes to $push, not to $url"
        return 1
    fi
    if ! git -C "$co" cat-file -e "$commit^{commit}" 2>/dev/null; then
        echo "  $name: fetching $commit into $co"
        git -C "$co" fetch --quiet --no-tags --no-recurse-submodules origin "$commit"
        if ! git -C "$co" cat-file -e "$commit^{commit}" 2>/dev/null; then
            echo "  ERROR: $name: commit $commit is not in $co, and its origin"
            echo "  does not give it out: the source the release names is lost unless"
            echo "  another clone has it (docs/maintenance.md, cutting a release)"
            return 1
        fi
    fi
    # Only the tag: no annotated tags push.followTags would add, and no
    # submodule commits push.recurseSubmodules would.
    if ! git -C "$co" push --quiet --no-follow-tags --recurse-submodules=no origin "$commit:refs/tags/$FORK_TAG"; then
        echo "  ERROR: $name: pushing $FORK_TAG from $co failed (credentials for its origin?)"
        return 1
    fi
    echo "  $name: tagged $(short "$commit") as $FORK_TAG from $co"
}

if [ -z "$SOURCES_FILE" ]; then
    self=$(github_name "$(mach9_repo)")
    if [ -z "$self" ]; then
        echo "Error: $REPO_DIR has no origin to download the release from; pass --sources" >&2
        exit 1
    fi
    TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/tag-forks.XXXXXX") || exit 1
    trap 'rm -rf "$TMP_DIR"' EXIT
    echo "==> sources.txt of the release $TAG of $self"
    if ! gh release download "$TAG" --repo "$self" --pattern sources.txt --dir "$TMP_DIR"; then
        echo "Error: cannot download sources.txt of the release $TAG of $self;" >&2
        echo "download it from the release's page and pass --sources <file>" >&2
        echo "(a release built before package.sh wrote it has none)" >&2
        exit 1
    fi
    SOURCES_FILE=$TMP_DIR/sources.txt
fi
if [ ! -f "$SOURCES_FILE" ]; then
    echo "Error: no $SOURCES_FILE" >&2
    exit 1
fi

# The record: every line valid (sources_line_ok), no name and no repository
# twice, so no source can be skipped or replaced unnoticed. Empty lines are
# skipped.
NAMES=()
URLS=()
COMMITS=()
SELF_URL=""
SELF_COMMIT=""
seen_names=" "
seen_repos=" "
line=0
while IFS= read -r text || [ -n "$text" ]; do
    line=$((line + 1))
    [ -n "$text" ] || continue
    if ! sources_line_ok "$text"; then
        echo "Error: $SOURCES_FILE:$line is not \"<name> <https repository URL> <full commit>\"" >&2
        exit 1
    fi
    read -r name url commit <<< "$text"
    key=$(repo_key "$url")
    case "$seen_names" in
        *" $name "*)
            echo "Error: $SOURCES_FILE:$line names $name a second time" >&2
            exit 1
            ;;
    esac
    case "$seen_repos" in
        *" $key "*)
            echo "Error: $SOURCES_FILE:$line names the repository $url a second time" >&2
            exit 1
            ;;
    esac
    seen_names="$seen_names$name "
    seen_repos="$seen_repos$key "
    if [ "$name" = Mach9 ]; then
        SELF_URL=$url
        SELF_COMMIT=$commit
    else
        NAMES+=("$name")
        URLS+=("$url")
        COMMITS+=("$commit")
    fi
done < "$SOURCES_FILE"
OWNER=$(github_owner "$SELF_URL")
if [ -z "$OWNER" ]; then
    echo "Error: $SOURCES_FILE has no Mach9 line with a GitHub repository" >&2
    exit 1
fi

# The forks, by index into NAMES; the other sources are listed and left.
FORKS=()
echo "==> Sources of $TAG ($SOURCES_FILE)"
for i in "${!NAMES[@]}"; do
    if [ "$(github_owner "${URLS[$i]}")" = "$OWNER" ] && [ "$(repo_key "${URLS[$i]}")" != "$(repo_key "$SELF_URL")" ]; then
        FORKS+=("$i")
        echo "  ${NAMES[$i]}: ${URLS[$i]} at $(short "${COMMITS[$i]}"), a fork"
    else
        echo "  ${NAMES[$i]}: ${URLS[$i]} at $(short "${COMMITS[$i]}"), not a repository of $OWNER: no tag"
    fi
done
if [ ${#FORKS[@]} -eq 0 ]; then
    echo "Error: $SOURCES_FILE names no repository of $OWNER besides $SELF_URL" >&2
    exit 1
fi

echo "==> The release tag"
if ! check_tag Mach9 "$SELF_URL" "$TAG" "$SELF_COMMIT"; then
    echo "Error: $SOURCES_FILE is not the record of the release $TAG; nothing is tagged" >&2
    exit 1
fi

BAD=0
if [ "$CHECK_ONLY" -eq 0 ]; then
    echo "==> Tagging the forks as $FORK_TAG"
    for i in "${FORKS[@]}"; do
        tag_fork "${NAMES[$i]}" "${URLS[$i]}" "${COMMITS[$i]}" || BAD=1
    done
fi
echo "==> Checking $FORK_TAG"
for i in "${FORKS[@]}"; do
    check_tag "${NAMES[$i]}" "${URLS[$i]}" "$FORK_TAG" "${COMMITS[$i]}" || BAD=1
done
if [ "$BAD" -ne 0 ]; then
    echo "Error: not every fork of $TAG has $FORK_TAG at the commit the release names" >&2
    exit 1
fi
echo "Every fork of $TAG has $FORK_TAG at the commit the release names."
