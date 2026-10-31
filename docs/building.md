# Building

How to build the bundle locally, where the scripts put things, and what
the bundle holds. How each step works and what the bundle step checks is
in [internals.md](internals.md); what the build needs is in its
[requirements](internals.md#requirements).

## Commands

```bash
bin/build-wine.sh    # Mesa and the Vulkan loader, GStreamer, Wine, the SDK, then FEX, DXMT and vkd3d-proton
bin/bundle-wine.sh   # assemble the bundle (see Bundle layout)
```

| Script | Flag | Effect |
|--------|------|--------|
| `build-wine.sh` | none | Incremental build; configure runs only when the build directory has no Makefile ([commands](internals.md#commands)) |
| `build-wine.sh` | `--clean` | Wipes the build directories and what they staged first; needed for changed configure flags or deployment target |
| `bundle-wine.sh` | `--dest <dir>` | Writes `<dir>/wine` instead of the bundle in the [layout](#layout) |
| `bundle-wine.sh` | `--yes` | Deletes an existing bundle without asking; a non-empty directory without `bin/wine` is still refused |
| `bundle-wine.sh` | `--runtime-only` | No development files; the flavor to put into an application |
| `bundle-wine.sh` | `--strict-floor` | A Mach-O above the floor is an error, not a warning ([Homebrew libraries and the floor](internals.md#homebrew-libraries-and-the-floor)) |
| `bundle-wine.sh` | `--symbols` | Also keeps the unstripped PE files in `wine-symbols` next to the bundle |

`usage()` and the argument parsing in
[`build-wine.sh`](../bin/build-wine.sh) and
[`bundle-wine.sh`](../bin/bundle-wine.sh) are authoritative. The component
scripts also run alone ([commands](internals.md#commands)).

## Repository

| Path | What it holds |
|------|---------------|
| [`bin/`](../bin) | The build and bundle scripts; [`bin/lib/profile.sh`](../bin/lib/profile.sh) holds the settings and helpers they share, [`bin/ci/`](../bin/ci) what the workflows run besides them |
| [`pins/`](../pins) | The pinned inputs: [`redist.env`](../pins/redist.env), the refs of the sources the build checks out, and the Homebrew formulae a CI build installs |
| [`.github/workflows/`](../.github/workflows) | The CI workflows ([releases](maintenance.md#releases)) |
| [`compatdb/`](../compatdb) | The crates of `compatdb.so` and `compatdb-core` ([compatdb.md](compatdb.md)); the Cargo workspace is the repository root, so run cargo there |
| [`launcher/`](../launcher) | The source of [Mach9.app](app.md)'s launcher (`app`), its Dock tile plug-in (`docktile`) and the code both share (`core`); nothing the bundle step builds depends on it. Without `SPARKLE_DIR` naming the directory that holds `Sparkle.framework`, the launcher builds without its updater ([`build.rs`](../launcher/app/build.rs)) |
| [`smoke/`](../smoke) | The programs the bundle step builds and runs to check the bundle |
| [`licenses/`](../licenses) | The licence texts the bundle step installs |
| [`signing/`](../signing) | The loader's provisioning profile |
| [`docs/`](.) | The documentation the README links to |

## Layout

Everything lives beside this repository, in the directory that holds it
(the root; `WINE_BUILD_ROOT` names another, for a checkout kept elsewhere):

| Path | What it is |
|------|------------|
| `scripts/` | The checkout of this repository, [mach9](https://github.com/athei/mach9), named `scripts` locally |
| `src/` | The Wine source, a checkout of athei/wine |
| `src-fex/` | The FEX checkout |
| `src-mesa/` | The Mesa checkout, for the Vulkan driver |
| `cache/` | Downloaded redistributables |
| `build/<component>/` | One tree per component, some with their own source clones ([paths](internals.md#paths)) |
| `dist/wine` | The bundle |
| `dist/wine-symbols` | With `--symbols`, the unstripped PE files; not part of the bundle |

The name of each tree is set in [`profile.sh`](../bin/lib/profile.sh),
with the variable that overrides it ([paths](internals.md#paths)). The
scripts mark each tree they make and refuse to fill or delete a non-empty
directory without the marker, so an override cannot point them at
something else (`claim_tree` in `profile.sh`).

## Configure policy

Every optional dependency is `--without-*` unless re-enabled with
`--with-*` in [`build-wine.sh`](../bin/build-wine.sh), so a missing
package fails configure instead of silently changing the feature set.
The libraries built here that configure finds are described with them:
[OpenGL](internals.md#opengl), [Vulkan](internals.md#vulkan) and
[GStreamer](internals.md#gstreamer).

## Bundle layout

| Path | Contents |
|------|----------|
| `bin/` | What `make install` creates; `bin/wine` is a symlink to the loader in `wine.app` |
| `lib/wine/aarch64-windows/`, `lib/wine/i386-windows/` | Wine's PE modules, FEX's emulator DLLs (also as `xtajit.dll` and `xtajit64.dll`, the names Wine loads), and fake-module markers for the Direct3D DLLs |
| `lib/wine/aarch64-unix/` | Wine's unix modules, `compatdb.so`, DXMT's `winemetal.so` and FEX's unix halves |
| `lib/wine/aarch64-unix/wine.app/` | The loader, its `Info.plist` and the embedded provisioning profile ([loader app](internals.md#loader-app)) |
| `lib/wine/dxgi/<impl>/`, `lib/wine/d3d9/<impl>/` | One tree per Direct3D implementation ([Direct3D backends](compatdb.md#direct3d-backends)) |
| `lib/wine/x86_64-windows/` | Full bundle only: the x86_64 link libraries of the [Wine SDK](internals.md#wine-sdk). Nothing reads the directory at run time |
| `lib/wine/tests/i386-windows/`, `lib/wine/tests/x86_64-windows/` | Full bundle only: Wine's graphics conformance tests as `<name>_test.exe` ([graphics tests](internals.md#graphics-tests)) |
| `lib/wine/tests/VERSION` | Full bundle only: the Wine version the tests were built from ([graphics tests](internals.md#graphics-tests)) |
| `lib/external/` | Bundled dylibs, the Vulkan loader, KosmicKrisp, Mesa's EGL with Zink ([OpenGL](internals.md#opengl)), and GStreamer with glib and FFmpeg, which `winegstreamer.so` links ([GStreamer](internals.md#gstreamer)) |
| `lib/external/gstreamer-1.0/` | GStreamer's plugins, gst-libav with FFmpeg's decoders among them ([GStreamer](internals.md#gstreamer)) |
| `share/vulkan/icd.d/` | KosmicKrisp's manifest, naming the driver by a path relative to the manifest |
| `share/wine/fonts/` | Wine's fonts and the [icon font](internals.md#icon-font) |
| `share/licenses/` | The licences of the [icon font](internals.md#icon-font) |

What the Direct3D trees and the default directories hold, and why, is in
[Direct3D trees](maintenance.md#direct3d-trees).
