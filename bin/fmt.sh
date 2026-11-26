#!/bin/bash
set -eo pipefail

# fmt.sh [<cargo fmt argument>...]: format the Rust code of this repository
# with the rustfmt of RUST_NIGHTLY in pins/rust.env, the one the workflows
# check the formatting with, so a tree formatted here passes their check.
#
#   bin/fmt.sh                      format the workspace
#   bin/fmt.sh --check              check the workspace
#   bin/fmt.sh --check -p conformance
#                                   check one crate
#
# The arguments are cargo fmt's and are passed through as given, after the
# two this script adds when they are missing: --manifest-path with this
# repository's Cargo.toml, so the script acts on this repository from any
# directory, and --all when no -p, --package or --all selects what to
# format. A relative --manifest-path is resolved from the current directory,
# as cargo does. Arguments after -- go to rustfmt and are not looked at.
#
# It runs cargo +RUST_NIGHTLY, which outranks RUSTUP_TOOLCHAIN, so it works
# the same in a CI job that pins its stable toolchain that way and for a
# developer. It never falls back to another toolchain and installs nothing:
# without rustup or the pinned nightly with rustfmt for this host, it stops
# and names the rustup command to run, and it stops on a cargo that is not
# rustup's. profile.sh is not sourced: it sets up the macOS build, and this
# also runs on Linux.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# Only the file's value counts, not one exported in the environment.
unset RUST_NIGHTLY
# shellcheck source=../pins/rust.env
. "$REPO_DIR/pins/rust.env"

if [ -z "$RUST_NIGHTLY" ]; then
    echo "Error: $REPO_DIR/pins/rust.env sets no RUST_NIGHTLY" >&2
    exit 1
fi
install="rustup toolchain install $RUST_NIGHTLY --profile minimal --component rustfmt"

if ! command -v rustup >/dev/null; then
    echo "Error: rustup not found, and the pinned rustfmt needs it; install rustup, then run: $install" >&2
    exit 1
fi
# rustup names an installed toolchain with its host triple, and
# cargo +RUST_NIGHTLY means the one for rustup's default host, which
# rustup show reports. rustup before 1.28 installs the active toolchain
# there when it is missing, so it runs in this repository, which has no
# rust-toolchain.toml, and without RUSTUP_TOOLCHAIN: a toolchain file of
# the caller's directory or the variable cannot make it install one. The
# subshell leaves the directory cargo fmt runs in below unchanged.
host=$(cd "$REPO_DIR" && env -u RUSTUP_TOOLCHAIN rustup show 2>/dev/null | sed -n 's/^Default host: //p' || true)
if [ -z "$host" ]; then
    echo "Error: rustup show names no default host" >&2
    exit 1
fi
toolchains=$(rustup toolchain list)
if ! grep -qE "^$RUST_NIGHTLY-$host( |$)" <<< "$toolchains"; then
    echo "Error: the toolchain $RUST_NIGHTLY of pins/rust.env is not installed for $host; run: $install" >&2
    exit 1
fi
components=$(rustup component list --installed --toolchain "$RUST_NIGHTLY")
if ! grep -q '^rustfmt' <<< "$components"; then
    echo "Error: $RUST_NIGHTLY of pins/rust.env has no rustfmt; run: $install" >&2
    exit 1
fi
if ! cargo +"$RUST_NIGHTLY" --version >/dev/null 2>&1; then
    cargo=$(command -v cargo || true)
    echo "Error: the cargo on PATH (${cargo:-none}) cannot run +$RUST_NIGHTLY, so it is not rustup's; put rustup's cargo first on PATH" >&2
    exit 1
fi

manifest=0
select=0
for arg in "$@"; do
    case "$arg" in
        --) break ;;
        --manifest-path|--manifest-path=*) manifest=1 ;;
        -p|-p*|--package|--package=*|--all) select=1 ;;
    esac
done
added=()
if [ "$manifest" = 0 ]; then
    added+=(--manifest-path "$REPO_DIR/Cargo.toml")
fi
if [ "$select" = 0 ]; then
    added+=(--all)
fi

exec cargo +"$RUST_NIGHTLY" fmt "${added[@]}" "$@"
