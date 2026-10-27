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
| `build-wine.sh` | none | Incremental build; configure runs only when the build directory has no Makefile ([commands](internals.md#commands), [Wine SDK](internals.md#wine-sdk)) |
| `build-wine.sh` | `--clean` | Wipes the build directories and what they staged first; the way to pick up changed configure flags or deployment target ([commands](internals.md#commands)) |
| `bundle-wine.sh` | `--dest <dir>` | Writes `<dir>/wine` instead of the bundle in the [layout](#layout). Either way it asks before it deletes an existing one, unless `--yes` |
| `bundle-wine.sh` | `--yes` | Deletes an existing bundle without asking, for an unattended run. A directory that is not empty and has no `bin/wine` is still refused |
| `bundle-wine.sh` | `--runtime-only` | No development files; the flavor to put into an application |
| `bundle-wine.sh` | `--strict-floor` | A Mach-O that needs a newer macOS than the floor is an error instead of a warning ([Homebrew libraries and the floor](internals.md#homebrew-libraries-and-the-floor)) |
| `bundle-wine.sh` | `--symbols` | Also keeps the PE files as they were before their symbols were stripped, in `wine-symbols` next to the bundle |

`usage()` and the argument parsing in
[`build-wine.sh`](../bin/build-wine.sh) and
[`bundle-wine.sh`](../bin/bundle-wine.sh) are authoritative.
`build-mesa.sh`, `build-gstreamer.sh`, `build-fex.sh`, `build-dxmt.sh` and
`build-vkd3d-proton.sh` also run alone
([commands](internals.md#commands)).

## Repository

| Path | What it holds |
|------|---------------|
| [`bin/`](../bin) | The build and bundle scripts; [`bin/lib/profile.sh`](../bin/lib/profile.sh) holds the settings and helpers they share, [`bin/ci/`](../bin/ci) what the workflows run besides them |
| [`pins/`](../pins) | The pinned inputs: [`redist.env`](../pins/redist.env), the refs of the sources the build checks out, and the Homebrew formulae a CI build installs |
| [`.github/workflows/`](../.github/workflows) | The CI workflows ([releases](maintenance.md#releases)) |
| [`compatdb/`](../compatdb) | The Cargo workspace of `compatdb.so` and the `compatdb-core` crate ([compatdb.md](compatdb.md)); run cargo there |
| [`smoke/`](../smoke) | The programs the bundle step builds and runs to check the bundle |
| [`signing/`](../signing) | The loader's provisioning profile |
| [`docs/`](.) | The documentation the README links to |

## Layout

Everything lives beside this repository, in the directory that holds it
(the root; `WINE_BUILD_ROOT` names another, for a checkout kept elsewhere):

| Path | What it is |
|------|------------|
| `scripts/` | The checkout of this repository, [wine-build](https://github.com/athei/wine-build), named `scripts` locally |
| `src/` | The Wine source, a checkout of athei/wine |
| `src-fex/` | The FEX checkout |
| `src-mesa/` | The Mesa checkout, for the Vulkan driver |
| `cache/` | Downloaded redistributables |
| `build/<component>/` | One tree per intermediate: the Wine build, its SDK and the x86 tree for its link libraries and tests, FEX, DXMT, vkd3d-proton, Mesa with the Vulkan loader, and GStreamer. DXMT's, vkd3d-proton's and GStreamer's trees hold their own clones of the source |
| `dist/wine` | The bundle |
| `dist/wine-symbols` | With `--symbols`, the bundle's PE files before their symbols were stripped (Step 4e of [`bundle-wine.sh`](../bin/bundle-wine.sh)); not part of the bundle |

The name of each tree is set in [`profile.sh`](../bin/lib/profile.sh),
with the variable that overrides it ([paths](internals.md#paths)). The
scripts mark each build tree they make with a `.wine-build-tree` file and
refuse to fill or delete a directory that is not empty and has no such
marker, so an override cannot point them at something else (`claim_tree`
in [`profile.sh`](../bin/lib/profile.sh)). The exception is an unmarked
tree from before markers existed, which is adopted when its parent is
`build/` or another tree and it is not a git checkout.

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
| `lib/wine/tests/i386-windows/`, `lib/wine/tests/x86_64-windows/` | Full bundle only: Wine's graphics conformance tests as `<name>_test.exe`, for testing the bundle's Direct3D, OpenGL and Vulkan stack ([graphics tests](internals.md#graphics-tests)) |
| `lib/wine/tests/VERSION` | Full bundle only: the Wine version the tests were built from ([graphics tests](internals.md#graphics-tests)) |
| `lib/external/` | Bundled dylibs, the Vulkan loader, KosmicKrisp, Mesa's EGL with Zink ([OpenGL](internals.md#opengl)), and GStreamer with glib and FFmpeg, which `winegstreamer.so` links ([GStreamer](internals.md#gstreamer)) |
| `lib/external/gstreamer-1.0/` | GStreamer's plugins, gst-libav with FFmpeg's decoders among them ([GStreamer](internals.md#gstreamer)) |
| `share/vulkan/icd.d/` | KosmicKrisp's manifest, naming the driver by a path relative to the manifest |

What the Direct3D trees and the default directories hold, and why, is in
[Direct3D trees](maintenance.md#direct3d-trees).
