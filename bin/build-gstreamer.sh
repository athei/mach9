#!/bin/bash
set -e

# Builds the GStreamer that winegstreamer.so links and stages it in
# GSTREAMER_DIST:
#
#   lib/*.dylib               GStreamer's libraries and those of glib and
#                             FFmpeg under them, loading each other by
#                             @loader_path/<name>, with the links to them
#                             that -l<name> finds; each is named by the path
#                             winegstreamer.so loads it by in the bundle,
#                             @loader_path/../../external/<name>
#   lib/gstreamer-1.0/*.dylib the plugins (GST_PLUGINS below), loading the
#                             libraries by @loader_path/../<name>; GStreamer
#                             looks for them in gstreamer-1.0/ next to
#                             libgstreamer-1.0
#   lib/pkgconfig/*.pc        the install's .pc files, which Wine's
#                             configure finds GStreamer with, each prefix
#                             relative to the file
#   lib/glib-2.0/include/, include/
#                             the headers winegstreamer is compiled with
#   FFMPEG_COMMIT             the commit of GStreamer's FFmpeg port
#   KEY                       what the dist was built from
#                             (dist_key in profile.sh)
#   COMMIT                    GStreamer's source commit
#
# GStreamer comes from its monorepo at the tag redist.env pins, which the
# script clones into GSTREAMER_BUILD_DIR and checks against the pinned
# commit. One meson tree builds GStreamer and its plugins with glib, its
# dependencies and FFmpeg as subprojects from the monorepo's wraps, for the
# floor. Homebrew's GStreamer and FFmpeg are not used: their builds are GPL
# and link a large part of Homebrew. Called by build-wine.sh before Wine's
# configure, which finds GStreamer through the .pc files here; running it
# alone rebuilds GStreamer only. bundle-wine.sh installs from
# GSTREAMER_DIST.
#
# What is built is what Wine's media code uses (dlls/winegstreamer): the
# elements it creates by name, decodebin and the demuxers, parsers and
# decoders decodebin and the transforms find by their caps, for the formats
# games ship (ASF with WMA and WMV, MP4, AVI, Matroska, WAV, MPEG audio and
# video). FFmpeg, through gst-libav, has the decoders, parsers and bitstream
# filters of FFMPEG_DECODERS, FFMPEG_PARSERS and FFMPEG_BSFS below and
# nothing else: no programs, devices, network, encoders, muxers, demuxers or
# hardware decoding. GPL code is off in GStreamer and FFmpeg. Every feature
# option meson would otherwise decide by what it finds is disabled
# (auto_features), and pkg-config searches nowhere, so a library the host
# has installed does not turn a feature on. A plain library check can still
# find one (Apple's clang searches /usr/local); the checks below fail the
# build on a header from there or a dependency outside the system and the
# dist, so such a find cannot reach what is staged.
# No plugin scanner is built: winegstreamer loads plugins in its own process
# (--gst-disable-registry-fork).
#
# Nothing is built when GSTREAMER_DIST is complete and has the KEY this run
# would write (skip_current_dist in profile.sh). --clean builds the meson
# tree and GSTREAMER_DIST from scratch; the clone and the downloaded wraps
# are kept while the pins stay the same. --print-key prints that KEY and
# exits (print_dist_key).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

CLEAN=0
PRINT_KEY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --clean) CLEAN=1; shift ;;
        --print-key) PRINT_KEY=1; shift ;;
        *) echo "Usage: $0 [--clean | --print-key]"; exit 1 ;;
    esac
done

# shellcheck source=lib/profile.sh
. "$SCRIPT_DIR/lib/profile.sh"
# shellcheck source=../pins/redist.env
. "$PINS_DIR/redist.env"

if [ -z "$GSTREAMER_GIT_COMMIT" ] || [ -z "$GSTREAMER_FFMPEG_GIT_COMMIT" ]; then
    echo "Error: redist.env pins no GSTREAMER_GIT_COMMIT or GSTREAMER_FFMPEG_GIT_COMMIT"
    exit 1
fi

GST_SRC="$GSTREAMER_BUILD_DIR/src"
FFMPEG_SRC="$GST_SRC/subprojects/FFmpeg"
GST_BUILD="$GSTREAMER_BUILD_DIR/build"
GST_PREFIX="$GSTREAMER_BUILD_DIR/install"

# The plugins the dist holds, by file name without lib and .dylib, and the
# elements of each that winegstreamer uses:
#   coreelements      capsfilter, and the queues decodebin inserts
#   playback          decodebin
#   typefindfunctions how decodebin tells a stream's container format
#   pbtypes           the GstDiscoverer and encoding profile types
#   audioconvert, audioresample, videoconvertscale (videoconvert)
#   deinterlace, videofilter (videoflip)
#   isomp4            qtdemux, MP4 and QuickTime; also mp4mux, which
#                     winegstreamer's muxer finds by caps
#   avi, matroska, wavparse, id3demux, asf (asfdemux, from gst-plugins-ugly)
#   audioparsers      the MPEG audio, AAC, AC-3 and FLAC parsers
#   videoparsersbad   h264parse, h265parse and the other video parsers
#   libav             gst-libav: an avdec_<name> decoder for each of
#                     FFMPEG_DECODERS
GST_PLUGINS="coreelements playback typefindfunctions pbtypes audioconvert
audioresample videoconvertscale deinterlace videofilter isomp4 avi matroska
wavparse id3demux asf audioparsers videoparsersbad libav"
# The meson options that build exactly those plugins, each a feature option
# of its subproject, which auto_features would disable.
GST_PLUGIN_OPTIONS=(
    -Dgst-plugins-base:playback=enabled
    -Dgst-plugins-base:typefind=enabled
    -Dgst-plugins-base:pbtypes=enabled
    -Dgst-plugins-base:audioconvert=enabled
    -Dgst-plugins-base:audioresample=enabled
    -Dgst-plugins-base:videoconvertscale=enabled
    -Dgst-plugins-good:deinterlace=enabled
    -Dgst-plugins-good:videofilter=enabled
    -Dgst-plugins-good:isomp4=enabled
    -Dgst-plugins-good:avi=enabled
    -Dgst-plugins-good:matroska=enabled
    -Dgst-plugins-good:wavparse=enabled
    -Dgst-plugins-good:id3demux=enabled
    -Dgst-plugins-good:audioparsers=enabled
    -Dgst-plugins-ugly:asfdemux=enabled
    -Dgst-plugins-bad:videoparsers=enabled
)
# FFmpeg's decoders, parsers and bitstream filters, by the names of its
# meson options without _decoder, _parser and _bsf. FFmpeg's port selects the
# internal code a decoder needs, but not a parser, filter or other decoder
# it builds on, which auto_features has disabled: those are listed here too
# (h263, which the MPEG-4 part 2 and MS-MPEG4 decoders build on, needs its
# parser, vp9 the superframe filter), and the opus decoder needs
# libswresample, which is built for it.
FFMPEG_DECODERS="wmav1 wmav2 wmapro wmavoice wmalossless
wmv1 wmv2 wmv3 vc1 msmpeg4v1 msmpeg4v2 msmpeg4v3 mpeg4 h263 h264 hevc
mpeg1video mpeg2video vp8 vp9 indeo2 indeo3 indeo4 indeo5 cinepak
aac mp1 mp2 mp3 ac3 flac vorbis opus adpcm_ms adpcm_ima_wav
pcm_alaw pcm_mulaw pcm_s8 pcm_u8 pcm_s16le pcm_s16be pcm_u16le pcm_u16be
pcm_s24le pcm_s24be pcm_u24le pcm_u24be pcm_s32le pcm_s32be pcm_u32le
pcm_u32be pcm_f32le pcm_f32be pcm_f64le pcm_f64be"
FFMPEG_PARSERS="h263 h264 hevc vc1 aac mpegaudio mpeg4video mpegvideo ac3 vp8 vp9"
FFMPEG_BSFS="vp9_superframe_split"

# ── Preflight ───────────────────────────────────────────────────────────
for tool in git meson ninja pkg-config; do
    if ! command -v "$tool" >/dev/null; then
        case $tool in
            pkg-config) echo "Error: pkg-config not found (brew install pkgconf)" ;;
            *) echo "Error: $tool not found (brew install $tool)" ;;
        esac
        exit 1
    fi
done
# By path, since the build runs them with Homebrew off PATH.
MESON=$(command -v meson)
NINJA=$(command -v ninja)
check_sdk_floor

# Only Apple's compilers, named in the native file, and none of the caller's
# flags or pkg-config paths.
unset CC CXX OBJC OBJCXX CFLAGS CXXFLAGS OBJCFLAGS CPPFLAGS LDFLAGS \
      PKG_CONFIG_PATH PKG_CONFIG_LIBDIR

# ── Dist ────────────────────────────────────────────────────────────────
# gstreamer_key <commit>: the KEY of a GSTREAMER_DIST built from <commit>.
# Besides the commit: the FFmpeg commit and the floor, and the tools that
# build all of it (meson, the Python it runs on, which also runs glib's and
# GStreamer's build scripts, ninja, Apple's clang and the macOS SDK). The
# wraps of glib and its dependencies are fixed by the GStreamer commit.
# meson's Python is the interpreter its script names.
MESON_VERSION=$("$MESON" --version)
meson_python=$(head -n 1 "$(realpath "$MESON")" | sed -n 's|^#! *||p')
# shellcheck disable=SC2086 # the interpreter line may hold arguments
MESON_PYTHON=$(${meson_python:-false} -c 'import platform; print(platform.python_version())' 2>/dev/null) \
    || MESON_PYTHON=unknown
GST_KEY_INPUTS="ffmpeg=$GSTREAMER_FFMPEG_GIT_COMMIT floor=$MACOS_FLOOR"
GST_KEY_INPUTS+=" meson=$MESON_VERSION python=$MESON_PYTHON ninja=$("$NINJA" --version)"
GST_KEY_INPUTS+=" cc=$(cc_version /usr/bin/clang) $(sdk_key)"
gstreamer_key() {
    dist_key "$SCRIPT_DIR/build-gstreamer.sh" "gstreamer=$1" "$GST_KEY_INPUTS"
}
print_dist_key "$GSTREAMER_GIT_COMMIT" redist.env gstreamer_key

# The trees this script deletes and recreates are fixed paths in
# GSTREAMER_BUILD_DIR. GSTREAMER_DIST is inside it unless overridden, so it
# is claimed on its own, and so is GSTREAMER_DIST.part below.
claim_tree "$GSTREAMER_BUILD_DIR" gstreamer
if [ "$CLEAN" -eq 1 ]; then
    echo "==> Cleaning GStreamer's meson tree and $GSTREAMER_DIST..."
    remove_tree "$GST_BUILD" "$GST_PREFIX"
fi
claim_tree "$GSTREAMER_DIST" gstreamer-dist "$CLEAN"
skip_current_dist "$GSTREAMER_DIST" GStreamer "$GSTREAMER_GIT_COMMIT" gstreamer_key

# ── Source ──────────────────────────────────────────────────────────────
# The monorepo at the pinned tag, without its one submodule (media files of
# the integration tests, in Git LFS), and in its subprojects/FFmpeg, where
# meson's FFmpeg wrap would clone the head of a branch, the pinned commit of
# that branch, fetched by hash. meson uses a subproject directory that
# exists as it is, and downloads the other wraps into subprojects/ on the
# first setup, checking each tarball against the checksum its wrap names.
echo "==> GStreamer source: $GSTREAMER_GIT_URL $GSTREAMER_GIT_TAG"
pinned_clone "$GSTREAMER_GIT_URL" "$GSTREAMER_GIT_TAG" "$GSTREAMER_GIT_COMMIT" "$GST_SRC" --no-submodules
if [ "$(git -C "$FFMPEG_SRC" rev-parse HEAD 2>/dev/null)" != "$GSTREAMER_FFMPEG_GIT_COMMIT" ]; then
    echo "==> FFmpeg: $GSTREAMER_FFMPEG_GIT_URL $GSTREAMER_FFMPEG_GIT_BRANCH at $GSTREAMER_FFMPEG_GIT_COMMIT"
    remove_tree "$FFMPEG_SRC"
    git init --quiet "$FFMPEG_SRC"
    GIT_TERMINAL_PROMPT=0 git -C "$FFMPEG_SRC" fetch --quiet --depth 1 \
        "$GSTREAMER_FFMPEG_GIT_URL" "$GSTREAMER_FFMPEG_GIT_COMMIT"
    git -C "$FFMPEG_SRC" -c advice.detachedHead=false checkout --quiet --detach FETCH_HEAD
fi
if ! grep -qx "revision = $GSTREAMER_FFMPEG_GIT_BRANCH" "$GST_SRC/subprojects/FFmpeg.wrap"; then
    echo "Error: $GST_SRC/subprojects/FFmpeg.wrap does not name the branch $GSTREAMER_FFMPEG_GIT_BRANCH;"
    echo "pin the commit of the branch it names in redist.env"
    exit 1
fi

# ── Build ───────────────────────────────────────────────────────────────
# in_list <word> <list>: succeed when <word> is one of the space-separated
# words in <list>.
in_list() {
    case " $2 " in
        *" $1 "*) return 0 ;;
    esac
    return 1
}

# write_if_changed <file>: write stdin to <file>, but leave the file alone
# when it already holds that. meson regenerates a tree whenever its native
# file is newer, even with the same content.
write_if_changed() {
    cat > "$1.new"
    if cmp -s "$1.new" "$1"; then
        rm -f "$1.new"
    else
        mv "$1.new" "$1"
    fi
}

# For the floor, which profile.sh exports as MACOSX_DEPLOYMENT_TARGET and
# the native file passes on as well. The compilers go through ccache with
# WINE_BUILD_CCACHE (profile.sh), named by its path, as in build-mesa.sh.
# cmake is named false, so meson's CMake lookup has no CMake to search with,
# and meson and ninja run with the system directories only on PATH. FFmpeg's
# assembly is the aarch64 kind, which clang assembles, so no nasm is needed.
#
# The SDK can declare functions that macOS only has above the floor (pipe2,
# FLOOR_NEWER_FUNCS in profile.sh). A configure check that finds one makes
# the code call it without an availability check, weak-imported, and on the
# floor's macOS the call jumps to address 0; glib does so with pipe2.
# meson's function checks declare the function themselves, without its
# availability, and cannot be told the result, so the HAVE_<FUNCTION>
# define each such check wrote into a config.h is turned off after setup
# and before every build (unconfig_floor_newer below), as build-wine.sh
# re-patches Wine's config.h, and the code takes its path for a macOS
# without the function, as Wine's configure is told the function is
# missing. -Wunguarded-availability-new as an error fails the build on any
# call of a function newer than the floor that is left, and check_macho
# below fails on an import of FLOOR_NEWER_FUNCS.
if [ -n "$WINE_CCACHE" ]; then
    NATIVE_CC="['$WINE_CCACHE', '/usr/bin/clang']"
    NATIVE_CXX="['$WINE_CCACHE', '/usr/bin/clang++']"
else
    NATIVE_CC="'/usr/bin/clang'"
    NATIVE_CXX="'/usr/bin/clang++'"
fi
min="-mmacosx-version-min=$MACOS_FLOOR"
cflags="'$min', '-Werror=unguarded-availability-new'"
write_if_changed "$GSTREAMER_BUILD_DIR/native.ini" <<EOF
[binaries]
c = $NATIVE_CC
cpp = $NATIVE_CXX
objc = $NATIVE_CC
objcpp = $NATIVE_CXX
cmake = 'false'

[built-in options]
c_args = [$cflags]
cpp_args = [$cflags]
objc_args = [$cflags]
objcpp_args = [$cflags]
c_link_args = ['$min']
cpp_link_args = ['$min']
objc_link_args = ['$min']
objcpp_link_args = ['$min']
EOF

# The options of the whole build. auto_features turns off every feature
# option left at auto, in every subproject: plugins, optional libraries,
# orc (whose generated code GStreamer then runs from the C fallbacks the
# sources carry), tests, examples, documentation, introspection, the
# bindings and the other modules of the monorepo. The ones that default to
# something else are set here. build-tools-source=system keeps the monorepo
# from downloading a bison binary; with gst_parse off, GStreamer needs no
# bison or flex (winegstreamer builds its pipelines by hand). GStreamer's
# unit test library (check) is built because the plugin modules' docs
# directories read a variable that GStreamer only sets with it, documentation
# off or not; it is not staged.
#
# Apple's clang searches /usr/local/include and /usr/local/lib whatever it
# is told, where an x86_64 Homebrew may be installed. ld skips its libraries
# for the wrong architecture, but a header found there passes a check all
# the same: glib would take /usr/local's libintl.h for a gettext and then
# fail to link it. Its libintl is forced to be the proxy-libintl wrap, and
# after the build no object may have read a header from /usr/local or the
# Homebrew prefix (the dependency check below). proxy-libintl is linked
# statically, into glib and GStreamer: as a library of its own it would be
# libintl.8.dylib, the name of Homebrew's gettext, which lib/external carries
# for GnuTLS.
GST_OPTIONS=(
    --buildtype=release
    --default-library=shared
    --force-fallback-for=intl
    -Dproxy-libintl:default_library=static
    -Dauto_features=disabled
    -Dbase=enabled -Dgood=enabled -Dugly=enabled -Dbad=enabled -Dlibav=enabled
    -Dgpl=disabled -Dnls=disabled -Dtools=disabled -Dtests=disabled
    -Dexamples=disabled -Ddoc=disabled -Dintrospection=disabled -Dorc=disabled
    -Dpython=disabled -Dges=disabled -Drtsp_server=disabled -Ddevtools=disabled
    -Dlibnice=disabled -Dtls=disabled -Dgst-examples=disabled
    -Dbuild-tools-source=system
    -Dgstreamer:gst_parse=false
    -Dgstreamer:check=enabled
    -Dglib:tests=false
    "${GST_PLUGIN_OPTIONS[@]}"
    -DFFmpeg:gpl=disabled -DFFmpeg:version3=disabled -DFFmpeg:nonfree=disabled
    -DFFmpeg:programs=disabled -DFFmpeg:network=disabled -DFFmpeg:devices=disabled
    -DFFmpeg:avdevice=disabled -DFFmpeg:postproc=disabled
    -DFFmpeg:swscale=disabled -DFFmpeg:swresample=enabled
    -DFFmpeg:hwaccels=disabled -DFFmpeg:pthreads=enabled
)
for d in $FFMPEG_DECODERS; do
    GST_OPTIONS+=("-DFFmpeg:${d}_decoder=enabled")
done
for p in $FFMPEG_PARSERS; do
    GST_OPTIONS+=("-DFFmpeg:${p}_parser=enabled")
done
for b in $FFMPEG_BSFS; do
    GST_OPTIONS+=("-DFFmpeg:${b}_bsf=enabled")
done

# unconfig_floor_newer: in every config.h of the meson tree, turn off the
# HAVE_<FUNCTION> define of each function of FLOOR_NEWER_FUNCS newer than the
# floor. A config.h that already has it off is left alone, so ninja only
# rebuilds after a setup wrote the define anew.
unconfig_floor_newer() {
    local func def h
    for func in $FLOOR_NEWER_FUNCS; do
        macos_newer "${func#*:}" "$MACOS_FLOOR" || continue
        def="HAVE_$(echo "${func%%:*}" | tr '[:lower:]' '[:upper:]')"
        while IFS= read -r h; do
            if grep -qx "#define $def 1" "$h"; then
                echo "    ${h#"$GST_BUILD"/}: $def off, ${func%%:*} is newer than the floor"
                sed -i '' "s|^#define $def 1\$|/* #undef $def: newer than the floor (build-gstreamer.sh) */|" "$h"
            fi
        done < <(find "$GST_BUILD" -name config.h -not -path '*/meson-private/*')
    done
}

# The tree is configured again from scratch whenever its native file, its
# options, a source commit or meson's version or Python differ from what it
# was configured with (the key in .setup-key), as build-mesa.sh does; a new
# meson regenerates the tree on its own, and the config.h files with it,
# which a fresh setup followed by unconfig_floor_newer covers.
echo "==> Building GStreamer $GSTREAMER_GIT_TAG..."
(
    export PKG_CONFIG_LIBDIR=
    # meson finds ninja through NINJA when it is not on PATH.
    export NINJA
    export PATH="/usr/bin:/bin:/usr/sbin:/sbin"
    key="$(cat "$GSTREAMER_BUILD_DIR/native.ini") ${GST_OPTIONS[*]} gstreamer=$GSTREAMER_GIT_COMMIT"
    key+=" ffmpeg=$GSTREAMER_FFMPEG_GIT_COMMIT meson=$MESON_VERSION python=$MESON_PYTHON"
    if [ "$(cat "$GST_BUILD/.setup-key" 2>/dev/null)" != "$key" ]; then
        remove_tree "$GST_BUILD"
        "$MESON" setup "$GST_BUILD" "$GST_SRC" --native-file "$GSTREAMER_BUILD_DIR/native.ini" \
            --prefix="$GST_PREFIX" --libdir=lib "${GST_OPTIONS[@]}" > "$GSTREAMER_BUILD_DIR/setup.log" || {
            echo "Error: meson setup of $GST_BUILD failed; see $GSTREAMER_BUILD_DIR/setup.log"
            echo "and $GST_BUILD/meson-logs/meson-log.txt"
            exit 1
        }
        echo "$key" > "$GST_BUILD/.setup-key"
    fi
    unconfig_floor_newer
    "$NINJA" -C "$GST_BUILD"
    remove_tree "$GST_PREFIX"
    "$NINJA" -C "$GST_BUILD" install >/dev/null
)

# The headers every object was compiled with, as ninja recorded them from the
# compiler's dependency output: none may come from a prefix outside the
# system, the SDK and the tree.
leaked=$("$NINJA" -C "$GST_BUILD" -t deps | awk '/^ / { print $1 }' | grep -E '^(/usr/local/|/opt/homebrew/)' | sort -u || true)
if [ -n "$leaked" ]; then
    echo "Error: GStreamer was compiled with headers from outside the system and the tree:"
    echo "$leaked" | sed 's/^/    /'
    exit 1
fi

# Every decoder and parser asked for is in the FFmpeg that was built: meson
# accepts an option a subproject has, whatever FFmpeg's port then makes of it.
components="$GST_BUILD/subprojects/FFmpeg/config_components.h"
missing=""
for d in $FFMPEG_DECODERS; do
    grep -qx "#define CONFIG_$(echo "$d" | tr '[:lower:]' '[:upper:]')_DECODER 1" "$components" \
        || missing="$missing ${d}_decoder"
done
for p in $FFMPEG_PARSERS; do
    grep -qx "#define CONFIG_$(echo "$p" | tr '[:lower:]' '[:upper:]')_PARSER 1" "$components" \
        || missing="$missing ${p}_parser"
done
for b in $FFMPEG_BSFS; do
    grep -qx "#define CONFIG_$(echo "$b" | tr '[:lower:]' '[:upper:]')_BSF 1" "$components" \
        || missing="$missing ${b}_bsf"
done
if [ -n "$missing" ]; then
    echo "Error: FFmpeg was built without:$missing (see $components)"
    exit 1
fi
# The licence FFmpeg's build derived from what it enabled: anything GPL,
# version 3 or nonfree changes it.
FFMPEG_LICENSE='#define FFMPEG_LICENSE "LGPL version 2.1 or later"'
if ! grep -qxF "$FFMPEG_LICENSE" "$GST_BUILD/subprojects/FFmpeg/config.h"; then
    echo "Error: FFmpeg's config.h does not say '$FFMPEG_LICENSE':"
    grep "FFMPEG_LICENSE" "$GST_BUILD/subprojects/FFmpeg/config.h" || echo "    (no FFMPEG_LICENSE)"
    exit 1
fi

# ── Stage ───────────────────────────────────────────────────────────────
# Into GSTREAMER_DIST.part, renamed into place once everything is checked,
# so an interrupted run never leaves a GSTREAMER_DIST behind that looks
# complete.
echo "==> Staging GStreamer into $GSTREAMER_DIST..."
part="$GSTREAMER_DIST.part"
claim_tree "$part" gstreamer-dist 1
mkdir -p "$part/lib/gstreamer-1.0" "$part/lib/pkgconfig" "$part/lib/glib-2.0"

# The packages Wine's configure asks for (configure.ac, GSTREAMER).
GST_PACKAGES="gstreamer-1.0 gstreamer-video-1.0 gstreamer-audio-1.0 gstreamer-tag-1.0"

# The plugins: exactly GST_PLUGINS, so a plugin that something else turned
# on fails here instead of reaching the bundle.
found=$(cd "$GST_PREFIX/lib/gstreamer-1.0" && ls -- *.dylib | sed 's/^libgst//; s/\.dylib$//' | sort)
want=$(tr -s ' \n' '\n\n' <<< "$GST_PLUGINS" | sort)
if [ "$found" != "$want" ]; then
    echo "Error: the install's plugins are not GST_PLUGINS:"
    diff <(echo "$want") <(echo "$found") | sed -n 's/^< /    missing: /p; s/^> /    not asked for: /p'
    exit 1
fi
PLUGINS=()
for p in $GST_PLUGINS; do
    cp "$GST_PREFIX/lib/gstreamer-1.0/libgst$p.dylib" "$part/lib/gstreamer-1.0/"
    PLUGINS+=("libgst$p.dylib")
done

# The libraries: what the packages link (-l<name>, the install's
# lib<name>.dylib link resolved to its file) and the plugins load, and what
# those load in turn, each as one file under the versioned name the others
# load it by, with the install's links to it. What nothing loads, such as
# GStreamer's unit test library, stays out.
# shellcheck disable=SC2086 # one word per package
names=$(PKG_CONFIG_LIBDIR="$GST_PREFIX/lib/pkgconfig" pkg-config --libs-only-l $GST_PACKAGES) || {
    echo "Error: pkg-config cannot resolve $GST_PACKAGES from $GST_PREFIX/lib/pkgconfig"
    exit 1
}
WALK=()
for l in $names; do
    f="$GST_PREFIX/lib/lib${l#-l}.dylib"
    # A static library (proxy-libintl's) is inside the libraries already.
    if [ ! -e "$f" ] && [ -f "$GST_PREFIX/lib/lib${l#-l}.a" ]; then
        continue
    elif [ ! -e "$f" ]; then
        echo "Error: $GST_PACKAGES link $l, but the install has no $(basename "$f")"
        exit 1
    fi
    WALK+=("$GST_PREFIX/lib/$(basename "$(realpath "$f")")")
done
for p in "${PLUGINS[@]}"; do
    WALK+=("$GST_PREFIX/lib/gstreamer-1.0/$p")
done
LIBS=()
i=0
while [ $i -lt ${#WALK[@]} ]; do
    src="${WALK[$i]}"
    i=$((i + 1))
    if [ "$(dirname "$src")" = "$GST_PREFIX/lib" ] && ! in_list "$(basename "$src")" "${LIBS[*]}"; then
        LIBS+=("$(basename "$src")")
    fi
    for dep in $(otool -L "$src" | tail -n +2 | awk '{print $1}'); do
        case "$dep" in
            /usr/lib/*|/System/Library/*) continue ;;
        esac
        base=$(basename "$dep")
        [ "$base" = "$(basename "$src")" ] && continue
        in_list "$base" "${LIBS[*]}" && continue
        if [ ! -f "$GST_PREFIX/lib/$base" ] || [ -L "$GST_PREFIX/lib/$base" ]; then
            echo "Error: $(basename "$src") loads $dep, which is not a library the install has"
            exit 1
        fi
        WALK+=("$GST_PREFIX/lib/$base")
    done
done
for lib in "${LIBS[@]}"; do
    cp "$GST_PREFIX/lib/$lib" "$part/lib/"
done
for f in "$GST_PREFIX"/lib/*.dylib; do
    [ -L "$f" ] && [ -f "$part/lib/$(readlink "$f")" ] || continue
    ln -s "$(readlink "$f")" "$part/lib/$(basename "$f")"
done
chmod u+w "$part"/lib/*.dylib "$part"/lib/gstreamer-1.0/*.dylib

# The install names meson wrote are @rpath/<name> or paths in the install
# prefix, and the libraries carry no LC_RPATH that would find the bundle's
# copies. They become @loader_path names: a library's references to the
# others in lib/ as @loader_path/<name>, a plugin's as ../<name>. A
# library's own install name, which only what links against the dist
# records, is the path a Wine unix module in lib/wine/aarch64-unix loads it
# by, @loader_path/../../external/<name> (UNIX_EXTERNAL), so winegstreamer.so
# gets its final references at link time: Wine links its modules without
# room in their headers for longer names, so they cannot be rewritten
# afterwards. Any LC_RPATH is removed. Since that invalidates the linker's
# signature, which arm64 code needs to load, each is signed again ad hoc, as
# the bundle step does.
UNIX_EXTERNAL=@loader_path/../../external/
# relink <file> <prefix>: rewrite the references of <file> to the libraries
# of lib/ as <prefix><name>; exit on any other one outside the system.
relink() {
    local f="$1" prefix="$2" dep base rpath
    for dep in $(otool -L "$f" | tail -n +2 | awk '{print $1}'); do
        case "$dep" in
            /usr/lib/*|/System/Library/*) continue ;;
        esac
        base=$(basename "$dep")
        [ "$base" = "$(basename "$f")" ] && continue
        if [ ! -f "$part/lib/$base" ] || [ -L "$part/lib/$base" ]; then
            echo "Error: $(basename "$f") loads $dep, which is not a library the install has"
            exit 1
        fi
        install_name_tool -change "$dep" "$prefix$base" "$f" 2>/dev/null
    done
    for rpath in $(otool -l "$f" | awk '$1 == "cmd" { r = ($2 == "LC_RPATH") } r && $1 == "path" { print $2 }'); do
        install_name_tool -delete_rpath "$rpath" "$f" 2>/dev/null
    done
}
for lib in "${LIBS[@]}"; do
    install_name_tool -id "$UNIX_EXTERNAL$lib" "$part/lib/$lib" 2>/dev/null
    relink "$part/lib/$lib" @loader_path/
    codesign --force --sign - "$part/lib/$lib" 2>/dev/null
done
for plugin in "${PLUGINS[@]}"; do
    if [ -n "$(otool -D "$part/lib/gstreamer-1.0/$plugin" | tail -n +2)" ]; then
        install_name_tool -id "@loader_path/$plugin" "$part/lib/gstreamer-1.0/$plugin" 2>/dev/null
    fi
    relink "$part/lib/gstreamer-1.0/$plugin" @loader_path/../
    codesign --force --sign - "$part/lib/gstreamer-1.0/$plugin" 2>/dev/null
done

# The headers and .pc files, for Wine's configure and the compile of
# winegstreamer. glib keeps glibconfig.h under lib/. Each .pc file's prefix
# is made relative to the file, so GSTREAMER_DIST is usable wherever it is,
# and none may name the install prefix any more. Two link flags are dropped:
# the LC_RPATH GStreamer's own .pc files give whatever links them, which
# would name GSTREAMER_DIST in winegstreamer.so, and -lintl, which names the
# static proxy-libintl already inside glib and GStreamer and would otherwise
# find Homebrew's libintl.
cp -R "$GST_PREFIX/include" "$part/"
cp -R "$GST_PREFIX/lib/glib-2.0/include" "$part/lib/glib-2.0/"
for pc in "$GST_PREFIX"/lib/pkgconfig/*.pc; do
    # shellcheck disable=SC2016 # ${pcfiledir} is pkg-config's variable
    sed -E -e 's|^prefix=.*|prefix=${pcfiledir}/../..|' \
        -e '/^Libs/s| -Wl,-rpath,[^ ]*||g' -e '/^Libs/s/ -lintl( |$)/\1/g' \
        "$pc" > "$part/lib/pkgconfig/$(basename "$pc")"
done
if grep -lF "$GST_PREFIX" "$part"/lib/pkgconfig/*.pc; then
    echo "Error: the .pc files above still name $GST_PREFIX"
    exit 1
fi
# shellcheck disable=SC2086 # one word per package
if ! pc_libs=$(PKG_CONFIG_LIBDIR="$part/lib/pkgconfig" pkg-config --cflags --libs $GST_PACKAGES); then
    echo "Error: pkg-config cannot resolve $GST_PACKAGES from the staged .pc files"
    exit 1
fi
for want in -lgstreamer-1.0 -lgstvideo-1.0 -lgstaudio-1.0 -lgsttag-1.0 -lglib-2.0 -lgobject-2.0 \
    "-I$part/lib/pkgconfig/../../include/gstreamer-1.0" "-I$part/lib/pkgconfig/../../lib/glib-2.0/include"; do
    if ! tr ' ' '\n' <<< "$pc_libs" | grep -qxF -- "$want"; then
        echo "Error: pkg-config gives no $want for $GST_PACKAGES from the staged .pc files:"
        echo "    $pc_libs"
        exit 1
    fi
done
for l in $(tr ' ' '\n' <<< "$pc_libs" | grep '^-l'); do
    if [ ! -e "$part/lib/lib${l#-l}.dylib" ]; then
        echo "Error: pkg-config gives $l for $GST_PACKAGES, but lib${l#-l}.dylib is not staged"
        exit 1
    fi
done
if tr ' ' '\n' <<< "$pc_libs" | grep -q -- '-rpath'; then
    echo "Error: pkg-config gives an rpath for $GST_PACKAGES: $pc_libs"
    exit 1
fi
echo "    pkg-config resolves $GST_PACKAGES inside the dist"

# check_macho <file> <id-prefix> <prefix>: exit unless <file> is an
# arm64-only Mach-O held to the floor, named <id-prefix><its name> when it
# has a name at all, loading only system libraries and libraries of lib/ by
# <prefix><name>, without LC_RPATH, and binding every import to a library
# (a flat-namespace import is looked up in whatever the process has loaded,
# and fails at run time where nothing provides it), and importing no
# function of FLOOR_NEWER_FUNCS newer than the floor. As build-mesa.sh's
# check_dylib, with the dist's own libraries as the allowed siblings.
check_macho() {
    local f="$1" id_prefix="$2" prefix="$3" name minos id deps dep flat rpaths func
    name=$(basename "$f")
    if [ "$(lipo -archs "$f")" != arm64 ]; then
        echo "Error: $name is not an arm64-only Mach-O ($(lipo -archs "$f"))"
        exit 1
    fi
    minos=$(otool -l "$f" | awk '$1 == "cmd" && $2 == "LC_BUILD_VERSION" { v = 1 } v && $1 == "minos" { print $2; exit }')
    if [ -z "$minos" ] || macos_newer "$minos" "$MACOS_FLOOR"; then
        echo "Error: $name has minos '${minos:-none}', the floor is $MACOS_FLOOR"
        exit 1
    fi
    id=$(otool -D "$f" | tail -n +2)
    if [ -n "$id" ] && [ "$id" != "$id_prefix$name" ]; then
        echo "Error: $name has the install name $id"
        exit 1
    fi
    deps=$(otool -L "$f" | tail -n +2 | awk '{print $1}' | grep -vxF -- "$id" \
        | grep -vE '^(/usr/lib/|/System/Library/)' || true)
    for dep in $deps; do
        if [ "${dep#"$prefix"}" = "$dep" ] || [ ! -f "$part/lib/${dep#"$prefix"}" ]; then
            echo "Error: $name loads $dep, which is neither a system library nor ${prefix}<a library of lib/>"
            exit 1
        fi
    done
    rpaths=$(otool -l "$f" | awk '$1 == "cmd" { r = ($2 == "LC_RPATH") } r && $1 == "path" { print $2 }')
    if [ -n "$rpaths" ]; then
        echo "Error: $name has the LC_RPATH entries:"
        echo "$rpaths" | sed 's/^/    /'
        exit 1
    fi
    flat=$(nm -m -u "$f" | grep 'dynamically looked up' | grep -v 'weak external' || true)
    if [ -n "$flat" ]; then
        echo "Error: $name has flat-namespace imports:"
        echo "$flat" | sed 's/^/    /'
        exit 1
    fi
    for func in $FLOOR_NEWER_FUNCS; do
        macos_newer "${func#*:}" "$MACOS_FLOOR" || continue
        if nm -u "$f" | grep -qx "_${func%%:*}"; then
            echo "Error: $name imports ${func%%:*}, which macOS only has from ${func#*:} on (the floor is $MACOS_FLOOR)"
            exit 1
        fi
    done
}
for lib in "${LIBS[@]}"; do
    check_macho "$part/lib/$lib" "$UNIX_EXTERNAL" @loader_path/
done
for plugin in "${PLUGINS[@]}"; do
    check_macho "$part/lib/gstreamer-1.0/$plugin" @loader_path/ @loader_path/../
done
echo "    ${#LIBS[@]} libraries and ${#PLUGINS[@]} plugins: arm64, minos at most $MACOS_FLOOR,"
echo "    system libraries and each other by @loader_path only"

echo "$GSTREAMER_FFMPEG_GIT_COMMIT" > "$part/FFMPEG_COMMIT"
gstreamer_key "$GSTREAMER_GIT_COMMIT" > "$part/KEY"
# Written last, so a staged directory without it is an incomplete one.
echo "$GSTREAMER_GIT_COMMIT" > "$part/COMMIT"
remove_tree "$GSTREAMER_DIST"
mv "$part" "$GSTREAMER_DIST"
echo "==> GStreamer $GSTREAMER_GIT_TAG ($GSTREAMER_GIT_COMMIT) staged in $GSTREAMER_DIST."
