#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# ROOT, the layout and the profiles.
# shellcheck source=profile.sh
. "$SCRIPT_DIR/profile.sh"
CACHE_DIR="${CACHE_DIR:-$ROOT/cache}"
MINGW_DIR="${MINGW_DIR:-/opt/llvm-mingw}"

# URL and SHA-256 of every third-party artifact (GPTK_URL/GPTK_SHA256 etc.).
# shellcheck source=redist.env
. "$SCRIPT_DIR/redist.env"
DIST_DIR=""
# Set by unpack_dxmt or from DXMT_DIST; never taken from the environment.
DXMT_SRC=""
# The files installed from the GPTK image (Step 4); never from the environment.
GPTK_FILES=""
RUNTIME_ONLY=0
COMPATDB_ONLY=0
STRICT_FLOOR=0
WITH_SYMBOLS=0
ASSUME_YES=0
ARCH_PROFILE=intel
# Whether wine.app (arm64 profile) gets a secure timestamp, which needs
# Apple's timestamp server. LOADER_TIMESTAMP=0 signs without one, for an
# offline local build; such a bundle cannot be notarized.
LOADER_TIMESTAMP="${LOADER_TIMESTAMP:-1}"
case "$LOADER_TIMESTAMP" in
    0|1) ;;
    *) echo "Error: LOADER_TIMESTAMP must be 0 or 1, not '$LOADER_TIMESTAMP'"; exit 1 ;;
esac

# `make install` may rebuild any stale targets, including PE modules that need mingw.
if [ -d "$MINGW_DIR/bin" ]; then
    export PATH="$PATH:$MINGW_DIR/bin"
fi

# The floor for Apple's D3DMetal.framework, which is copied in unmodified and
# needs a newer macOS than the rest. Must equal GPTK_MIN in
# compatdb/src/macos.rs, which is where compatdb.so decides that gptk can load.
GPTK_FLOOR=26.4

usage() {
    echo "Usage: $0 [--dest <dir>] [--runtime-only] [--compatdb-only] [--strict-floor] [--symbols] [--yes] [--arm64]"
    exit 1
}

# --dest names the directory the bundle goes into as <dir>/wine; without it,
# the profile's place in the layout (BUNDLE_DEST in profile.sh).
# --compatdb-only replaces compatdb.so in the bundle already at <dir>/wine and
# leaves everything else alone. --runtime-only is accepted alongside it and
# ignored, since it only affects what a full bundle installs. --strict-floor
# turns a Mach-O that needs a newer macOS than MACOS_FLOOR into an error
# instead of a warning. CI passes it: its libraries come from the deps
# tarball package-deps.sh builds, which meets the floor. A local build copies
# whatever Homebrew has, which on a newer macOS can carry a newer minos, and
# is not distributed. --symbols keeps the PE files as they were before Step
# 4e stripped them in <dir>/wine-symbols (the release workflow publishes
# them). --yes deletes an existing bundle at <dir>/wine without asking first,
# for a run nobody watches. --arm64 bundles the native arm64 build
# (profile.sh).
while [ $# -gt 0 ]; do
    case "$1" in
        --dest) [ $# -ge 2 ] || usage; DIST_DIR="$2"; shift 2 ;;
        --runtime-only) RUNTIME_ONLY=1; shift ;;
        --compatdb-only) COMPATDB_ONLY=1; shift ;;
        --strict-floor) STRICT_FLOOR=1; shift ;;
        --symbols) WITH_SYMBOLS=1; shift ;;
        --yes) ASSUME_YES=1; shift ;;
        --arm64) ARCH_PROFILE=arm64; shift ;;
        *) usage ;;
    esac
done

# Paths, the floor every Mach-O is checked against (MACOS_FLOOR, the
# deployment target build-wine.sh builds with) and the feature flags.
set_profile "$ARCH_PROFILE"
if [ -z "$DIST_DIR" ]; then
    DIST_DIR="$BUNDLE_DEST"
fi

WINE_DIR="$DIST_DIR/wine"
# The PE files' symbols (Step 4e, with --symbols), next to the bundle and
# never in it.
SYMBOLS_DIR="$DIST_DIR/wine-symbols"
unixdir="$WINE_DIR/lib/wine/$UNIX_DIR"
COMPATDB_BUILD="$BUILD_DIR/compatdb/$RUST_TARGET/release/libcompatdb.dylib"

TMP_DIR=$(mktemp -d)
GPTK_MNT="$TMP_DIR/gptk"
cleanup() {
    if [ -d "$GPTK_MNT" ]; then
        hdiutil detach "$GPTK_MNT" -quiet 2>/dev/null || true
    fi
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# in_list <word> <list>: succeed when <word> is one of the space-separated
# words in <list>.
in_list() {
    case " $2 " in
        *" $1 "*) return 0 ;;
    esac
    return 1
}

# Timing. time_mark <label> ends the interval of the previous mark and starts
# one for <label>; print_timings lists them all, in order, at the end. Wall
# time in milliseconds, through perl since bash 3.2 has no subsecond clock.
TIMINGS=()
TIMING_LABEL=""
TIMING_START=0
now_ms() {
    perl -MTime::HiRes=time -e 'printf "%d\n", time * 1000'
}
time_mark() {
    local now
    now=$(now_ms)
    if [ -n "$TIMING_LABEL" ]; then
        TIMINGS+=("$((now - TIMING_START))|$TIMING_LABEL")
    fi
    TIMING_LABEL="$1"
    TIMING_START=$now
}
print_timings() {
    local entry total=0
    time_mark ""
    echo "==> Time per step"
    for entry in "${TIMINGS[@]}"; do
        printf '  %8.1fs  %s\n' "$(echo "${entry%%|*}" | awk '{print $1 / 1000}')" "${entry#*|}"
        total=$((total + ${entry%%|*}))
    done
    printf '  %8.1fs  total\n' "$(echo "$total" | awk '{print $1 / 1000}')"
}

# fetch <url> <sha256> [name]: download an artifact into $CACHE_DIR, or reuse
# the cached copy when its checksum still matches the pin. Prints the local
# path. The cached file is named after the URL's last component unless <name>
# is given, which is for URLs that end in a generic name such as LICENSE.
# A cached copy that no longer matches is stale (a bump that kept the file
# name, as mtld3d.tar.xz does) and is replaced. A fresh download that does
# not match is an error, not a retry: either the pin in redist.env is wrong
# or the download is, and both need a human. Downloads land in a .part file
# first, so an interrupted run never leaves a plausible-looking file behind.
fetch() {
    local url="$1" want="$2" name="${3:-}"
    local target got
    [ -n "$name" ] || name=$(basename "$url")
    target="$CACHE_DIR/$name"
    if [ -f "$target" ]; then
        got=$(shasum -a 256 "$target" | cut -d' ' -f1)
        if [ "$got" = "$want" ]; then
            echo "    $name (cached)" >&2
            echo "$target"
            return 0
        fi
        echo "    $name: cached copy does not match redist.env, re-downloading" >&2
        rm -f "$target"
    fi
    echo "    $name (downloading)" >&2
    mkdir -p "$CACHE_DIR"
    curl -fsSL --retry 3 -o "$target.part" "$url"
    got=$(shasum -a 256 "$target.part" | cut -d' ' -f1)
    if [ "$got" != "$want" ]; then
        rm -f "$target.part"
        echo "Error: $name checksum mismatch" >&2
        echo "  expected $want" >&2
        echo "  got      $got" >&2
        echo "  from     $url" >&2
        return 1
    fi
    mv "$target.part" "$target"
    echo "$target"
}

# pe_scan: read NUL-separated paths on stdin and print, for each that is a PE
# image, "<path>\t<symbol table offset>\t<symbol count>" from its COFF
# header, sorted by path. A PE image is an MZ file whose e_lfanew points at a
# PE signature, not only an MZ file: Wine's .fon fonts are MZ files with a
# 16-bit NE header. One perl process reads the headers of every file, rather
# than a few processes per file.
pe_scan() {
    perl -0 -ne '
        chomp;
        my $f = $_;
        open(my $h, "<", $f) or next;
        binmode $h;
        my ($mz, $coff);
        next unless read($h, $mz, 64) == 64 && substr($mz, 0, 2) eq "MZ";
        seek($h, unpack("V", substr($mz, 60, 4)), 0) or next;
        next unless read($h, $coff, 24) == 24 && substr($coff, 0, 4) eq "PE\0\0";
        my ($ptr, $count) = unpack("VV", substr($coff, 12, 8));
        print "$f\t$ptr\t$count\n";
    ' | LC_ALL=C sort
}

# strip_pe <rel>...: strip each PE file <rel> (a path in the bundle) in
# place, as Step 4e describes, keeping the unstripped copy in SYMBOLS_DIR
# with --symbols. Runs in parallel batches under xargs, so it reports
# instead of exiting: an error goes to $STRIP_ERRORS/<n> and leaves the file
# as it was. Prints the path of every file it changed.
strip_pe() {
    local rel f out
    for rel in "$@"; do
        f="$WINE_DIR/$rel"
        out="$STRIP_WORK/$(printf '%s' "$rel" | tr '/' '%')"
        if ! "$MINGW_DIR/bin/llvm-strip" --strip-all -o "$out" "$f" 2>"$out.err" \
            && ! drop_coff_symtab "$f" "$out"; then
            { echo "Error: cannot strip $rel:"; cat "$out.err"; } > "$STRIP_ERRORS/${out##*/}"
            rm -f "$out" "$out.err"
            continue
        fi
        if cmp -s "$f" "$out"; then
            rm -f "$out" "$out.err"
            continue
        fi
        if head -c 128 "$f" | grep -qa "Wine builtin DLL" \
            && ! head -c 128 "$out" | grep -qa "Wine builtin DLL"; then
            echo "Error: stripping $rel lost its builtin marker" > "$STRIP_ERRORS/${out##*/}"
            rm -f "$out" "$out.err"
            continue
        fi
        if [ "$WITH_SYMBOLS" -eq 1 ]; then
            if ! mkdir -p "$SYMBOLS_DIR/$(dirname "$rel")" || ! cp -p "$f" "$SYMBOLS_DIR/$rel"; then
                echo "Error: cannot keep the unstripped $rel in $SYMBOLS_DIR" > "$STRIP_ERRORS/${out##*/}"
                rm -f "$out" "$out.err"
                continue
            fi
        fi
        if ! cp "$out" "$f"; then
            echo "Error: cannot replace $rel with its stripped copy" > "$STRIP_ERRORS/${out##*/}"
            rm -f "$out" "$out.err"
            continue
        fi
        rm -f "$out" "$out.err"
        echo "$rel"
    done
}

# pe_debug_files <file>...: print each of the PE files that has .debug_*
# sections, and "!<file>" for one llvm-objdump cannot read. One llvm-objdump
# reads the whole batch; only when it fails is each file read on its own, to
# tell which one it was.
pe_debug_files() {
    local out f
    if out=$("$MINGW_DIR/bin/llvm-objdump" -h "$@" 2>&1); then
        awk '
            /:\tfile format / { cur = $0; sub(/:\tfile format .*/, "", cur); seen = 0; next }
            / \.debug_/ && !seen { print cur; seen = 1 }
        ' <<< "$out"
        return 0
    fi
    for f in "$@"; do
        if out=$("$MINGW_DIR/bin/llvm-objdump" -h "$f" 2>&1); then
            if grep -q ' \.debug_' <<< "$out"; then echo "$f"; fi
        else
            echo "!$f"
        fi
    done
}

# coff_symtab <pe-file>: print where the file's COFF symbol table starts,
# its symbol count and where the string table after it ends, as file
# offsets; "0 0 0" when the header points at no table.
coff_symtab() {
    local pe ptr count strings
    pe=$(od -An -tu4 -j60 -N4 "$1" | tr -d ' ')
    ptr=$(od -An -tu4 -j$((pe + 12)) -N4 "$1" | tr -d ' ')
    count=$(od -An -tu4 -j$((pe + 16)) -N4 "$1" | tr -d ' ')
    if [ "$ptr" -eq 0 ] && [ "$count" -eq 0 ]; then
        echo "0 0 0"
        return
    fi
    strings=$(od -An -tu4 -j$((ptr + count * 18)) -N4 "$1" | tr -d ' ')
    echo "$ptr $count $((ptr + count * 18 + ${strings:-0}))"
}

# drop_coff_symtab <pe-file> <out>: write <pe-file> to <out> without its COFF
# symbol table, by cutting the file where the table starts and clearing the
# header's pointer and count. Fails unless the table and its strings end the
# file and the file has no .debug_* sections, which would need llvm-strip.
drop_coff_symtab() {
    local pe ptr count end sections
    read -r ptr count end < <(coff_symtab "$1")
    [ "$ptr" -ne 0 ] && [ "$end" -eq "$(stat -f %z "$1")" ] || return 1
    sections=$("$MINGW_DIR/bin/llvm-objdump" -h "$1" 2>&1) || return 1
    if grep -q ' \.debug_' <<< "$sections"; then
        return 1
    fi
    pe=$(od -An -tu4 -j60 -N4 "$1" | tr -d ' ')
    head -c "$ptr" "$1" > "$2" || return 1
    printf '\0\0\0\0\0\0\0\0' | dd of="$2" bs=1 seek=$((pe + 12)) conv=notrunc 2>/dev/null
}

# The helpers below exit on failure instead of returning a status. Call them
# as plain statements, never inside `if` or before `||`: either would turn
# off set -e for everything they run.

# build_compatdb: build the compatdb/ crate and install it as
# $unixdir/compatdb.so. The profile exports MACOSX_DEPLOYMENT_TARGET, which
# cargo hands to rustc and which takes precedence over the same pin in
# .cargo/config.toml; cargo only reads that file relative to the working
# directory, hence the subshell in $SCRIPT_DIR all the same. The new file is
# copied next to the old one and renamed over it, so a Wine process that has
# the old library mapped keeps a valid mapping instead of a file that changes
# underneath it.
build_compatdb() {
    if ! command -v cargo >/dev/null; then
        echo "Error: cargo not found (compatdb.so needs a Rust toolchain with the"
        echo "$RUST_TARGET target: rustup target add $RUST_TARGET)"
        exit 1
    fi
    if command -v rustup >/dev/null \
        && ! rustup target list --installed | grep -qx "$RUST_TARGET"; then
        echo "Error: the Rust target $RUST_TARGET is not installed"
        echo "(rustup target add $RUST_TARGET)"
        exit 1
    fi
    (cd "$SCRIPT_DIR" && CARGO_TARGET_DIR="$BUILD_DIR/compatdb" \
        cargo build --quiet --release --manifest-path "$SCRIPT_DIR/Cargo.toml" \
            -p compatdb --target "$RUST_TARGET")
    echo "    $UNIX_DIR/compatdb.so"
    cp "$COMPATDB_BUILD" "$unixdir/compatdb.so.new"
    mv -f "$unixdir/compatdb.so.new" "$unixdir/compatdb.so"
}

# check_no_leaks <file>...: report every file whose load commands still
# reference one of LEAK_PREFIXES and set LEAKED=1. Paths that do not exist
# (a glob that matched nothing) are skipped. The caller resets LEAKED and
# acts on it.
check_no_leaks() {
    local f p
    for f in "$@"; do
        [ -e "$f" ] || continue
        for p in $LEAK_PREFIXES; do
            if otool -L "$f" 2>/dev/null | grep -q "$p"; then
                echo "  ERROR: $(basename "$f") still references $p"
                LEAKED=1
            fi
        done
    done
}

# check_minos <path>...: report every Mach-O under the paths whose minos is
# newer than its floor, then exit with --strict-floor or warn without it.
# Like package-deps.sh, this looks at executables, .dylib and .so files, and
# skips files with no LC_BUILD_VERSION (not Mach-O, or too old to have one).
# For a universal file the newest slice counts. The floor is MACOS_FLOOR,
# except for lib/external/D3DMetal.framework: its libdxccontainer.dylib needs
# macOS 26.4, so those files are held to GPTK_FLOOR, and below that compatdb.so
# gives 64-bit processes DXMT instead. libd3dshared.dylib is not part of that:
# ntdll loads it in every process that runs a game executable, so it has to
# meet MACOS_FLOOR. If no file had a minos at all, nothing was checked, which
# is an error with or without --strict-floor.
check_minos() {
    local f rel minos floor bad=0 checked=0
    if ! command -v otool >/dev/null; then
        echo "Error: otool not found (it comes with the Xcode command line tools)"
        exit 1
    fi
    while IFS= read -r f; do
        rel=${f#"$WINE_DIR"/}
        case "$rel" in
            lib/external/D3DMetal.framework/*) floor=$GPTK_FLOOR ;;
            *) floor=$MACOS_FLOOR ;;
        esac
        minos=$(otool -arch all -l "$f" 2>/dev/null | awk '
            $1 == "cmd" && $2 == "LC_BUILD_VERSION" { v = 1 }
            v && $1 == "minos" {
                split($2, p, ".")
                k = p[1] * 10000 + p[2] * 100 + p[3]
                if (k > max) { max = k; s = $2 }
                v = 0
            }
            END { if (s != "") print s }')
        [ -n "$minos" ] || continue
        checked=$((checked + 1))
        if macos_newer "$minos" "$floor"; then
            echo "  too new: minos $minos, floor $floor  $rel"
            bad=1
        fi
    done < <(find "$@" -type f \( -perm +111 -o -name '*.dylib' -o -name '*.so' \))
    if [ "$checked" -eq 0 ]; then
        echo "Error: no Mach-O with an LC_BUILD_VERSION found under $*"
        echo "(otool -l printed no minos for any file, so nothing was checked)"
        exit 1
    fi
    if [ "$bad" -eq 0 ]; then
        echo "  $checked Mach-O files meet their floor (macOS $MACOS_FLOOR, D3DMetal $GPTK_FLOOR)."
        return 0
    fi
    if [ "$STRICT_FLOOR" -eq 1 ]; then
        echo "Error: the above need a newer macOS than their floor"
        exit 1
    fi
    echo "  WARNING: the above need a newer macOS than their floor and will not"
    echo "  load on older systems (--strict-floor makes this an error)"
}

# check_query_export: fail unless the installed compatdb.so exports
# compatdb_query_env and, with the x87sidecar, compatdb_query_x87. ntdll
# reads a missing compatdb_query_x87 as no opinion, which means attaching the
# x87sidecar to every i386 process, and without compatdb_query_env a rule's
# env reaches the process too late for its x87sidecar and Rosetta. Either way
# a stripped or renamed export would otherwise go unnoticed. A build without
# the x87sidecar has no Rosetta to start one under, so the x87 query is not
# asked for there (check_no_x87 makes sure it is gone).
check_query_export() {
    local sym syms="compatdb_query_x87 compatdb_query_env"
    if [ "$WITH_X87SIDECAR" -eq 0 ]; then
        syms="compatdb_query_env"
    fi
    for sym in $syms; do
        if ! nm -gU "$unixdir/compatdb.so" 2>/dev/null | grep -q "_${sym}\$"; then
            echo "Error: $UNIX_DIR/compatdb.so does not export $sym"
            exit 1
        fi
        echo "  compatdb.so exports $sym."
    done
}

# check_no_x87: for a build without the x87sidecar, fail if anything of it
# is left: the binary, or an exported x87 symbol in compatdb.so or ntdll.so
# (compatdb_query_x87, or the ntdll side that would look for a sidecar).
check_no_x87() {
    local f
    if [ -e "$WINE_DIR/bin/x87sidecar" ]; then
        echo "Error: bin/x87sidecar is present in a build without the x87sidecar"
        exit 1
    fi
    for f in compatdb.so ntdll.so; do
        if nm -gU "$unixdir/$f" 2>/dev/null | grep -qi x87; then
            echo "Error: $UNIX_DIR/$f exports x87 symbols:"
            nm -gU "$unixdir/$f" | grep -i x87
            exit 1
        fi
    done
    echo "  No x87sidecar, and no x87 exports in compatdb.so or ntdll.so."
}

# check_x87sidecar_binary: fail unless bin/x87sidecar is an executable arm64
# Mach-O. ntdll checks the sidecar with access(X_OK) and skips it otherwise,
# and only an arm64 binary can attach to Rosetta, so an x86_64 file or a
# non-executable one must not pass for a working sidecar.
check_x87sidecar_binary() {
    if [ ! -x "$WINE_DIR/bin/x87sidecar" ]; then
        echo "Error: bin/x87sidecar is missing or not executable"
        exit 1
    fi
    if ! lipo -archs "$WINE_DIR/bin/x87sidecar" 2>/dev/null | grep -qw arm64; then
        echo "Error: bin/x87sidecar is not an arm64 Mach-O"
        exit 1
    fi
    echo "  bin/x87sidecar is an arm64 executable."
}

# run_limited <seconds> <log> <command>...: run the command with its output in
# <log>, and stop it with SIGALRM after <seconds>; the alarm survives exec,
# so the command may start with env. Sets RUN_STATUS to its exit status; 142
# (128 + SIGALRM) means the alarm fired. perl, because macOS has no
# timeout(1).
run_limited() {
    local secs="$1" log="$2"
    shift 2
    RUN_STATUS=0
    perl -e 'alarm shift; exec @ARGV or die "exec: $!\n"' "$secs" "$@" >"$log" 2>&1 \
        || RUN_STATUS=$?
}

# check_default_trees <log> [<dxgi> <d3d9>]: exit unless compatdb.so reported
# those trees in <log>. The "(from ...)" lines only appear once it found and
# prepended them. Without the two names, the default tree of each family for
# an x86 program: the first in the profile's list. D3DMetal needs macOS
# $GPTK_FLOOR or later, and below that compatdb.so gives the 64-bit cmd.exe
# DXMT instead, so on an older host that is the line to expect.
check_default_trees() {
    local want dxgi_want d3d9_want
    dxgi_want="dxgi = ${2:-${DXGI_IMPLS%% *}} (from"
    d3d9_want="d3d9 = ${3:-${D3D9_IMPLS%% *}} (from"
    if [ -z "${2:-}" ] && [ "${DXGI_IMPLS%% *}" = gptk ] \
        && macos_newer "$GPTK_FLOOR" "$(sw_vers -productVersion)"; then
        dxgi_want="dxgi = dxmt (from"
    fi
    for want in "$dxgi_want" "$d3d9_want"; do
        if ! grep -q "compatdb: .*$want" "$1"; then
            echo "Error: compatdb.so did not report '$want'"
            grep "compatdb:" "$1" || echo "  (no compatdb lines at all)"
            exit 1
        fi
    done
    echo "  compatdb.so loads and finds its trees."
}

# smoke_test: run the bundled wine and check that compatdb.so loads in it, and
# that an i386 process starts under the bundled x87sidecar.
smoke_test() {
    local version smoke_log x87_log
    version=$("$WINE_DIR/bin/wine" --version) || {
        echo "Error: bundled wine failed to run"
        exit 1
    }
    echo "  Testing: $version"

    # `wine --version` never loads ntdll, so it proves nothing about compatdb.so.
    # Booting a throwaway prefix does: the library logs one block per process,
    # and its "(from ...)" lines only appear once it found and prepended the
    # trees. The Mono and Gecko installers are kept out so nothing pops a dialog.
    time_mark "smoke: prefix boot"
    echo "  Booting a throwaway prefix to check compatdb.so..."
    smoke_log="$TMP_DIR/smoke.log"
    WINEPREFIX="$TMP_DIR/prefix" WINEDLLOVERRIDES="mscoree,mshtml=" WINEDEBUG=-all \
        "$WINE_DIR/bin/wine" cmd /c exit >/dev/null 2>"$smoke_log" || {
        echo "Error: bundled wine failed to boot a prefix"
        cat "$smoke_log"
        exit 1
    }

    # An ntdll from before the per-executable x87sidecar decision lacks this
    # export and only starts the sidecar named by ROSETTA_X87_PATH, so there is
    # nothing to check. --compatdb-only meets that on an older release and
    # skips the check. A full bundle without the export was built from a Wine
    # tree that lacks the ntdll commits, which is an error.
    x87_log=""
    if nm -gU "$unixdir/ntdll.so" 2>/dev/null | grep -q '_compatdb_preinit_query$'; then
        check_x87sidecar_binary
        # ntdll maps the DOS path through the prefix's dosdevices and resolves
        # it to a unix path before it starts the first process, so compatdb.so
        # can read the file's version resource; when the path cannot be
        # resolved, only the basename is used. Either way the built-in
        # x87-sidecar rule says yes. ntdll logs the launch before it execs the
        # sidecar, and that line is what this looks for. Together with a clean
        # exit, that proves ntdll started bin/x87sidecar and the program ran
        # to the end under it; it does not prove that the sidecar's x87 hook
        # took effect. Run in the prefix just booted, while its server is
        # still up, so it takes about a second; the alarm stops a hung run
        # after two minutes. The caller's ROSETTA_X87_PATH and WINE_COMPATDB
        # are dropped so that the bundled sidecar and the built-in rules
        # decide.
        time_mark "smoke: i386 process under the x87sidecar"
        echo "  Starting an i386 process to check the x87sidecar..."
        x87_log="$TMP_DIR/x87.log"
        local x87_status=0
        env -u ROSETTA_X87_PATH -u WINE_COMPATDB \
            WINEPREFIX="$TMP_DIR/prefix" WINEDLLOVERRIDES="mscoree,mshtml=" WINEDEBUG=-all,err+module \
            perl -e 'alarm shift; exec @ARGV or die "exec: $!\n"' 120 \
            "$WINE_DIR/bin/wine" 'C:\windows\syswow64\cmd.exe' /c exit >"$x87_log" 2>&1 \
            || x87_status=$?
        if [ "$x87_status" -ne 0 ]; then
            # 142 is 128 + SIGALRM: the alarm fired.
            if [ "$x87_status" -eq 142 ]; then
                echo "Error: the i386 cmd.exe did not exit within 120 seconds"
            else
                echo "Error: the i386 cmd.exe failed under the bundled wine (status $x87_status)"
            fi
            cat "$x87_log"
            WINEPREFIX="$TMP_DIR/prefix" "$WINE_DIR/bin/wineserver" -k 2>/dev/null || true
            exit 1
        fi
    elif [ "$COMPATDB_ONLY" -eq 1 ]; then
        echo "  Skipping the x87sidecar check: this ntdll predates the per-executable decision."
    else
        echo "Error: ntdll.so lacks compatdb_preinit_query; the Wine tree is missing"
        echo "the ntdll commits for the x87sidecar"
        WINEPREFIX="$TMP_DIR/prefix" "$WINE_DIR/bin/wineserver" -k 2>/dev/null || true
        exit 1
    fi

    # Every Direct3D tree, in x86_64 and i386 programs. Not for
    # --compatdb-only, which leaves the PE side as it was and has no
    # llvm-mingw to build the program with.
    if [ "$COMPATDB_ONLY" -eq 0 ]; then
        echo "  Loading each Direct3D tree's modules..."
        check_tree_loads x86_64 env -u WINE_COMPATDB -u ROSETTA_X87_PATH \
            WINEPREFIX="$TMP_DIR/prefix" WINEDLLOVERRIDES="mscoree,mshtml=" WINEDEBUG=-all
        check_tree_loads i686 env -u WINE_COMPATDB -u ROSETTA_X87_PATH \
            WINEPREFIX="$TMP_DIR/prefix" WINEDLLOVERRIDES="mscoree,mshtml=" WINEDEBUG=-all
    fi

    # The server is persistent, so it has to be told to go rather than waited for.
    WINEPREFIX="$TMP_DIR/prefix" "$WINE_DIR/bin/wineserver" -k 2>/dev/null || true
    check_default_trees "$smoke_log"

    if [ -n "$x87_log" ]; then
        # "x87sidecar:" rather than "ROSETTA_X87_PATH:" is how ntdll says the
        # sidecar is the one in bin/.
        if ! grep -q "x87sidecar: attaching rosettax87 --cooperative" "$x87_log"; then
            echo "Error: ntdll did not start the i386 cmd.exe under bin/x87sidecar"
            cat "$x87_log"
            exit 1
        fi
        echo "  i386 processes start under bin/x87sidecar."
    fi
}

# smoke_test_arm64: run the bundled arm64 wine through the signed loader and
# check that native, x86_64 and i386 programs run, the last two under FEX.
# Every run has a time limit, since an emulator problem tends to show as a
# hang rather than an error. The caller's WINE_COMPATDB and FEX_* variables
# are dropped so that the built-in rules and FEX's defaults decide.
smoke_test_arm64() {
    local version log arch cc exe marker markers sidecar var
    local wine_env=(env -u WINE_COMPATDB)
    for var in $(compgen -e | grep '^FEX_'); do
        wine_env+=(-u "$var")
    done
    wine_env+=(WINEPREFIX="$TMP_DIR/prefix" WINEDLLOVERRIDES="mscoree,mshtml=" WINEDEBUG=-all)
    version=$("$WINE_DIR/bin/wine" --version) || {
        echo "Error: bundled wine failed to run"
        exit 1
    }
    echo "  Testing: $version"

    # wineboot creates the prefix here, so the limit is generous. The Mono
    # and Gecko installers are kept out so nothing pops a dialog.
    time_mark "smoke: prefix boot"
    echo "  Booting a throwaway prefix..."
    log="$TMP_DIR/smoke.log"
    run_limited 600 "$log" "${wine_env[@]}" "$WINE_DIR/bin/wine" cmd /c exit
    smoke_check "the prefix boot (cmd /c exit)" 600 "$log"
    # compatdb.so logs a block in every process; `wine --version` never loads
    # ntdll, so these lines are the first proof that the library loads.
    if ! grep -q "compatdb:" "$log"; then
        echo "Error: compatdb.so wrote no log lines while the prefix booted"
        cat "$log"
        smoke_stop
        exit 1
    fi
    # Nothing may try to start or ask about the x87sidecar. The pattern names
    # the sidecar's traces only, not x87 as such: the arm64 compatdb.so logs
    # its fex-x87-reduced-precision rule and that rule's FEX variable in every
    # process.
    sidecar='x87sidecar|x87_sidecar|x87 query|rosettax87|rosetta_x87'
    if grep -qiE "$sidecar" "$log"; then
        echo "Error: the prefix boot shows x87sidecar traces in a build without it:"
        grep -iE "$sidecar" "$log"
        smoke_stop
        exit 1
    fi
    echo "  Nothing involves the x87sidecar."

    time_mark "smoke: i386 cmd.exe under FEX"
    echo "  Starting the i386 cmd.exe under FEX..."
    log="$TMP_DIR/wow64.log"
    run_limited 120 "$log" "${wine_env[@]}" "$WINE_DIR/bin/wine" 'C:\windows\syswow64\cmd.exe' /c exit
    smoke_check "the i386 cmd.exe" 120 "$log"

    # One console program per PE architecture in SMOKE_PE_ARCHS, built here so
    # the test does not depend on what the prefix happens to contain. Each
    # prints a marker naming the architecture it was compiled for, then
    # tries dxgi.dll and d3d9.dll and says whether both loaded. Its compatdb
    # lines say which trees it got: the x86 defaults for an x86 program, and
    # wined3d for both families for a native ARM64 one, whose code the dxmt
    # and mtld3d trees do not hold (Dxgi::default_for and D3d9::default_for in
    # compatdb/table). The loads have to work for the x86 programs, which are
    # what the bundle is for. A native ARM64 program cannot load them: the
    # prefix's placeholders for those names are AMD64, because a placeholder
    # serves one machine and an x64 process needs that one
    # (docs/arm64.md), so its line is only shown.
    for arch in $SMOKE_PE_ARCHS; do
        cc="$MINGW_DIR/bin/$arch-w64-mingw32-clang"
        exe="$TMP_DIR/hello-$arch.exe"
        "$cc" -O2 -o "$exe" "$SCRIPT_DIR/smoke/hello.c"
        time_mark "smoke: hello.exe $arch"
        echo "  Running hello.exe for $arch..."
        log="$TMP_DIR/hello-$arch.log"
        run_limited 120 "$log" "${wine_env[@]}" "$WINE_DIR/bin/wine" "$exe"
        smoke_check "hello.exe for $arch" 120 "$log"
        markers=("wine-build smoke test: hello from $arch")
        if [ "$arch" != aarch64 ]; then
            markers+=("wine-build smoke test: $arch loads dxgi.dll d3d9.dll")
        fi
        for marker in "${markers[@]}"; do
            if ! grep -q "$marker" "$log"; then
                echo "Error: hello.exe for $arch did not print '$marker'"
                cat "$log"
                smoke_stop
                exit 1
            fi
        done
        grep "wine-build smoke test: $arch " "$log" | sed 's/^/    /'
        if [ "$arch" = aarch64 ]; then
            check_default_trees "$log" wined3d wined3d
        else
            check_default_trees "$log"
        fi
    done
    echo "  Loading each Direct3D tree's modules..."
    check_tree_loads x86_64 "${wine_env[@]}"
    check_tree_loads i686 "${wine_env[@]}"
    smoke_stop
    echo "  Programs for $SMOKE_PE_ARCHS run; the x86 ones load their Direct3D trees."
}

# smoke_check <what> <seconds> <log>: after run_limited, exit with the log
# unless the run ended cleanly.
smoke_check() {
    if [ "$RUN_STATUS" -eq 142 ]; then
        echo "Error: $1 did not exit within $2 seconds"
    elif [ "$RUN_STATUS" -ne 0 ]; then
        echo "Error: $1 failed under the bundled wine (status $RUN_STATUS)"
    else
        return 0
    fi
    cat "$3"
    smoke_stop
    exit 1
}

# smoke_stop: stop the throwaway prefix's wineserver, which is persistent
# and has to be told to go rather than waited for.
smoke_stop() {
    WINEPREFIX="$TMP_DIR/prefix" "$WINE_DIR/bin/wineserver" -k 2>/dev/null || true
}

# check_tree_loads <arch> <env-command>...: build smoke/hello.c for <arch>
# (x86_64 or i686) and run it in the booted throwaway prefix once per
# Direct3D tree that has modules for it, with a WINE_COMPATDB rule that gives
# the program that tree, and exit unless compatdb.so reports the tree and the
# program loads every module in it, plus the other family's d3d9.dll or
# dxgi.dll. This is what proves the default-dir placeholders let each tree's
# modules into a process of that arch. <env-command> is the env invocation
# the runs start with; the rule is appended to it. gptk is skipped on a
# macOS older than GPTK_FLOOR, where compatdb.so hands out dxmt instead.
check_tree_loads() {
    local arch="$1" dir=i386-windows exe log family impl other names f want header
    shift
    [ "$arch" = i686 ] || dir="$PE64_DIR"
    # The table format header WINE_COMPATDB starts with, from its source.
    header=$(sed -n 's/^pub const HEADER: &str = "\(.*\)";$/\1/p' "$SCRIPT_DIR/compatdb/table/src/lib.rs")
    exe="$TMP_DIR/trees-$arch.exe"
    "$MINGW_DIR/bin/$arch-w64-mingw32-clang" -O2 -o "$exe" "$SCRIPT_DIR/smoke/hello.c"
    for family in dxgi d3d9; do
        if [ "$family" = dxgi ]; then other=d3d9.dll; else other=dxgi.dll; fi
        for impl in $(if [ "$family" = dxgi ]; then echo "$DXGI_IMPLS"; else echo "$D3D9_IMPLS"; fi); do
            [ -d "$WINE_DIR/lib/wine/$family/$impl/$dir" ] || continue
            if [ "$impl" = gptk ] && macos_newer "$GPTK_FLOOR" "$(sw_vers -productVersion)"; then
                echo "    $arch, $family=$impl: skipped below macOS $GPTK_FLOOR"
                continue
            fi
            names=()
            for f in "$WINE_DIR/lib/wine/$family/$impl/$dir"/*.dll; do
                names+=("$(basename "$f")")
            done
            names+=("$other")
            time_mark "smoke: $arch on $family/$impl"
            log="$TMP_DIR/trees-$arch-$family-$impl.log"
            run_limited 120 "$log" "$@" \
                WINE_COMPATDB="$header"$'\n'"name=smoke;exe=$(basename "$exe");$family=$impl" \
                "$WINE_DIR/bin/wine" "$exe" "${names[@]}"
            smoke_check "the $arch program on the $family/$impl tree" 120 "$log"
            for want in "compatdb: .*$family = $impl (from" \
                        "wine-build smoke test: $arch loads ${names[*]}"; do
                if ! grep -q "$want" "$log"; then
                    echo "Error: the $arch program on the $family/$impl tree did not print '$want'"
                    cat "$log"
                    smoke_stop
                    exit 1
                fi
            done
            echo "    $arch, $family=$impl: loads ${names[*]}"
        done
    done
}

# unpack_dxmt: fetch DXMT's release tarball and unpack it once, setting
# DXMT_TAR and DXMT_SRC (its top-level directory).
unpack_dxmt() {
    [ -z "${DXMT_SRC:-}" ] || return 0
    DXMT_TAR=$(fetch "$DXMT_URL" "$DXMT_SHA256")
    mkdir -p "$TMP_DIR/dxmt"
    tar xzf "$DXMT_TAR" -C "$TMP_DIR/dxmt"
    DXMT_SRC=$(find "$TMP_DIR/dxmt" -mindepth 1 -maxdepth 1 -type d -print -quit)
    if [ -z "$DXMT_SRC" ]; then
        echo "Error: $(basename "$DXMT_TAR") has no top-level directory"
        exit 1
    fi
}

# check_fex_build: exit unless build-fex.sh built all of FEX from the commit
# fex-src.ref names now. Needs the network, to resolve a branch or tag.
check_fex_build() {
    local t tool built want
    for tool in cmake ninja; do
        if ! command -v "$tool" >/dev/null; then
            echo "Error: $tool not found (installing FEX from its build trees needs it)"
            exit 1
        fi
    done
    for t in wow64 arm64ec unixlib; do
        if [ ! -f "$FEX_BUILD_DIR/$t/build.ninja" ]; then
            echo "Error: no FEX build at $FEX_BUILD_DIR/$t (run build-wine.sh --arm64)"
            exit 1
        fi
    done
    built=$(cat "$FEX_BUILD_DIR/COMMIT" 2>/dev/null) || {
        echo "Error: $FEX_BUILD_DIR has no COMMIT; the last build-fex.sh run did not finish"
        exit 1
    }
    want=$(fex_ref_commit)
    if [ -z "$want" ]; then
        echo "Error: cannot resolve fex-src.ref ($(tr -d '[:space:]' < "$SCRIPT_DIR/fex-src.ref")) in $FEX_REPO"
        exit 1
    fi
    if [ "$built" != "$want" ]; then
        echo "Error: FEX was built from $built, but fex-src.ref now names $want"
        echo "(run build-fex.sh)"
        exit 1
    fi
    echo "  FEX: $built"
}

# check_wine_sdk: exit unless the Wine SDK build-wine.sh staged is for the
# Wine source as it is now, which DXMT's DLLs were linked against and whose
# x86_64 link libraries a full bundle carries.
check_wine_sdk() {
    if [ "$(cat "$WINE_SDK_DIR/WINE_COMMIT" 2>/dev/null)" != "$(wine_src_commit)" ]; then
        echo "Error: the Wine SDK at $WINE_SDK_DIR is for $(cat "$WINE_SDK_DIR/WINE_COMMIT" 2>/dev/null || echo nothing),"
        echo "but $WINE_SRC is at $(wine_src_commit) (run build-wine.sh --arm64)"
        exit 1
    fi
    echo "  Wine SDK: $(cat "$WINE_SDK_DIR/WINE_COMMIT")"
}

# check_dxmt_build: exit unless build-dxmt.sh staged all of DXMT from the
# pinned commit. The DLLs and winemetal.so share a private call interface, so
# they must be one build: the COMMIT build-dxmt.sh wrote last, and the
# DESCRIBE it recorded, which d3d11.dll carries as its version string.
check_dxmt_build() {
    local f describe
    if [ "$(cat "$DXMT_DIST/COMMIT" 2>/dev/null)" != "$DXMT_GIT_COMMIT" ]; then
        echo "Error: $DXMT_DIST was not built from $DXMT_GIT_COMMIT (run build-dxmt.sh)"
        exit 1
    fi
    for f in x86_64-windows/dxgi.dll x86_64-windows/d3d10core.dll x86_64-windows/d3d11.dll \
             x86_64-windows/winemetal.dll i386-windows/dxgi.dll i386-windows/d3d10core.dll \
             i386-windows/d3d11.dll i386-windows/winemetal.dll "$UNIX_DIR/winemetal.so" DESCRIBE; do
        if [ ! -f "$DXMT_DIST/$f" ]; then
            echo "Error: no $f in $DXMT_DIST (run build-dxmt.sh)"
            exit 1
        fi
    done
    # Linked against this Wine source's winemac.so, ntdll.so and SDK.
    if [ "$(cat "$DXMT_DIST/WINE_COMMIT" 2>/dev/null)" != "$(wine_src_commit)" ]; then
        echo "Error: $DXMT_DIST was built against Wine $(cat "$DXMT_DIST/WINE_COMMIT" 2>/dev/null || echo unknown),"
        echo "but $WINE_SRC is at $(wine_src_commit) (run build-dxmt.sh)"
        exit 1
    fi
    describe=$(cat "$DXMT_DIST/DESCRIBE")
    for f in x86_64-windows/d3d11.dll i386-windows/d3d11.dll; do
        if ! strings -a "$DXMT_DIST/$f" | grep -qxF "$describe"; then
            echo "Error: $DXMT_DIST/$f is not DXMT $describe"
            exit 1
        fi
    done
    echo "  DXMT: $describe ($DXMT_GIT_COMMIT)"
}

# ── Loader app ──────────────────────────────────────────────────────────
# The arm64 loader runs x86 code through FEX, which needs the hardware x86
# memory ordering macOS only grants to a process signed with
# com.apple.developer.cross-architecture-support. That entitlement is
# restricted: it is only honoured with a provisioning profile that grants it,
# embedded in an app bundle, and a signature by a certificate of the same
# team. So the loader moves into lib/wine/<unix>/wine.app and everything that
# starts a Wine process reaches it through a symlink. Everything about the
# signature comes from the profile in LOADER_PROFILE: the entitlements, the
# bundle identifier and the team whose certificate signs.

# loader_preflight: read LOADER_PROFILE and find the signing identity, or
# exit. Sets LOADER_PLIST (the decoded profile), LOADER_TEAM,
# LOADER_BUNDLE_ID, LOADER_IDENTITY (the certificate's SHA-1) and
# LOADER_IDENTITY_NAME. Runs before anything is deleted, so a bad profile
# costs nothing.
loader_preflight() {
    local expires now appid certs cert i id
    if [ ! -f "$LOADER_PROFILE" ]; then
        echo "Error: no provisioning profile at $LOADER_PROFILE"
        exit 1
    fi
    LOADER_PLIST="$TMP_DIR/loader-profile.plist"
    security cms -D -i "$LOADER_PROFILE" > "$LOADER_PLIST" 2>/dev/null || {
        echo "Error: $LOADER_PROFILE is not a signed provisioning profile"
        exit 1
    }
    # Both are ISO 8601 in UTC, so they compare as strings.
    expires=$(plutil -extract ExpirationDate raw -o - "$LOADER_PLIST" 2>/dev/null) || {
        echo "Error: the provisioning profile has no ExpirationDate"
        exit 1
    }
    now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    if [[ "$expires" < "$now" ]]; then
        echo "Error: the provisioning profile expired on $expires"
        exit 1
    fi
    if [ "$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.developer.cross-architecture-support' \
            "$LOADER_PLIST" 2>/dev/null)" != true ]; then
        echo "Error: the provisioning profile does not grant"
        echo "com.apple.developer.cross-architecture-support"
        exit 1
    fi
    LOADER_TEAM=$(/usr/libexec/PlistBuddy -c 'Print :TeamIdentifier:0' "$LOADER_PLIST" 2>/dev/null) || {
        echo "Error: the provisioning profile names no TeamIdentifier"
        exit 1
    }
    appid=$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.application-identifier' \
            "$LOADER_PLIST" 2>/dev/null) || {
        echo "Error: the provisioning profile grants no com.apple.application-identifier"
        exit 1
    }
    LOADER_BUNDLE_ID=${appid#"$LOADER_TEAM".}
    if [ -z "$LOADER_TEAM" ] || [ "$LOADER_BUNDLE_ID" = "$appid" ]; then
        echo "Error: the application identifier $appid does not start with the team $LOADER_TEAM"
        exit 1
    fi
    # The identity is picked by SHA-1 rather than by name, since several
    # certificates can share a name: the first valid one of the profile's
    # team that is also one of the certificates the profile lists, since
    # macOS rejects the profile for a signature by any other.
    certs=""
    i=0
    while cert=$(plutil -extract "DeveloperCertificates.$i" raw -o - "$LOADER_PLIST" 2>/dev/null); do
        certs="$certs $(printf '%s' "$cert" | base64 --decode | shasum -a 1 | cut -d' ' -f1 | tr a-f A-F)"
        i=$((i + 1))
    done
    LOADER_IDENTITY=""
    for id in $(security find-identity -v -p codesigning \
                | awk -v team="($LOADER_TEAM)\"" 'index($0, team) { print $2 }'); do
        if in_list "$id" "$certs"; then
            LOADER_IDENTITY=$id
            break
        fi
    done
    if [ -z "$LOADER_IDENTITY" ]; then
        echo "Error: no valid code signing identity of team $LOADER_TEAM in the keychain"
        echo "is among the certificates the provisioning profile lists"
        exit 1
    fi
    LOADER_IDENTITY_NAME=$(security find-identity -v -p codesigning \
        | awk -v id="$LOADER_IDENTITY" '$2 == id { sub(/^[^"]*"/, ""); sub(/"$/, ""); print; exit }')
    echo "  Loader: $LOADER_BUNDLE_ID, signed by $LOADER_IDENTITY_NAME (profile expires $expires)."
}

# The entitlements the loader claims on top of the profile's, for the hardened
# runtime, which notarization and Mach9's install verification require. Each
# lifts a restriction of the runtime that Wine runs into:
#   cs.allow-jit                      FEX's code buffers are MAP_JIT memory
#                                     (MEM_EXTENDED_PARAMETER_WINE_UNIX_JIT)
#   cs.allow-unsigned-executable-memory
#                                     ntdll gives anonymous memory an
#                                     executable protection (VirtualAlloc and
#                                     VirtualProtect with PAGE_EXECUTE_*)
#   cs.disable-executable-page-protection
#                                     Windows code expects a
#                                     PAGE_EXECUTE_READWRITE page to be
#                                     writable and executable at once, which
#                                     the first two do not allow
#   cs.disable-library-validation     the unix modules, Homebrew's dylibs and
#                                     FEX's unixlibs are signed ad hoc, not by
#                                     the loader's team
#   device.audio-input                winecoreaudio's capture devices (voice
#                                     chat); the runtime denies the microphone
#                                     without it. Info.plist carries the
#                                     NSMicrophoneUsageDescription macOS
#                                     requires once the loader is the process
#                                     the microphone prompt is for; without it
#                                     macOS kills the process on access
# On macOS 27 a process with the cross-architecture entitlement runs Wine and
# FEX with only library validation disabled, but the first three are what
# Apple documents for what Wine does, and they keep it running if that
# relaxation goes. Not claimed: device.camera (this Wine has no video capture
# on macOS) and cs.allow-dyld-environment-variables (nothing in the bundle
# uses DYLD_* variables; the libraries are found through @loader_path).
LOADER_RUNTIME_ENTITLEMENTS="com.apple.security.cs.allow-jit
com.apple.security.cs.allow-unsigned-executable-memory
com.apple.security.cs.disable-executable-page-protection
com.apple.security.cs.disable-library-validation
com.apple.security.device.audio-input"

# package_loader: wrap the loader `make install` left in lib/wine/<unix>/wine
# into wine.app and sign it. Hadron's layout:
#   lib/wine/<unix>/wine.app/Contents/{MacOS/wine, Info.plist, embedded.provisionprofile}
#   lib/wine/<unix>/wine -> wine.app/Contents/MacOS/wine   (what ntdll execs)
#   bin/wine -> ../lib/wine/<unix>/wine.app/Contents/MacOS/wine
# The app is signed as a whole, without --deep (nothing inside it is code but
# the loader), with the hardened runtime and, unless LOADER_TIMESTAMP=0, a
# secure timestamp from Apple's server.
package_loader() {
    local app="$unixdir/wine.app" ents="$TMP_DIR/loader-entitlements.plist" version key
    if [ ! -f "$unixdir/wine" ] || [ -L "$unixdir/wine" ]; then
        echo "Error: no loader binary at $UNIX_DIR/wine"
        exit 1
    fi
    version=$("$unixdir/wine" --version 2>/dev/null | sed 's/^wine-//')
    [ -n "$version" ] || version=0
    mkdir -p "$app/Contents/MacOS"
    mv "$unixdir/wine" "$app/Contents/MacOS/wine"
    cp "$LOADER_PROFILE" "$app/Contents/embedded.provisionprofile"
    # Named Mach9, the launcher that runs this Wine: macOS shows the running
    # executable's bundle in the app menu, and a game process runs wine.app's
    # executable. The category and the Game Mode keys match Mach9's own
    # Info.plist, so a game the launcher starts gets Game Mode.
    cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>wine</string>
    <key>CFBundleIdentifier</key>
    <string>$LOADER_BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>Mach9</string>
    <key>CFBundleDisplayName</key>
    <string>Mach9</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.games</string>
    <key>LSSupportsGameMode</key>
    <true/>
    <key>GCSupportsGameMode</key>
    <true/>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSMinimumSystemVersion</key>
    <string>$MACOS_FLOOR</string>
    <key>NSPrincipalClass</key>
    <string>WineApplication</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>A Windows program running in Wine wants to record audio from the microphone.</string>
</dict>
</plist>
EOF
    ln -s wine.app/Contents/MacOS/wine "$unixdir/wine"
    rm -f "$WINE_DIR/bin/wine"
    ln -s "../lib/wine/$UNIX_DIR/wine.app/Contents/MacOS/wine" "$WINE_DIR/bin/wine"

    # Only what the loader needs: its identity and the cross-architecture
    # entitlement, all three granted by the profile, the custom x18 ABI,
    # which FEX needs because Windows code uses x18 as its TEB pointer, and
    # LOADER_RUNTIME_ENTITLEMENTS. Nothing else the profile allows (keychain
    # access groups) is claimed.
    rm -f "$ents"
    /usr/libexec/PlistBuddy \
        -c "Add :com.apple.application-identifier string $LOADER_TEAM.$LOADER_BUNDLE_ID" \
        -c "Add :com.apple.developer.team-identifier string $LOADER_TEAM" \
        -c 'Add :com.apple.developer.cross-architecture-support bool true' \
        -c 'Add :com.apple.security.custom-x18-abi-toggle bool true' \
        "$ents" >/dev/null
    for key in $LOADER_RUNTIME_ENTITLEMENTS; do
        /usr/libexec/PlistBuddy -c "Add :$key bool true" "$ents" >/dev/null
    done
    echo "    wine.app ($LOADER_BUNDLE_ID)"
    if ! codesign --force --options runtime "$(loader_timestamp_flag)" --sign "$LOADER_IDENTITY" \
            --entitlements "$ents" "$app"; then
        if [ "$LOADER_TIMESTAMP" = 1 ]; then
            echo "Error: signing wine.app failed; a secure timestamp needs Apple's timestamp"
            echo "server, so offline set LOADER_TIMESTAMP=0 (the result cannot be notarized)"
        else
            echo "Error: signing wine.app failed"
        fi
        exit 1
    fi
}

# loader_timestamp_flag: the codesign option for LOADER_TIMESTAMP.
loader_timestamp_flag() {
    if [ "$LOADER_TIMESTAMP" = 1 ]; then echo --timestamp; else echo --timestamp=none; fi
}

# check_loader_app: fail unless wine.app carries a valid signature by the
# profile's team with the entitlements package_loader gave it, and both entry
# points lead to it.
check_loader_app() {
    local app="$unixdir/wine.app" info signed="$TMP_DIR/signed-entitlements.plist" key want got pair
    if ! codesign --verify --strict "$app" 2>"$TMP_DIR/codesign.log"; then
        echo "Error: wine.app does not verify:"
        cat "$TMP_DIR/codesign.log"
        exit 1
    fi
    info=$(codesign -dvv "$app" 2>&1)
    if ! grep -qxF "Authority=$LOADER_IDENTITY_NAME" <<< "$info" \
        || ! grep -qxF "TeamIdentifier=$LOADER_TEAM" <<< "$info"; then
        echo "Error: wine.app is not signed by $LOADER_IDENTITY_NAME:"
        echo "$info"
        exit 1
    fi
    if ! grep -q '^CodeDirectory.*flags=.*runtime' <<< "$info"; then
        echo "Error: wine.app is not signed with the hardened runtime"
        exit 1
    fi
    if [ "$LOADER_TIMESTAMP" = 1 ] && ! grep -q '^Timestamp=' <<< "$info"; then
        echo "Error: wine.app's signature has no secure timestamp"
        exit 1
    fi
    codesign -d --entitlements - --xml "$app" > "$signed" 2>/dev/null
    # Exactly the set package_loader wrote: nothing missing, nothing extra.
    want=$(printf '%s\n' com.apple.application-identifier com.apple.developer.team-identifier \
               com.apple.developer.cross-architecture-support com.apple.security.custom-x18-abi-toggle \
               $LOADER_RUNTIME_ENTITLEMENTS | sort)
    got=$(/usr/libexec/PlistBuddy -c Print "$signed" 2>/dev/null \
              | sed -n 's/^    \([^ ]*\) = .*/\1/p' | sort)
    if [ "$got" != "$want" ]; then
        echo "Error: wine.app's entitlements are not the expected set:"
        diff <(echo "$want") <(echo "$got") | sed 's/^/    /'
        exit 1
    fi
    for key in com.apple.developer.cross-architecture-support com.apple.security.custom-x18-abi-toggle \
               $LOADER_RUNTIME_ENTITLEMENTS; do
        if [ "$(/usr/libexec/PlistBuddy -c "Print :$key" "$signed" 2>/dev/null)" != true ]; then
            echo "Error: wine.app's signature does not set $key to true"
            exit 1
        fi
    done
    if [ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.application-identifier' "$signed" 2>/dev/null)" \
            != "$LOADER_TEAM.$LOADER_BUNDLE_ID" ]; then
        echo "Error: wine.app's application identifier is not $LOADER_TEAM.$LOADER_BUNDLE_ID"
        exit 1
    fi
    if [ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.developer.team-identifier' "$signed" 2>/dev/null)" \
            != "$LOADER_TEAM" ]; then
        echo "Error: wine.app's team identifier is not $LOADER_TEAM"
        exit 1
    fi
    if [ "$(readlink "$WINE_DIR/bin/wine")" != "../lib/wine/$UNIX_DIR/wine.app/Contents/MacOS/wine" ] \
        || [ "$(readlink "$unixdir/wine")" != "wine.app/Contents/MacOS/wine" ]; then
        echo "Error: bin/wine and $UNIX_DIR/wine must be the symlinks into wine.app"
        exit 1
    fi
    # The Mach9 keys package_loader writes; the signature binds Info.plist,
    # which the strict verification above checks.
    for pair in CFBundleName=Mach9 CFBundleDisplayName=Mach9 \
                LSApplicationCategoryType=public.app-category.games \
                LSSupportsGameMode=true GCSupportsGameMode=true; do
        if [ "$(/usr/libexec/PlistBuddy -c "Print :${pair%%=*}" "$app/Contents/Info.plist" 2>/dev/null)" \
                != "${pair#*=}" ]; then
            echo "Error: wine.app's Info.plist does not set ${pair%%=*} to ${pair#*=}"
            exit 1
        fi
    done
    echo "  wine.app is named Mach9 and supports Game Mode."
    if [ "$LOADER_TIMESTAMP" = 1 ]; then
        echo "  wine.app is signed by $LOADER_IDENTITY_NAME with the hardened runtime, a secure timestamp"
    else
        echo "  wine.app is signed by $LOADER_IDENTITY_NAME with the hardened runtime, no secure timestamp"
        echo "  (LOADER_TIMESTAMP=0: it cannot be notarized)"
    fi
    echo "  and the cross-architecture entitlement."
}

# ── compatdb-only: replace compatdb.so in an existing bundle ────────────
# For a change confined to the compatdb/ crate. Nothing is wiped, nothing is
# prompted for and nothing is read from $BUILD_DIR except cargo's target dir,
# so this works on an unpacked release tarball as well as on a bundle this
# script built.
if [ "$COMPATDB_ONLY" -eq 1 ]; then
    if [ ! -x "$WINE_DIR/bin/wine" ]; then
        echo "Error: no bundle at $WINE_DIR (--compatdb-only updates an existing"
        echo "bundle; build one without it first, or unpack a release tarball there)"
        exit 1
    fi

    echo "==> Step 1: Build compatdb.so"
    build_compatdb

    # Same reason as Step 4f of a full bundle. A freshly linked file normally
    # carries no quarantine, so only remove it when it is there.
    echo "==> Step 2: Remove inherited quarantine"
    if xattr -p com.apple.quarantine "$unixdir/compatdb.so" >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$unixdir/compatdb.so"
    fi

    echo "==> Step 3: Verify"
    if ! cmp -s "$COMPATDB_BUILD" "$unixdir/compatdb.so"; then
        echo "Error: $UNIX_DIR/compatdb.so does not match the build output"
        exit 1
    fi
    echo "  compatdb.so: $(file -b "$unixdir/compatdb.so")"
    LEAKED=0
    check_no_leaks "$unixdir/compatdb.so"
    if [ $LEAKED -ne 0 ]; then
        echo "Error: compatdb.so is not self-contained"
        exit 1
    fi
    # Only compatdb.so: the rest of the bundle was not built here, and a base
    # release keeps whatever floor it shipped with.
    check_minos "$unixdir/compatdb.so"
    check_query_export
    if [ "$WITH_X87SIDECAR" -eq 0 ]; then
        check_no_x87
    fi
    "$SMOKE_TEST"

    echo ""
    echo "==> Done! compatdb.so updated in:"
    echo "    $WINE_DIR/"
    exit 0
fi

# ── Preflight ───────────────────────────────────────────────────────────
# What a full bundle needs beyond the Wine build, checked before Step 0
# deletes anything.
time_mark "Preflight"
echo "==> Preflight"
check_sdk_floor
check_deps_formulae
if [ "$WITH_FEX" -eq 1 ]; then
    check_fex_build
fi
if [ "$WITH_WINE_SDK" -eq 1 ]; then
    check_wine_sdk
fi
if [ "$WITH_DXMT_BUILD" -eq 1 ]; then
    check_dxmt_build
fi
for arch in $SMOKE_PE_ARCHS x86_64 i686; do
    if [ ! -x "$MINGW_DIR/bin/$arch-w64-mingw32-clang" ]; then
        echo "Error: no $arch-w64-mingw32-clang in $MINGW_DIR/bin (the smoke test needs it)"
        exit 1
    fi
done
for tool in llvm-strip llvm-objdump; do
    if [ ! -x "$MINGW_DIR/bin/$tool" ]; then
        echo "Error: no $tool in $MINGW_DIR/bin (Step 4e needs it)"
        exit 1
    fi
done
if [ "$WITH_LOADER_APP" -eq 1 ]; then
    loader_preflight
fi

time_mark "Step 0: Clean previous bundle"
# ── Step 0: Clean previous bundle ──────────────────────────────────────
# The bundle ships, so it carries no tree marker. Only a directory that looks
# like a bundle, one with a bin/wine, is offered for deletion. make install
# writes the tree under <dest>/<INSTALL_PREFIX>, which Step 2 moves away and
# then removes, so that directory must not be there already.
INSTALL_TOP="${INSTALL_PREFIX#/}"
INSTALL_TOP="${INSTALL_TOP%%/*}"
if [ -e "$DIST_DIR/$INSTALL_TOP" ]; then
    echo "Error: $DIST_DIR/$INSTALL_TOP exists and the staged install would go into it;"
    echo "remove it first if an interrupted run left it"
    exit 1
fi
if [ -d "$WINE_DIR" ]; then
    if [ ! -e "$WINE_DIR/bin/wine" ] && [ ! -L "$WINE_DIR/bin/wine" ] \
        && [ -n "$(ls -A "$WINE_DIR")" ]; then
        echo "Error: $WINE_DIR is not empty and has no bin/wine; not deleting what is not a bundle"
        exit 1
    fi
    echo "Will delete existing bundle: $WINE_DIR"
    if [ "$ASSUME_YES" -eq 0 ]; then
        read -r -p "Continue? [y/N] " confirm
        case "$confirm" in
            [yY]) ;;
            *) echo "Aborted."; exit 1 ;;
        esac
    fi
    remove_tree "$WINE_DIR"
fi
# The previous bundle's symbols go too, so a run that fails before Step 4e,
# or one without --symbols, cannot leave them next to a bundle they do not
# belong to.
if [ "$WITH_SYMBOLS" -eq 1 ]; then
    claim_tree "$SYMBOLS_DIR" symbols 1
elif [ -e "$SYMBOLS_DIR" ]; then
    claim_tree "$SYMBOLS_DIR" symbols 1
    remove_tree "$SYMBOLS_DIR"
fi
mkdir -p "$DIST_DIR"

# ── Step 1: Staged install ──────────────────────────────────────────────
time_mark "Step 1: Staged install"
# Under the same wrapper as configure, since it can rerun configure's probes.
cd "$BUILD_DIR"
if [ "$RUNTIME_ONLY" -eq 1 ]; then
    echo "==> Step 1: Staged install (runtime only)"
    "${CONFIGURE_WRAPPER[@]}" make install-lib DESTDIR="$DIST_DIR"
else
    echo "==> Step 1: Staged install with DESTDIR"
    "${CONFIGURE_WRAPPER[@]}" make install DESTDIR="$DIST_DIR"
fi

# ── Step 2: Flatten prefix ──────────────────────────────────────────────
time_mark "Step 2: Flatten prefix"
echo "==> Step 2: Flatten prefix"
mv "$DIST_DIR$INSTALL_PREFIX" "$WINE_DIR"
# Left behind is the empty parent of the prefix (/usr of /usr/local), unless
# a Finder window showing <dest> put a .DS_Store into it.
rm -f "$DIST_DIR/$INSTALL_TOP/.DS_Store"
rmdir "$DIST_DIR/$INSTALL_TOP"

EXT_DIR="$WINE_DIR/lib/external"

# ── Step 2b: Bundle the d3d9 test binaries ──────────────────────────────
# Wine's `make install` skips test binaries, so copy the two `d3d9_test.exe`
# in by hand. They are the de-facto D3D9 conformance suite, and shipping them
# lets a consumer gate its own d3d9 builtin against the suite with nothing but
# this bundle (no Wine build tree, which is what a CI job has). Plain PEs that
# `wine` executes, so they are NOT builtin-marked, and they live outside the
# per-arch module directories the loader searches by module name.
#
# Development files only: the runtime-only bundle goes into an application, and
# a test suite has no business there.
if [ "$RUNTIME_ONLY" -eq 0 ] && [ "$WITH_D3D9_TESTS" -eq 1 ]; then
    time_mark "Step 2b: d3d9 test binaries"
    echo "==> Step 2b: Bundle the d3d9 test binaries"
    for arch in i386-windows "$PE64_DIR"; do
        src="$BUILD_DIR/dlls/d3d9/tests/$arch/d3d9_test.exe"
        if [ ! -f "$src" ]; then
            echo "Error: $src not found (run build-wine.sh first)"
            exit 1
        fi
        echo "    $arch/d3d9_test.exe"
        mkdir -p "$WINE_DIR/lib/wine/tests/$arch"
        cp "$src" "$WINE_DIR/lib/wine/tests/$arch/"
    done
fi

# ── Step 2c: x86_64 link libraries ─────────────────────────────────────
# A full bundle carries Wine's link libraries, so it can serve as the Wine
# SDK of PE code built against it. `make install` puts those of the build's
# own PE architectures in place; the x86_64 ones, which an arm64 Wine cannot
# build, come from the Wine SDK build-wine.sh staged, so the bundle offers
# the same set as that SDK. The directory holds nothing else and nothing
# reads it at run time: on an arm64 host ntdll looks for an x64 builtin in
# aarch64-windows, and the loader and setupapi's fake-DLL install only pick
# .dll and .exe files that are PE images.
if [ "$RUNTIME_ONLY" -eq 0 ] && [ "$WITH_WINE_SDK" -eq 1 ]; then
    time_mark "Step 2c: x86_64 link libraries"
    echo "==> Step 2c: x86_64 link libraries"
    mkdir -p "$WINE_DIR/lib/wine/x86_64-windows"
    for lib in "$WINE_SDK_DIR"/lib/wine/x86_64-windows/*.a; do
        echo "    x86_64-windows/$(basename "$lib")"
        cp "$lib" "$WINE_DIR/lib/wine/x86_64-windows/"
    done
fi

# ── Step 3: Bundle dynamic libraries ────────────────────────────────────
# Wine's .so modules dlopen these via @loader_path sonames (patched by
# build-wine.sh). Transitive deps are loaded by dyld from their @loader_path
# install names.
time_mark "Step 3: Bundle dynamic libraries"
echo "==> Step 3: Bundle dynamic libraries"
mkdir -p "$EXT_DIR"

# Direct deps: Wine dlopen's these by soname. build-wine.sh patches their
# sonames in config.h to @loader_path/../../external/<name>, so config.h is
# the authoritative (version-correct) list; resolve each against
# $DEPS_PREFIX/lib. Deduplicated, since one library can back several defines.
CONFIG_H="$BUILD_DIR/include/config.h"
if [ ! -f "$CONFIG_H" ]; then
    echo "Error: $CONFIG_H not found (run build-wine.sh first)"
    exit 1
fi
LIBS=()
for name in $(sed -n 's|.*"@loader_path/\.\./\.\./external/\([^"]*\)".*|\1|p' "$CONFIG_H" | sort -u); do
    LIBS+=("$DEPS_PREFIX/lib/$name")
done
if [ ${#LIBS[@]} -eq 0 ]; then
    echo "Error: no @loader_path sonames in $CONFIG_H: the build is not"
    echo "relocatable (config.h lost its soname patches). Re-run build-wine.sh."
    exit 1
fi
# Homebrew's "libSDL2" is sdl2-compat, a shim that loads real SDL3 at runtime
# via @loader_path/libSDL3.dylib, so SDL3 must sit beside it in the bundle.
LIBS+=("$DEPS_PREFIX/lib/libSDL3.dylib")

echo "  Copying direct deps..."
MISSED=0
for lib in "${LIBS[@]}"; do
    name=$(basename "$lib")
    if [ -f "$lib" ]; then
        echo "    $name"
        cp -L "$lib" "$EXT_DIR/$name"
    else
        echo "    MISSING: $name from $lib"
        MISSED=1
    fi
done

# Transitive deps: walk the otool -L closure, copying every $DEPS_PREFIX
# dependency until no new ones appear. Line 2 of otool -L is the dylib's
# own install name, not a dependency, so skip it.
echo "  Copying transitive deps..."
while :; do
    added=0
    for dylib in "$EXT_DIR"/*.dylib; do
        for dep in $(otool -L "$dylib" | tail -n +3 | awk '{print $1}'); do
            case "$dep" in
                "$DEPS_PREFIX"/*)
                    depname=$(basename "$dep")
                    if [ -f "$EXT_DIR/$depname" ]; then
                        :
                    elif [ -f "$dep" ]; then
                        echo "    $depname (needed by $(basename "$dylib"))"
                        cp -L "$dep" "$EXT_DIR/$depname"
                        added=1
                    else
                        echo "    MISSING: $depname (needed by $(basename "$dylib")) from $dep"
                        MISSED=1
                    fi
                    ;;
            esac
        done
    done
    [ $added -eq 0 ] && break
done
if [ $MISSED -ne 0 ]; then
    echo "Error: required libraries are missing, bundle would not be self-contained"
    exit 1
fi
chmod +w "$EXT_DIR"/*.dylib

# Fix install names so bundled dylibs reference each other via @loader_path
echo "  Fixing install names..."
for dylib in "$EXT_DIR"/*.dylib; do
    name=$(basename "$dylib")
    install_name_tool -id "@loader_path/$name" "$dylib"
    for dep in $(otool -L "$dylib" | tail -n +2 | awk '{print $1}'); do
        case "$dep" in
            "$DEPS_PREFIX"/*)
                depname=$(basename "$dep")
                if [ -f "$EXT_DIR/$depname" ]; then
                    install_name_tool -change "$dep" "@loader_path/$depname" "$dylib"
                fi
                ;;
        esac
    done
done

# install_name_tool breaks the signature Homebrew's linker gave each dylib,
# and arm64 code with a broken signature is killed on load. An ad-hoc
# signature is enough for a library the signed loader loads: the loader runs
# with library validation disabled (LOADER_RUNTIME_ENTITLEMENTS).
if [ "$SIGN_RELINKED" -eq 1 ]; then
    echo "  Re-signing the relinked dylibs ad hoc..."
    for dylib in "$EXT_DIR"/*.dylib; do
        codesign --force --sign - "$dylib" 2>/dev/null
    done
fi

# ── Step 3b: Remove the modules Vulkan removal orphans ──────────────────
# Wine's own D3D12 (vkd3d) and Vulkan modules cannot work in a build
# configured --without-vulkan. Drop them rather than ship modules that
# advertise an API they cannot serve. An x86_64 d3d12.dll that a Direct3D
# tree supplies (D3DMetal's) is installed after this, in Step 4. d3d10core
# stays: i386's is replaced by DXMT, and on x86_64 it is Wine's, loaded only
# by the wined3d side tree (D3DMetal's dxgi cannot back it, so the default
# GPTK stack never reaches it).
time_mark "Step 3b: Vulkan removal"
echo "==> Step 3b: Remove the modules Vulkan removal orphans"
for dead in \
    i386-windows/vulkan-1.dll \
    i386-windows/winevulkan.dll \
    i386-windows/d3d12.dll \
    i386-windows/d3d12core.dll \
    "$PE64_DIR/vulkan-1.dll" \
    "$PE64_DIR/winevulkan.dll" \
    "$PE64_DIR/d3d12.dll" \
    "$PE64_DIR/d3d12core.dll" \
    "$UNIX_DIR/winevulkan.so"
do
    if [ -e "$WINE_DIR/lib/wine/$dead" ]; then
        echo "    $dead"
        rm -f "$WINE_DIR/lib/wine/$dead"
    fi
done

# ── Step 4: Direct3D backends ───────────────────────────────────────────
# Direct3D comes from third-party implementations that talk to Metal
# directly, plus Wine's own wined3d as the fallback for each: Apple's D3DMetal
# (Game Porting Toolkit, gptk) for x86_64 D3D10-12, DXMT for D3D10 and D3D11
# (all of it where D3DMetal does not exist, the i386 half where it does), and
# mtld3d for D3D9. Which of them a build carries is DXGI_IMPLS and D3D9_IMPLS
# in profile.sh. Every one lives in its own tree under lib/wine/dxgi/<impl>
# or lib/wine/d3d9/<impl>, with <pe64>-windows, i386-windows and, where it
# has a unix half, <unix> subdirectories; the default dirs hold only
# fake-module markers, and compatdb.so (Step 4b) picks one tree per family
# per process. D3D8 and DDraw stay on wined3d only.
#
# Deliberately after Step 3: that step's dependency walk, chmod and
# install_name_tool loops all iterate $EXT_DIR/*.dylib, and none of them may
# touch the Apple-signed libd3dshared.dylib. Staging afterwards keeps them out
# of reach without any exemption logic. Nothing below rewrites install names,
# re-signs, or walks the otool closure.
time_mark "Step 4: Direct3D backends"
echo "==> Step 4: Direct3D backends"

# Every artifact, the x87sidecar of Step 4c included, is downloaded (or taken
# from the cache) before anything is installed, so a bad pin fails the run
# before the tree is half-modified.
echo "  Fetching artifacts into $CACHE_DIR..."
if in_list gptk "$DXGI_IMPLS"; then
    GPTK_DMG=$(fetch "$GPTK_URL" "$GPTK_SHA256")
fi
if in_list dxmt "$DXGI_IMPLS"; then
    if [ "$WITH_DXMT_BUILD" -eq 1 ]; then
        DXMT_SRC="$DXMT_DIST"
    else
        unpack_dxmt
    fi
fi
if in_list mtld3d "$D3D9_IMPLS"; then
    MTLD3D_TAR=$(fetch "$MTLD3D_URL" "$MTLD3D_SHA256")
fi
if [ "$WITH_X87SIDECAR" -eq 1 ]; then
    X87SIDECAR_TAR=$(fetch "$X87SIDECAR_URL" "$X87SIDECAR_SHA256")
    X87SIDECAR_LICENSE=$(fetch "$X87SIDECAR_LICENSE_URL" "$X87SIDECAR_LICENSE_SHA256" \
        x87sidecar-LICENSE)
fi

WINEBUILD="$BUILD_DIR/tools/winebuild/winebuild"
d3d64="$WINE_DIR/lib/wine/$PE64_DIR"
d3d32="$WINE_DIR/lib/wine/i386-windows"
tree="$WINE_DIR/lib/wine/dxgi"
d3d9tree="$WINE_DIR/lib/wine/d3d9"

# wined3d (both arches): Wine's own dxgi/d3d10/d3d10core/d3d11/d3d10_1 from the
# build tree. No d3d12 (vkd3d is unusable --without-vulkan); wined3d.dll itself
# stays in the default dir (the GL engine, shared, reached by fall-through).
if in_list wined3d "$DXGI_IMPLS"; then
    echo "  Building dxgi/wined3d from the Wine build..."
    for arch in i386-windows "$PE64_DIR"; do
        dest="$tree/wined3d/$arch"
        mkdir -p "$dest"
        for dll in dxgi d3d10 d3d10core d3d11 d3d10_1; do
            src="$BUILD_DIR/dlls/$dll/$arch/$dll.dll"
            if [ ! -f "$src" ]; then
                echo "Error: $src not found (run build-wine.sh first)"
                exit 1
            fi
            cp "$src" "$dest/"
        done
    done
fi

if in_list gptk "$DXGI_IMPLS"; then
    # D3DMetal, straight off the GPTK image. Apple's license allows
    # redistributing the Redistributables unmodified for non-commercial
    # purposes, so the files are copied byte for byte, signatures and all.
    # `ditto` preserves the framework's symlinks and its _CodeSignature
    # directory; `cp -R` would not. The image carries a click-through license
    # agreement, which hdiutil prompts for on stdin and would otherwise block
    # a CI run forever. The here-string answers it and PAGER keeps the
    # agreement text from being paged. D3DMetal is x86_64 only, so this
    # profile's 64-bit dirs are x86_64 ones.
    echo "  Mounting $(basename "$GPTK_DMG")..."
    mkdir -p "$GPTK_MNT"
    PAGER=cat hdiutil attach "$GPTK_DMG" \
        -readonly -nobrowse -noautoopen -mountpoint "$GPTK_MNT" -quiet <<< "Y"

    # Located rather than hardcoded, so a renamed volume or a reshuffled image
    # does not silently install nothing.
    GPTK_LIB=$(find "$GPTK_MNT" -maxdepth 4 -type d -path '*/redist/lib' -print -quit)
    if [ -z "$GPTK_LIB" ]; then
        echo "Error: no redist/lib directory on $(basename "$GPTK_DMG")"
        exit 1
    fi
    # A copy as it is on the image, which Step 5 compares the installed files
    # with.
    ditto "$GPTK_LIB" "$TMP_DIR/gptk-image"

    # D3DMetal's PE modules, each as <name on the image>:<name in the bundle>.
    # Each goes into the gptk tree with a unixlib symlink at the deeper path,
    # which stands in for the image's own x86_64-unix symlink of that name.
    # nvngx is what games look for when probing DLSS; Apple ships its MetalFX
    # implementation under a descriptive name that no game ever loads, and
    # wine.inf already registers nvapi64.dll and nvngx.dll as fake DLLs. This
    # one list drives the install, the record of Apple's files that Step 4e
    # and Step 5 work from, and the check that the image holds nothing else.
    GPTK_MODULES="dxgi:dxgi d3d10:d3d10 d3d11:d3d11 d3d12:d3d12 nvapi64:nvapi64 nvngx-on-metalfx:nvngx"
    echo "  Installing D3DMetal..."
    ditto "$GPTK_LIB/external" "$EXT_DIR"
    gw="$tree/gptk/$PE64_DIR"
    gu="$tree/gptk/$UNIX_DIR"
    mkdir -p "$gw" "$gu"
    for m in $GPTK_MODULES; do
        echo "    ${m%%:*}.dll -> dxgi/gptk/$PE64_DIR/${m#*:}.dll"
        ditto "$GPTK_LIB/wine/x86_64-windows/${m%%:*}.dll" "$gw/${m#*:}.dll"
        ln -sf ../../../../external/libd3dshared.dylib "$gu/${m#*:}.so"
    done

    GPTK_LICENSE=$(find "$GPTK_MNT" -maxdepth 2 -iname 'License.rtf' -print -quit)
    if [ -n "$GPTK_LICENSE" ]; then
        echo "    D3DMetal-License.rtf"
        cp "$GPTK_LICENSE" "$EXT_DIR/D3DMetal-License.rtf"
    else
        echo "Error: no License.rtf on $(basename "$GPTK_DMG")"
        exit 1
    fi

    hdiutil detach "$GPTK_MNT" -quiet
    rmdir "$GPTK_MNT" 2>/dev/null || true

    # D3DMetal ships no d3d10core/d3d10_1; Wine's were reachable by GPTK
    # processes in the old default dir (they return E_FAIL on GPTK's dxgi but
    # must be present so anything probing D3D10 keeps its old behaviour rather
    # than failing to load). Pure PE, no unixlib.
    cp "$BUILD_DIR/dlls/d3d10core/$PE64_DIR/d3d10core.dll" "$gw/"
    cp "$BUILD_DIR/dlls/d3d10_1/$PE64_DIR/d3d10_1.dll"     "$gw/"

    # Every regular file the bundle takes from the image, as
    # "<path on the image>|<path in the bundle>" lines. Step 4e leaves these
    # alone and Step 5 checks them against the image copy: they ship as
    # Apple made them. Every entry of the image has to be accounted for: a
    # file or symlink under external/ is installed at the same path under
    # lib/external/, a module named in GPTK_MODULES goes into the gptk tree,
    # and the x86_64-unix symlink of such a module is replaced by the tree's
    # own. Anything else, or a file missing where it should be, is an error,
    # so a change to the image or to this step cannot let an Apple file slip
    # past Step 4e and Step 5.
    while IFS= read -r -d '' f; do
        src="${f#"$TMP_DIR/gptk-image/"}"
        name="${src##*/}"
        dst=""
        case "$src" in
            external/*) dst="lib/$src" ;;
            wine/x86_64-windows/*.dll|wine/x86_64-unix/*.so)
                for m in $GPTK_MODULES; do
                    if [ "${m%%:*}" = "${name%.*}" ]; then
                        dst="lib/wine/dxgi/gptk/$PE64_DIR/${m#*:}.dll"
                    fi
                done ;;
        esac
        if [ -z "$dst" ]; then
            echo "Error: $src on the GPTK image is not installed (GPTK_MODULES does not name it)"
            exit 1
        fi
        if [ -L "$f" ]; then
            case "$src" in
                wine/x86_64-unix/*) ;;
                *)
                    if [ "$(readlink "$f")" != "$(readlink "$WINE_DIR/$dst" 2>/dev/null)" ]; then
                        echo "Error: $src on the GPTK image is not installed as the same symlink at $dst"
                        exit 1
                    fi ;;
            esac
            continue
        fi
        if [ ! -f "$WINE_DIR/$dst" ] || [ -L "$WINE_DIR/$dst" ]; then
            echo "Error: $src from the GPTK image is not installed at $dst"
            exit 1
        fi
        GPTK_FILES+="$src|$dst"$'\n'
    done < <(find "$TMP_DIR/gptk-image" \( -type f -o -type l \) -print0)
fi

if in_list dxmt "$DXGI_IMPLS"; then
    # DXMT_SRC is the release tarball (unpacked above) or, with
    # WITH_DXMT_BUILD, what build-dxmt.sh staged, as the preflight checked.
    # Both have the same layout, with a winemetal.so for this profile.
    echo "  Installing DXMT..."
    echo "    $DXMT_SRC"
    if [ ! -f "$DXMT_SRC/$UNIX_DIR/winemetal.so" ]; then
        echo "Error: no $UNIX_DIR/winemetal.so in $DXMT_SRC"
        exit 1
    fi

    # dxmt tree: DXMT's dxgi/d3d10core/d3d11/winemetal plus Wine's
    # d3d10/d3d10_1, for both arches. DXMT's x86_64 DLLs go into the profile's
    # 64-bit dir, which on arm64 is aarch64-windows (x64 processes run as
    # ARM64EC there). The single winemetal.so stays in the default unix dir,
    # where its @rpath finds winemac.so and ntdll.so, and serves the i386 PE
    # modules too through its wow64 entry points; the tree links to it.
    dx64="$tree/dxmt/$PE64_DIR"
    dxu="$tree/dxmt/$UNIX_DIR"
    dx32="$tree/dxmt/i386-windows"
    mkdir -p "$dx64" "$dxu" "$dx32"
    cp "$DXMT_SRC/$UNIX_DIR/winemetal.so" "$unixdir/"
    cp "$DXMT_SRC"/x86_64-windows/dxgi.dll \
       "$DXMT_SRC"/x86_64-windows/d3d10core.dll \
       "$DXMT_SRC"/x86_64-windows/d3d11.dll \
       "$DXMT_SRC"/x86_64-windows/winemetal.dll \
       "$dx64/"
    cp "$BUILD_DIR/dlls/d3d10/$PE64_DIR/d3d10.dll"     "$dx64/"
    cp "$BUILD_DIR/dlls/d3d10_1/$PE64_DIR/d3d10_1.dll" "$dx64/"
    ln -sf "../../../$UNIX_DIR/winemetal.so" "$dxu/winemetal.so"
    cp "$DXMT_SRC"/i386-windows/dxgi.dll \
       "$DXMT_SRC"/i386-windows/d3d10core.dll \
       "$DXMT_SRC"/i386-windows/d3d11.dll \
       "$DXMT_SRC"/i386-windows/winemetal.dll \
       "$dx32/"
    cp "$BUILD_DIR/dlls/d3d10/i386-windows/d3d10.dll"     "$dx32/"
    cp "$BUILD_DIR/dlls/d3d10_1/i386-windows/d3d10_1.dll" "$dx32/"
fi

# D3D9, the second family: mtld3d and Wine's own. mtld3d's d3d9.dll bridges
# to its mtld3d.dll, whose unix half is a single mtld3d.so that also serves
# the i386 PE through its wow64 entry points. wined3d's d3d9.dll needs only
# wined3d.dll, which stays in the default dir.
if in_list mtld3d "$D3D9_IMPLS"; then
    # mtld3d's tarball mirrors the lib/wine layout under wine/, with an
    # x86_64-unix and an aarch64-unix mtld3d.so; the one for this profile's
    # unix dir is taken. Its x86_64 PE DLLs go into the profile's 64-bit dir,
    # like DXMT's. The tarball's prefix-markers/ holds the system32 and
    # syswow64 placeholders for installs into an existing prefix and is not
    # used: the prefixes this bundle creates get their markers from wine.inf's
    # wildcard over the default dirs (stamped below).
    echo "  Installing mtld3d..."
    mkdir -p "$TMP_DIR/mtld3d"
    tar xf "$MTLD3D_TAR" -C "$TMP_DIR/mtld3d"
    MTLD3D_SRC="$TMP_DIR/mtld3d/wine"
    if [ ! -d "$MTLD3D_SRC" ]; then
        echo "Error: $(basename "$MTLD3D_TAR") has no wine/ directory"
        exit 1
    fi
    for arch in i386-windows "$PE64_DIR"; do
        srcarch=$arch
        [ "$arch" = i386-windows ] || srcarch=x86_64-windows
        mkdir -p "$d3d9tree/mtld3d/$arch"
        cp "$MTLD3D_SRC/$srcarch/d3d9.dll" "$MTLD3D_SRC/$srcarch/mtld3d.dll" \
           "$d3d9tree/mtld3d/$arch/"
    done
    mkdir -p "$d3d9tree/mtld3d/$UNIX_DIR"
    cp "$MTLD3D_SRC/$UNIX_DIR/mtld3d.so" "$d3d9tree/mtld3d/$UNIX_DIR/"
fi
if in_list wined3d "$D3D9_IMPLS"; then
    for arch in i386-windows "$PE64_DIR"; do
        mkdir -p "$d3d9tree/wined3d/$arch"
        src="$BUILD_DIR/dlls/d3d9/$arch/d3d9.dll"
        if [ ! -f "$src" ]; then
            echo "Error: $src not found (run build-wine.sh first)"
            exit 1
        fi
        cp "$src" "$d3d9tree/wined3d/$arch/"
    done
fi

# ── Direct3D: fake-module markers in the default dirs ───────────────────
# The default <arch>-windows dirs ship NO real Direct3D. compatdb.so always
# loads and prepends exactly one tree per family per process (the arch
# default, or a database override) via prepend_dll_path, so a DXGI stack is
# never mixed and nothing is ever removed. Every module any tree of a family
# supplies is therefore taken out of the default dir (Wine's own copies from
# `make install`, D3DMetal's from the image) and replaced by a fake-module
# marker. wineboot's 11,,* wildcard stamps a system32/syswow64 placeholder
# for each, which is what lets the prepended tree supply the real DLL (a
# builtin will not load without its placeholder). Each marker mirrors the
# exports of the real DLL it stands in for, taken from the first tree in the
# profile's order that has it. Its machine is that of the processes the
# directory serves, not the real DLL's: i386 in i386-windows, AMD64 in the
# 64-bit dir. In aarch64-windows of the arm64 bundle the placeholder's
# machine decides which module the loader accepts, and an x64 (ARM64EC)
# process rejects an ARM64 placeholder with error 193 even when the real
# module, like Wine's own d3d10.dll there, is an ARM64X one it could load.
# x64 programs are what the bundle is for, so every 64-bit marker says AMD64
# (docs/arm64.md, Limitations). An arm64 winebuild has no default PE target,
# hence the explicit -b; for the x86_64 profile it gives the same bytes as
# the -m32/-m64 used before.
time_mark "Step 4: fake-module markers"
echo "  Stamping fake-module markers in the default dirs..."
mark() { # <out-dir> <arch-dir> <real-dll> <name>
    local target=x86_64-windows
    if [ "$2" = i386-windows ]; then target=i686-windows; fi
    "$WINEBUILD" --fake-module -o "$1/$4.dll" -b "$target" --dll "$3"
}
for family in dxgi d3d9; do
    if [ "$family" = dxgi ]; then impls="$DXGI_IMPLS"; else impls="$D3D9_IMPLS"; fi
    for arch in i386-windows "$PE64_DIR"; do
        if [ "$arch" = i386-windows ]; then default="$d3d32"; else default="$d3d64"; fi
        names=$(for impl in $impls; do
                    for f in "$WINE_DIR/lib/wine/$family/$impl/$arch"/*.dll; do
                        if [ -f "$f" ]; then basename "$f" .dll; fi
                    done
                done | sort -u)
        for dll in $names; do
            rm -f "$default/$dll.dll"
            for impl in $impls; do
                real="$WINE_DIR/lib/wine/$family/$impl/$arch/$dll.dll"
                if [ -f "$real" ]; then mark "$default" "$arch" "$real" "$dll"; break; fi
            done
        done
    done
done

# ── Step 4b: Compat database ────────────────────────────────────────────
# compatdb.so is what turns the trees above into a working Direct3D: ntdll
# dlopens <ntdll_dir>/compatdb.so in every process (the slot CrossOver hack
# 24067 provides) and the library prepends one dxgi/ and one d3d9/ tree,
# chosen per process from its built-in rules plus whatever WINE_COMPATDB
# carries. Built here from the compatdb/ crate for this profile's Rust
# target, the one Wine in this bundle.
time_mark "Step 4b: Compat database"
echo "==> Step 4b: Compat database"
build_compatdb

# ── Step 4c: x87sidecar ─────────────────────────────────────────────────
# ntdll starts i386 processes under bin/x87sidecar when it exists and
# compatdb.so does not keep it away from the executable. ROSETTA_X87_PATH
# overrides the path, and an empty value turns the sidecar off. The binary is
# an ad-hoc signed arm64 executable and is copied as is: no install_name_tool,
# no re-signing, and the leak check in Step 5 does not look at bin/. Only for
# a Wine under Rosetta; a native arm64 Wine runs i386 code under FEX.
if [ "$WITH_X87SIDECAR" -eq 1 ]; then
    time_mark "Step 4c: x87sidecar"
    echo "==> Step 4c: x87sidecar"
    mkdir -p "$TMP_DIR/x87sidecar"
    tar xJf "$X87SIDECAR_TAR" -C "$TMP_DIR/x87sidecar"
    if [ ! -f "$TMP_DIR/x87sidecar/x87sidecar" ]; then
        echo "Error: $(basename "$X87SIDECAR_TAR") has no x87sidecar at its top level"
        exit 1
    fi
    echo "    bin/x87sidecar"
    install -m 0755 "$TMP_DIR/x87sidecar/x87sidecar" "$WINE_DIR/bin/x87sidecar"
    echo "    x87sidecar-LICENSE"
    cp "$X87SIDECAR_LICENSE" "$EXT_DIR/x87sidecar-LICENSE"
fi

# ── Step 4d: FEX ────────────────────────────────────────────────────────
# FEX from the trees build-fex.sh built, installed into a scratch prefix
# first because its install also carries headers, tools and data that have no
# place in a Wine tree. Wine loads its x86 emulators as xtajit.dll (i386
# under WoW64) and xtajit64.dll (x86_64 as ARM64EC) unless the registry names
# others, so FEX's DLLs are installed under their own names and as those two,
# replacing the stubs `make install` put there.
if [ "$WITH_FEX" -eq 1 ]; then
    time_mark "Step 4d: FEX"
    echo "==> Step 4d: FEX"
    for t in wow64 arm64ec unixlib; do
        if [ ! -f "$FEX_BUILD_DIR/$t/build.ninja" ]; then
            echo "Error: no FEX build at $FEX_BUILD_DIR/$t (run build-wine.sh --arm64)"
            exit 1
        fi
        cmake --install "$FEX_BUILD_DIR/$t" --prefix "$TMP_DIR/fex" >/dev/null
    done
    fexpe="$TMP_DIR/fex/lib/wine/$PE64_DIR"
    fexunix="$TMP_DIR/fex/lib/wine/$UNIX_DIR"
    for f in "$fexpe/libwow64fex.dll" "$fexpe/libarm64ecfex.dll" \
             "$fexunix/libwow64fex.so" "$fexunix/libarm64ecfex.so"; do
        if [ ! -f "$f" ]; then
            echo "Error: the FEX install has no $(basename "$f")"
            exit 1
        fi
    done
    echo "    $PE64_DIR/libwow64fex.dll, xtajit.dll"
    cp "$fexpe/libwow64fex.dll" "$WINE_DIR/lib/wine/$PE64_DIR/"
    cp "$fexpe/libwow64fex.dll" "$WINE_DIR/lib/wine/$PE64_DIR/xtajit.dll"
    echo "    $PE64_DIR/libarm64ecfex.dll, xtajit64.dll"
    cp "$fexpe/libarm64ecfex.dll" "$WINE_DIR/lib/wine/$PE64_DIR/"
    cp "$fexpe/libarm64ecfex.dll" "$WINE_DIR/lib/wine/$PE64_DIR/xtajit64.dll"
    echo "    $UNIX_DIR/libwow64fex.so, libarm64ecfex.so"
    cp "$fexunix/libwow64fex.so" "$fexunix/libarm64ecfex.so" "$unixdir/"
fi

# ── Step 4e: Split off PE symbols ───────────────────────────────────────
# Wine, DXMT, FEX and the d3d9 tests are built with DWARF debug info, and
# most PE files also carry a COFF symbol table. Together they make up more
# than half the size of the PE modules and only serve a debugger or the
# symbolizing of a crash report, so the bundle ships none of them. Every PE
# file under lib/wine is stripped of both here, except those from the GPTK
# image, which may only be redistributed unmodified (Step 4) and are left
# alone. With --symbols the unstripped copy of each stripped file goes to
# <dest>/wine-symbols (cleared in Step 0), next to the bundle, under the same
# relative path: an address in lib/wine/<arch>-windows/foo.dll is looked up
# in wine-symbols/lib/wine/<arch>-windows/foo.dll. The files are stripped in
# parallel batches (strip_pe), and errors are listed sorted by file once all
# are done. llvm-strip keeps the DOS stub, which holds Wine's builtin
# marker, and handles ARM64X images. It cannot read the symbol table GNU ld
# writes for some i386 DLLs (DXMT's release); there the table and its
# strings, which end the file, are cut off instead (drop_coff_symtab). Step 5
# checks that no PE file kept either.
time_mark "Step 4e: Split off PE symbols"
echo "==> Step 4e: Split off PE symbols"
if in_list gptk "$DXGI_IMPLS" && [ -z "$GPTK_FILES" ]; then
    echo "Error: no record of the files from the GPTK image; nothing would keep them unmodified"
    exit 1
fi
# mtld3d's d3d9.dll as installed, for Step 5's comparison with the release.
if in_list mtld3d "$D3D9_IMPLS"; then
    for arch in i386-windows "$PE64_DIR"; do
        mkdir -p "$TMP_DIR/installed/$arch"
        cp -p "$d3d9tree/mtld3d/$arch/d3d9.dll" "$TMP_DIR/installed/$arch/"
    done
fi
# Every PE file but Apple's (they ship unmodified, Step 4; Step 5 checks
# them), in parallel batches.
STRIP_WORK="$TMP_DIR/strip"
STRIP_ERRORS="$TMP_DIR/strip-errors"
mkdir -p "$STRIP_WORK" "$STRIP_ERRORS"
export WINE_DIR MINGW_DIR SYMBOLS_DIR WITH_SYMBOLS STRIP_WORK STRIP_ERRORS
export -f strip_pe drop_coff_symtab coff_symtab
# pipefail in a subshell, so a failing find or perl stops the step instead
# of handing xargs an empty list.
(
    set -o pipefail
    find "$WINE_DIR/lib/wine" -type f ! -name '*.a' -print0 | pe_scan | cut -f1 \
        | while IFS= read -r f; do
              rel="${f#"$WINE_DIR"/}"
              case "$GPTK_FILES" in *"|$rel"$'\n'*) continue ;; esac
              printf '%s\0' "$rel"
          done \
        | xargs -0 -n 32 -P "$(sysctl -n hw.ncpu)" /bin/bash -c 'strip_pe "$@"' _ > "$TMP_DIR/stripped.list"
)
if [ -n "$(ls "$STRIP_ERRORS")" ]; then
    for e in $(ls "$STRIP_ERRORS" | LC_ALL=C sort); do
        cat "$STRIP_ERRORS/$e"
    done
    exit 1
fi
split=$(wc -l < "$TMP_DIR/stripped.list" | tr -d ' ')
if [ "$WITH_SYMBOLS" -eq 1 ]; then
    echo "    $split PE files stripped, their symbols in $SYMBOLS_DIR"
else
    echo "    $split PE files stripped (--symbols keeps a copy with their symbols)"
fi

# ── Step 4f: Remove inherited quarantine ──────────────────────────────
# Extracting and copying the checksum-verified redistributables can preserve
# download quarantine, which blocks libraries such as winemetal.so at load time.
# Clear only that attribute from the assembled bundle before running it. Keep
# signatures and other metadata intact; -s acts on symlinks themselves so this
# cannot change a target outside the bundle.
time_mark "Step 4f: Remove inherited quarantine"
echo "==> Step 4f: Remove inherited quarantine"
if ! xattr -drs com.apple.quarantine "$WINE_DIR"; then
    echo "Error: failed to remove inherited quarantine from $WINE_DIR" >&2
    exit 1
fi

# ── Step 4g: Loader app ─────────────────────────────────────────────────
# Last of all that touches the loader, so nothing changes the signed bundle
# after it is sealed.
if [ "$WITH_LOADER_APP" -eq 1 ]; then
    time_mark "Step 4g: Loader app"
    echo "==> Step 4g: Loader app"
    package_loader
fi

# ── Step 5: Verify ──────────────────────────────────────────────────────
time_mark "Step 5: library references"
echo "==> Step 5: Verify"

echo "  wine binary: $(file -L "$WINE_DIR/bin/wine" | sed 's|.*/||')"

# Check that no bundled dylib or .so module still loads from a prefix outside
# the bundle, the side-tree unix libs included (symlinks resolve to the real
# modules in the default unix dir, so this also proves the side-tree links
# are intact).
LEAKED=0
check_no_leaks "$EXT_DIR"/*.dylib
check_no_leaks "$unixdir"/*.so \
               "$WINE_DIR"/lib/wine/dxgi/*/"$UNIX_DIR"/*.so \
               "$WINE_DIR"/lib/wine/d3d9/*/"$UNIX_DIR"/*.so
if [ "$WITH_LOADER_APP" -eq 1 ]; then
    check_no_leaks "$unixdir/wine.app/Contents/MacOS/wine"
fi
if [ $LEAKED -ne 0 ]; then
    echo "Error: bundle is not self-contained"
    exit 1
fi
echo "  All binaries clean, no ${LEAK_PREFIXES// / or } references."

# The Direct3D backends and everything else copied in rather than built,
# which nothing the build does would have caught.
time_mark "Step 5: copied-in components"
WANT=("lib/wine/$UNIX_DIR/compatdb.so"
      "lib/wine/$PE64_DIR/dxgi.dll" "lib/wine/i386-windows/dxgi.dll"
      "lib/wine/$PE64_DIR/d3d9.dll" "lib/wine/i386-windows/d3d9.dll")
if in_list gptk "$DXGI_IMPLS"; then
    WANT+=("lib/external/D3DMetal.framework/Versions/A/D3DMetal"
           "lib/external/libd3dshared.dylib"
           "lib/wine/dxgi/gptk/$PE64_DIR/dxgi.dll"
           "lib/wine/dxgi/gptk/$PE64_DIR/d3d12.dll"
           "lib/wine/dxgi/gptk/$UNIX_DIR/d3d11.so"
           "lib/wine/dxgi/gptk/$UNIX_DIR/nvngx.so")
fi
if in_list dxmt "$DXGI_IMPLS"; then
    WANT+=("lib/wine/$UNIX_DIR/winemetal.so"
           "lib/wine/dxgi/dxmt/$PE64_DIR/dxgi.dll"
           "lib/wine/dxgi/dxmt/$PE64_DIR/winemetal.dll"
           "lib/wine/dxgi/dxmt/$UNIX_DIR/winemetal.so"
           "lib/wine/dxgi/dxmt/i386-windows/dxgi.dll"
           "lib/wine/dxgi/dxmt/i386-windows/winemetal.dll")
fi
if in_list wined3d "$DXGI_IMPLS"; then
    WANT+=("lib/wine/dxgi/wined3d/$PE64_DIR/dxgi.dll"
           "lib/wine/dxgi/wined3d/$PE64_DIR/d3d10_1.dll"
           "lib/wine/dxgi/wined3d/i386-windows/dxgi.dll"
           "lib/wine/dxgi/wined3d/i386-windows/d3d10_1.dll")
fi
if in_list mtld3d "$D3D9_IMPLS"; then
    WANT+=("lib/wine/d3d9/mtld3d/$UNIX_DIR/mtld3d.so"
           "lib/wine/d3d9/mtld3d/i386-windows/mtld3d.dll"
           "lib/wine/d3d9/mtld3d/$PE64_DIR/mtld3d.dll")
fi
if in_list wined3d "$D3D9_IMPLS"; then
    WANT+=("lib/wine/d3d9/wined3d/i386-windows/d3d9.dll"
           "lib/wine/d3d9/wined3d/$PE64_DIR/d3d9.dll")
fi
if [ "$WITH_X87SIDECAR" -eq 1 ]; then
    WANT+=("bin/x87sidecar" "lib/external/x87sidecar-LICENSE")
fi
if [ "$RUNTIME_ONLY" -eq 0 ] && [ "$WITH_WINE_SDK" -eq 1 ]; then
    WANT+=("lib/wine/x86_64-windows/libwinecrt0.a" "lib/wine/x86_64-windows/libntdll.a")
fi
if [ "$WITH_FEX" -eq 1 ]; then
    WANT+=("lib/wine/$PE64_DIR/xtajit.dll" "lib/wine/$PE64_DIR/xtajit64.dll"
           "lib/wine/$PE64_DIR/libwow64fex.dll" "lib/wine/$PE64_DIR/libarm64ecfex.dll"
           "lib/wine/$UNIX_DIR/libwow64fex.so" "lib/wine/$UNIX_DIR/libarm64ecfex.so")
fi
if [ "$WITH_LOADER_APP" -eq 1 ]; then
    WANT+=("lib/wine/$UNIX_DIR/wine.app/Contents/MacOS/wine"
           "lib/wine/$UNIX_DIR/wine.app/Contents/Info.plist"
           "lib/wine/$UNIX_DIR/wine.app/Contents/embedded.provisionprofile")
fi
MISSING=0
for want in "${WANT[@]}"; do
    # -e follows symlinks, so this also proves the GPTK .so links resolve.
    if [ ! -e "$WINE_DIR/$want" ]; then
        echo "  ERROR: missing $want"
        MISSING=1
    fi
done
# A d3d9.dll named like mtld3d's could still be Wine's; only a byte comparison
# proves the right copy landed in the tree. Step 4e kept a copy of it as it
# was installed, before stripping.
if in_list mtld3d "$D3D9_IMPLS"; then
    for arch in i386-windows "$PE64_DIR"; do
        srcarch=$arch
        [ "$arch" = i386-windows ] || srcarch=x86_64-windows
        if ! cmp -s "$MTLD3D_SRC/$srcarch/d3d9.dll" "$TMP_DIR/installed/$arch/d3d9.dll"; then
            echo "  ERROR: d3d9/mtld3d/$arch/d3d9.dll is not mtld3d's"
            MISSING=1
        fi
    done
fi
# The emulator Wine loads by default has to be FEX, not the stub it replaced.
if [ "$WITH_FEX" -eq 1 ]; then
    for pair in libwow64fex:xtajit libarm64ecfex:xtajit64; do
        if ! cmp -s "$WINE_DIR/lib/wine/$PE64_DIR/${pair%%:*}.dll" \
                    "$WINE_DIR/lib/wine/$PE64_DIR/${pair##*:}.dll"; then
            echo "  ERROR: $PE64_DIR/${pair##*:}.dll is not FEX's ${pair%%:*}.dll"
            MISSING=1
        fi
    done
fi
if [ $MISSING -ne 0 ]; then
    echo "Error: copied-in components are not installed correctly"
    exit 1
fi
echo "  Direct3D trees (dxgi: $DXGI_IMPLS; d3d9: $D3D9_IMPLS) and the other copied-in components in place."

# Step 4e stripped every PE file's symbols; none may be left, except in
# Apple's files, which ship as they are on the image. Those are checked for
# exactly that, by content (shasum reads the file; cmp maps it, which macOS
# can kill for a signed Mach-O). One header scan (pe_scan) finds the PE files
# and their COFF symbol tables, and llvm-objdump reads their section tables
# in parallel batches (pe_debug_files).
time_mark "Step 5: PE symbol check"
(
    set -o pipefail
    find "$WINE_DIR" -type f ! -name '*.a' -print0 | pe_scan > "$TMP_DIR/pe-scan"
)
# A scan that came back empty, or without a module every bundle has, read
# nothing and would pass for a clean bundle.
if ! cut -f1 "$TMP_DIR/pe-scan" | grep -qxF "$WINE_DIR/lib/wine/$PE64_DIR/ntdll.dll"; then
    echo "  ERROR: the PE scan found $(wc -l < "$TMP_DIR/pe-scan" | tr -d ' ') files, not lib/wine/$PE64_DIR/ntdll.dll"
    exit 1
fi
: > "$TMP_DIR/pe-check"
: > "$TMP_DIR/pe-symboled"
while IFS=$'\t' read -r f ptr count; do
    rel="${f#"$WINE_DIR"/}"
    case "$GPTK_FILES" in *"|$rel"$'\n'*) continue ;; esac
    printf '%s\0' "$f" >> "$TMP_DIR/pe-check"
    if [ "$ptr" != 0 ] || [ "$count" != 0 ]; then
        echo "$rel" >> "$TMP_DIR/pe-symboled"
    fi
done < "$TMP_DIR/pe-scan"
export MINGW_DIR
export -f pe_debug_files
xargs -0 -n 64 -P "$(sysctl -n hw.ncpu)" /bin/bash -c 'pe_debug_files "$@"' _ \
    < "$TMP_DIR/pe-check" > "$TMP_DIR/pe-debug"
unreadable=$(grep '^!' "$TMP_DIR/pe-debug" | LC_ALL=C sort | head -1)
if [ -n "$unreadable" ]; then
    echo "  ERROR: llvm-objdump cannot read ${unreadable#!"$WINE_DIR"/}:"
    "$MINGW_DIR/bin/llvm-objdump" -h "${unreadable#!}" 2>&1 || true
    exit 1
fi
while IFS= read -r f; do
    echo "${f#"$WINE_DIR"/}" >> "$TMP_DIR/pe-symboled"
done < "$TMP_DIR/pe-debug"
if [ -s "$TMP_DIR/pe-symboled" ]; then
    echo "  ERROR: PE files with debug sections or a symbol table:"
    LC_ALL=C sort -u "$TMP_DIR/pe-symboled" | sed 's/^/    /'
    exit 1
fi
echo "  No PE file carries debug sections or a symbol table."
if in_list gptk "$DXGI_IMPLS"; then
    if [ -z "$GPTK_FILES" ]; then
        echo "  ERROR: no record of the files from the GPTK image"
        exit 1
    fi
    time_mark "Step 5: GPTK files against the image"
    gptk_count=0
    while IFS='|' read -r src dst; do
        [ -n "$src" ] || continue
        if [ "$(shasum -a 256 < "$TMP_DIR/gptk-image/$src")" != "$(shasum -a 256 < "$WINE_DIR/$dst")" ]; then
            echo "  ERROR: $dst differs from $src on the GPTK image"
            exit 1
        fi
        gptk_count=$((gptk_count + 1))
    done <<< "$GPTK_FILES"
    echo "  The $gptk_count files from the GPTK image are byte-identical to it."
fi

# A symlink whose target a step removed or never installed fails only at run
# time, or, inside wine.app, as a signature that does not verify.
time_mark "Step 5: dangling symlinks"
DANGLING=$(find "$WINE_DIR" -type l ! -exec test -e {} \; -print)
if [ -n "$DANGLING" ]; then
    echo "  ERROR: dangling symlinks:"
    echo "$DANGLING" | sed "s|^$WINE_DIR/|    |"
    exit 1
fi
echo "  No dangling symlinks."

time_mark "Step 5: exports and loader signature"
check_query_export
if [ "$WITH_X87SIDECAR" -eq 0 ]; then
    check_no_x87
fi
if [ "$WITH_LOADER_APP" -eq 1 ]; then
    check_loader_app
fi

# After everything above is in place, so the copied-in redistributables and
# compatdb.so are checked as well as what Wine built.
time_mark "Step 5: minos"
echo "  Checking every Mach-O against macOS $MACOS_FLOOR..."
check_minos "$WINE_DIR"
# smoke_test checks that bin/x87sidecar is an arm64 executable before it
# starts the i386 process, for this mode and --compatdb-only alike.

"$SMOKE_TEST"

echo ""
print_timings
echo "==> Done! Distribution is at:"
echo "    $DIST_DIR/wine/"
if [ "$RUNTIME_ONLY" -eq 1 ]; then
    echo "    (runtime only, no development files)"
fi
