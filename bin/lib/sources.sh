# shellcheck shell=bash
# The sources the bundle is built from, for the release notes and
# sources.txt of bin/ci/package.sh and the source notice Step 4c of
# bundle-wine.sh writes (write_source_notice). Both source this file after
# profile.sh and redist.env; bin/tag-forks.sh, which reads sources.txt, for
# its helpers. No build script reads it, so it is in no dist's KEY.

# GIT_SOURCES: the components besides Wine that the build scripts fetch from
# a git repository, one per line as
#   <name>:<repository var>:<ref var>:<dist var>/<commit file>:<texts>
# where each <... var> names a shell variable.
# <texts> is empty for a component the bundle ships under a permissive
# licence. For one it ships under the LGPL, it is its licence texts, comma
# separated, as Step 4c installs them under share/licenses, and the source
# notice has a section for it.
GIT_SOURCES="FEX:FEX_REPO:FEX_REF:FEX_DIST/COMMIT:
Mesa:MESA_REPO:MESA_REF:MESA_DIST/COMMIT:
DXMT:DXMT_REPO:DXMT_REF:DXMT_DIST/COMMIT:dxmt/LICENSE,dxmt/COPYING.LIB
vkd3d-proton:VKD3D_PROTON_REPO:VKD3D_PROTON_REF:VKD3D_PROTON_DIST/COMMIT:vkd3d-proton/LICENSE,vkd3d-proton/COPYING
GStreamer:GSTREAMER_GIT_URL:GSTREAMER_GIT_TAG:GSTREAMER_DIST/COMMIT:gstreamer/gstreamer.txt,gstreamer/gst-plugins-base.txt,gstreamer/gst-plugins-good.txt,gstreamer/gst-plugins-ugly.txt,gstreamer/gst-plugins-bad.txt,gstreamer/gst-libav.txt
FFmpeg:GSTREAMER_FFMPEG_GIT_URL:GSTREAMER_FFMPEG_GIT_BRANCH:GSTREAMER_DIST/FFMPEG_COMMIT:gstreamer/ffmpeg.md,gstreamer/ffmpeg-COPYING.LGPLv2.1"

# github_name <url>: owner/name of a GitHub repository URL, or the URL as
# given when it is not one.
github_name() {
    local n="${1%.git}"
    n=${n#https://github.com/}
    n=${n#git@github.com:}
    echo "$n"
}

# public_url <url>: the https URL of a GitHub repository, whatever form
# <url> names it in, or <url> as given for another host.
public_url() {
    local n
    n=$(github_name "$1")
    if [ "$n" = "${1%.git}" ]; then
        echo "$1"
    else
        echo "https://github.com/$n"
    fi
}

# source_commit <dist variable>/<file>: print the commit a build script wrote
# into that file of its dist, or nothing.
source_commit() {
    local var="${1%%/*}"
    cat "${!var}/${1#*/}" 2>/dev/null
}

# repo_has_commit <url> <commit>: succeed when <commit> can be fetched by
# its hash from the repository at <url>, the way the source notice tells a
# recipient to fetch it: one shallow fetch of the commit object alone,
# without trees and blobs, into an empty repository that is removed again.
# GitHub and GitLab serve a commit they have that way, also one no branch
# has any more, and refuse one they do not have. Exits 1 when the
# repository refuses the commit or is not a public one, 2 when it cannot be
# reached. git asks for
# no credentials, uses no credential helper, and gives up on a connection
# that moves less than 1000 bytes a second for 30 seconds.
repo_has_commit() {
    local probe err rc=0
    probe=$(mktemp -d) || return 2
    if ! git init -q "$probe" ||
       ! err=$(GIT_TERMINAL_PROMPT=0 git -c credential.helper= -c http.lowSpeedLimit=1000 \
                -c http.lowSpeedTime=30 -C "$probe" fetch --quiet --no-tags --depth=1 \
                --filter=tree:0 "$1" "$2" 2>&1); then
        # A repository that does not exist or is not public answers as one
        # that wants credentials, which git must not ask for.
        case "$err" in
            *"not our ref"*|*"unadvertised object"*|*"couldn't find remote ref"*) rc=1 ;;
            *"could not read Username"*|*"not found"*|*"Authentication failed"*) rc=1 ;;
            *) rc=2 ;;
        esac
    fi
    rm -rf "$probe"
    return "$rc"
}

# source_url <repository>: the URL published_repo judges <repository> by:
# the public_url of a URL, or of a local clone's origin. Nothing for a clone
# without an origin.
source_url() {
    local url="$1"
    if [ -d "$1" ]; then
        url=$(git -C "$1" remote get-url origin 2>/dev/null) || return 0
    fi
    public_url "$url"
}

# published_repo <repository> <commit>: print the public URL of the
# repository <commit> can be fetched from, and exit 0; exit 1 when there is
# none, 2 when the repository cannot be reached to tell (unpublished_reason
# says which). Only an https URL counts: a file://, ssh or git URL names no
# repository a recipient can fetch from. <repository> is a build script's
# *_REPO or WINE_SRC.
#  - A URL has the commit public when it serves it (repo_has_commit). The
#    dist may have been built from another repository than the *_REPO the
#    bundle step runs with, such as a local clone holding a full-hash
#    *_REF, which the bundle step's checks take as it is.
#  - A local clone, such as a fork checkout a candidate is built from, has
#    it public only when a branch of its origin contains it, as of the
#    clone's last fetch: a commit only on a local branch, or with
#    uncommitted changes (a "-dirty" commit of wine_src_commit), is not.
published_repo() {
    local repo="$1" commit="$2" url
    case "$commit" in
        *-dirty) return 1 ;;
    esac
    url=$(source_url "$repo")
    case "$url" in
        https://*) ;;
        *) return 1 ;;
    esac
    if [ -d "$repo" ]; then
        [ -n "$(git -C "$repo" for-each-ref --count=1 --contains "$commit" refs/remotes/origin 2>/dev/null)" ] || return 1
    else
        repo_has_commit "$url" "$commit" || return
    fi
    echo "$url"
}

# unpublished_reason <repository> <commit> <status>: print why
# published_repo exited with <status> for <repository> and <commit>.
unpublished_reason() {
    local url
    case "$2" in
        *-dirty) echo "the tree has uncommitted changes"; return 0 ;;
    esac
    url=$(source_url "$1")
    case "$url" in
        https://*) ;;
        "") echo "$1 has no origin"; return 0 ;;
        *) echo "$url is not an https repository"; return 0 ;;
    esac
    if [ "$3" -eq 2 ]; then
        echo "cannot reach $url"
    elif [ -d "$1" ]; then
        echo "no branch of $url has it, as $1 last fetched"
    else
        echo "$url does not have it, or is no public repository"
    fi
}

# sources_line_ok <line>: succeed when <line> is a line of the sources.txt
# bin/ci/package.sh writes and bin/tag-forks.sh reads: "<name> <repository>
# <commit>" with single spaces, the repository an https URL with a host and
# a path, the commit a full one.
sources_line_ok() {
    local re='^[^[:space:]]+ https://[^/[:space:]]+/[^[:space:]]+ [0-9a-f]{40}$'
    [[ "$1" =~ $re ]]
}

# mach9_source: print <repository>|<commit> of this repository, whose scripts
# control how every component is built and linked. In a workflow run,
# GitHub's repository and the commit the run is for (GITHUB_SHA), which a
# tag's checkout has on no remote branch. Elsewhere REPO_DIR at its HEAD,
# with "-dirty" appended when tracked files differ from it, as
# wine_src_commit does, for published_repo to judge. The commit is empty
# when REPO_DIR is not a git checkout.
mach9_source() {
    local commit
    if [ -n "${GITHUB_REPOSITORY:-}" ] && [ -n "${GITHUB_SHA:-}" ]; then
        echo "https://github.com/$GITHUB_REPOSITORY|$GITHUB_SHA"
        return 0
    fi
    commit=$(git -C "$REPO_DIR" rev-parse HEAD 2>/dev/null) || commit=""
    if [ -n "$commit" ] && [ -n "$(git -C "$REPO_DIR" status --porcelain --untracked-files=no)" ]; then
        commit="$commit-dirty"
    fi
    echo "$REPO_DIR|$commit"
}

# mach9_repo: the URL of this repository: GitHub's for a workflow run, else
# the checkout's origin. Nothing when it has none.
mach9_repo() {
    local url
    if [ -n "${GITHUB_REPOSITORY:-}" ]; then
        echo "https://github.com/$GITHUB_REPOSITORY"
    elif url=$(git -C "$REPO_DIR" remote get-url origin 2>/dev/null); then
        public_url "$url"
    fi
}
