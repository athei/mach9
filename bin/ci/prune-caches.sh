#!/bin/bash
set -o pipefail

# prune-caches.sh [--dry-run] [--repo <owner/name>] [--before <time>]
#                 <prefix> <key> [<prefix> <key>]...
#
# Deletes the GitHub Actions caches of main that a cache this run saved on
# main replaces. For each pair, <key> is the key just saved and <prefix> the
# part of it shared by every key of the same cache (its family), such as
# ccache-macos26- for the rolling ccache keys. Deleted is every cache with
# the ref refs/heads/main whose key starts with <prefix>, is not <key>, and
# was created before <time>, so each family keeps one cache.
#
# <time> defaults to the start of this run's attempt (GITHUB_RUN_ID and
# GITHUB_RUN_ATTEMPT); an explicit one is UTC, as GitHub prints it
# (2026-11-07T15:40:12Z). A cache that a run going on at the same time, such
# as brew-cache.yml next to release.yml, saved after this run started is
# kept.
#
# A pair with an empty <key> is skipped, so a caller can pass the key of a
# save that did not run as empty. Nothing is deleted for a <prefix> when
# <key> is not among main's caches, as after a save that failed to upload.
#
# --dry-run lists what would be deleted and deletes nothing. Needs gh and a
# token in GH_TOKEN that may write actions. An error is a warning and the
# script goes on, so pruning never fails a run; only a usage error exits
# non-zero.

usage() {
    echo "Usage: $0 [--dry-run] [--repo <owner/name>] [--before <time>] <prefix> <key> [<prefix> <key>]..." >&2
    exit 2
}

warn() {
    if [ -n "$GITHUB_ACTIONS" ]; then
        echo "::warning::prune-caches.sh: $*"
    else
        echo "Warning: $*" >&2
    fi
}

DRY_RUN=0
REPO=${GITHUB_REPOSITORY:-}
BEFORE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        --repo|--before)
            [ $# -ge 2 ] || usage
            if [ "$1" = --repo ]; then REPO=$2; else BEFORE=$2; fi
            shift 2
            ;;
        --) shift; break ;;
        -*) usage ;;
        *) break ;;
    esac
done
if [ $# -eq 0 ] || [ $(( $# % 2 )) -ne 0 ] || [ -z "$REPO" ]; then
    usage
fi
# Every second argument is a key.
saved=0
i=0
for arg in "$@"; do
    i=$((i + 1))
    if [ $((i % 2)) -eq 0 ] && [ -n "$arg" ]; then
        saved=1
    fi
done
if [ "$saved" -eq 0 ]; then
    echo "No cache saved; nothing to prune"
    exit 0
fi

if [ -z "$BEFORE" ]; then
    if [ -z "$GITHUB_RUN_ID" ]; then
        echo "Error: --before is needed outside a GitHub Actions run" >&2
        exit 2
    fi
    if ! BEFORE=$(gh api "repos/$REPO/actions/runs/$GITHUB_RUN_ID/attempts/${GITHUB_RUN_ATTEMPT:-1}" --jq .run_started_at); then
        warn "cannot read when this run started; nothing pruned"
        exit 0
    fi
fi
# Whole seconds as one number, YYYYMMDDHHMMSS, compared with each cache's
# creation time cut to the second, so a cache created in the second the run
# started is kept.
time_re='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?Z$'
if ! [[ "$BEFORE" =~ $time_re ]]; then
    warn "not a UTC time: '$BEFORE'; nothing pruned"
    exit 0
fi
before=${BEFORE:0:19}
before=${before//[!0-9]/}
echo "Pruning caches of refs/heads/main created before $BEFORE"

# prune <prefix> <key>
prune() {
    local prefix=$1 key=$2 list id k created n found=0 doomed=""
    if [ -z "$key" ]; then
        return
    fi
    if [ -z "$prefix" ] || [ "$prefix" = "$key" ]; then
        warn "no family prefix for $key; not pruning"
        return
    fi
    case "$key" in
        "$prefix"*) ;;
        *) warn "$key does not start with $prefix; not pruning"; return ;;
    esac
    if ! list=$(gh cache list --repo "$REPO" --ref refs/heads/main --key "$prefix" \
            --limit 1000 --json id,key,ref,createdAt \
            --jq '.[] | select(.ref == "refs/heads/main") | [.id, .key, .createdAt] | @tsv'); then
        warn "cannot list the caches $prefix*; not pruning"
        return
    fi
    echo "$prefix*: saved $key"
    while IFS=$'\t' read -r id k created; do
        [ -n "$id" ] || continue
        if [ "$k" = "$key" ]; then
            found=1
            continue
        fi
        n=${created:0:19}
        n=${n//[!0-9]/}
        if ! [[ "$n" =~ ^[0-9]{14}$ ]] || [ "$n" -ge "$before" ]; then
            echo "  keep   $k (created $created)"
            continue
        fi
        doomed="$doomed$id $k $created"$'\n'
    done <<< "$list"
    if [ "$found" -eq 0 ]; then
        warn "$key is not among main's caches; not pruning $prefix*"
        return
    fi
    while read -r id k created; do
        [ -n "$id" ] || continue
        # gh matches --key as a prefix; check again before deleting.
        case "$k" in
            "$prefix"*) ;;
            *) continue ;;
        esac
        if [ "$DRY_RUN" -eq 1 ]; then
            echo "  would delete $k (created $created)"
        elif gh cache delete "$id" --repo "$REPO" > /dev/null; then
            echo "  deleted $k (created $created)"
        else
            warn "cannot delete $k"
        fi
    done <<< "$doomed"
}

while [ $# -gt 0 ]; do
    prune "$1" "$2"
    shift 2
done
exit 0
