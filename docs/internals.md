# How the build works

`build-wine.sh` and `bundle-wine.sh` build and bundle a native arm64 Wine
that runs x86 programs through [FEX](https://github.com/FEX-Emu/FEX). The
settings they share are in [`profile.sh`](../bin/lib/profile.sh), and each
script's header says what it builds and stages.

- Wine is upstream Wine with our series on top, the branch
  [`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) of
  athei/wine, some of it from Hadron. It is configured against Homebrew
  with the allowlist in [`build-wine.sh`](../bin/build-wine.sh), plus the
  floor overrides ([macOS floors](maintenance.md#macos-floors)).
- `WINE_ARCHS` pairs aarch64 with arm64ec, so x64 code runs as ARM64EC,
  and adds i386 for WoW64.
- FEX is our fork ([FEX](#fex)). Built-in compatdb rules keep no-execute
  on in 32-bit processes
  ([`builtin::force_nx`](https://docs.mach9.app/compatdb_core/builtin/fn.force_nx.html))
  and turn on FEX's reduced x87 precision
  ([`builtin::fex_reduced_x87_precision`](https://docs.mach9.app/compatdb_core/builtin/fn.fex_reduced_x87_precision.html)).
- Direct3D comes from [DXMT](#dxmt), [vkd3d-proton](#vkd3d-proton) and
  [mtld3d](#mtld3d), in the trees `DXGI_IMPLS` and `D3D9_IMPLS` name.
  Their 64-bit DLLs are ARM64X: an x64 game loads the ARM64EC half, so
  they run natively and only the game's own code goes through FEX, and a
  native ARM64 program loads the ARM64 half.
- Vulkan runs on KosmicKrisp ([Vulkan](#vulkan)), OpenGL on Zink
  ([OpenGL](#opengl)).
- Media decoding runs on a GStreamer built here ([GStreamer](#gstreamer)).
- A [Wine SDK](#wine-sdk) is staged for the PE code built outside Wine.
- The loader runs from a signed app bundle with 4K pages
  ([Loader app](#loader-app)).

## Requirements

- An Apple Silicon Mac and a macOS SDK at least as new as `MACOS_FLOOR`.
- Homebrew at `DEPS_PREFIX` with the formulae in
  [`brew-formulae`](../pins/brew-formulae), which a CI build installs.
  Only a host on the floor's major macOS version gets bottles that meet
  the floor
  ([Homebrew libraries and the floor](#homebrew-libraries-and-the-floor)).
- Xcode's Metal toolchain (`xcrun metal`), for DXMT's shaders.
- [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) at `MINGW_DIR`,
  with its arm64ec toolchain. The scripts append its `bin/` to `PATH`;
  never prepend it, since its unprefixed `clang` breaks configure's host
  compiler detection.
- Rust with the `RUST_TARGET` target, for `compatdb.so`.
- A code signing identity in the keychain whose certificate
  [`signing/loader.provisionprofile`](../signing/loader.provisionprofile)
  lists.
- Network access:
  - The bundle step resolves the `*-src.ref` pins and gets the loader's
    secure timestamp.
  - `build-fex.sh`, `build-mesa.sh`, `build-dxmt.sh` and
    `build-vkd3d-proton.sh` resolve their ref against `*_REPO` on every
    run, which needs the network unless `*_REPO` is a local clone.
  - When they build: `build-vkd3d-proton.sh` for the submodules,
    `build-mesa.sh` for a new Vulkan loader or Python environment, and
    `build-gstreamer.sh` unless its clone and wraps are already there.

## Commands

| Script | What it does |
|--------|--------------|
| [`build-wine.sh`](../bin/build-wine.sh) | Runs `build-mesa.sh` and `build-gstreamer.sh`, builds Wine and the [Wine SDK](#wine-sdk), then runs `build-fex.sh`, `build-dxmt.sh` and `build-vkd3d-proton.sh`; passes `--clean` on to all five |
| [`build-mesa.sh`](../bin/build-mesa.sh) | The Vulkan loader, KosmicKrisp, and EGL with Zink ([Vulkan](#vulkan), [OpenGL](#opengl)) |
| [`build-gstreamer.sh`](../bin/build-gstreamer.sh) | GStreamer with glib and FFmpeg ([GStreamer](#gstreamer)) |
| [`build-fex.sh`](../bin/build-fex.sh) | FEX's emulator DLLs and their unix halves ([FEX](#fex)) |
| [`build-dxmt.sh`](../bin/build-dxmt.sh) | DXMT, after a Wine build ([DXMT](#dxmt)) |
| [`build-vkd3d-proton.sh`](../bin/build-vkd3d-proton.sh) | vkd3d-proton's D3D12 DLLs, after a Wine build ([vkd3d-proton](#vkd3d-proton)) |
| [`bundle-wine.sh`](../bin/bundle-wine.sh) | Assembles the bundle, signs the loader and checks the result ([what the bundle step checks](#what-the-bundle-step-checks)) |

The flags are in [building.md](building.md#commands). The five component
scripts also run alone.

- FEX, Mesa, DXMT and vkd3d-proton are fetched from their `*_REPO` at the
  ref their `*-src.ref` names. A commit that is only in a local clone is
  built only with `*_REPO` set to that clone.
- Each staged directory (`*_DIST`) holds a `KEY` line naming what it was
  built from: the source commit, the hashes of the scripts, the tool
  versions (`dist_key` in `profile.sh`, and each script's key function).
- A script builds nothing when its directory is complete and has the
  `KEY` it would write now (`skip_current_dist`); `--clean` deletes the
  directory. DXMT also builds whenever the Wine source has uncommitted
  changes.
- `--print-key` prints that `KEY` and exits; the release workflow keys
  its caches on it ([other caches](maintenance.md#other-caches)).
- `WINE_BUILD_CCACHE`, the absolute path of a ccache, runs the compilers
  of Wine, Mesa and FEX through it (`profile.sh`). It changes neither
  configure results nor what is built, and needs no `--clean`. DXMT and
  vkd3d-proton do not use it.

## Paths

Every tree lives under `build/` and the bundle in `dist/wine`, in the
[layout](building.md#layout). Each variable below overrides the default of
the same name, set in [`profile.sh`](../bin/lib/profile.sh) or, for
`MINGW_DIR` and `CACHE_DIR`, at the top of the scripts that use them. A
`MACOSX_DEPLOYMENT_TARGET` may raise the floor but not lower it.

| Variable | What it is |
|----------|------------|
| `WINE_SRC` | The Wine source, a checkout of athei/wine |
| `BUILD_DIR` | The Wine build tree; also holds cargo's target directory for `compatdb.so`, one per checkout of this repository (`COMPATDB_TARGET` in [`bundle-wine.sh`](../bin/bundle-wine.sh)) |
| `X86LIBS_BUILD_DIR` | The x86 tree for the SDK's x86_64 archives and [graphics tests](#graphics-tests) |
| `WINE_SDK_DIR` | The [Wine SDK](#wine-sdk) |
| `FEX_REPO`, `MESA_REPO`, `DXMT_REPO`, `VKD3D_PROTON_REPO` | The repositories the forks are fetched from |
| `FEX_SRC`, `MESA_SRC` | The local clones `build-fex.sh` and `build-mesa.sh` check out |
| `FEX_BUILD_DIR`, `MESA_BUILD_DIR`, `DXMT_BUILD_DIR`, `VKD3D_PROTON_BUILD_DIR`, `GSTREAMER_BUILD_DIR` | Each component's trees, and the clones its script keeps there |
| `DXMT_LLVM` | The LLVM static libraries `winemetal.so` links |
| `FEX_DIST`, `MESA_DIST`, `DXMT_DIST`, `VKD3D_PROTON_DIST`, `GSTREAMER_DIST` | What the bundle installs from each component, with the commits it was built from; by default inside the component's build directory |
| `LOADER_PROFILE` | The provisioning profile the loader is signed for |
| `MINGW_DIR` | llvm-mingw |
| `CACHE_DIR` | Downloaded redistributables |

## Runtime

The variables are listed in the
[README](../README.md#environment-variables).

msync is CrossOver's implementation of Windows synchronization objects on
Mach semaphores and shared memory, which saves wineserver round trips.

- Only the wineserver reads `WINEMSYNC`. Unset or a non-zero number, msync
  is on. Any other value (`0`, empty, not a number) turns it off, and Wine
  synchronizes through wineserver requests as upstream Wine does.
- Set `WINEMSYNC=0` where the wineserver session starts, normally the
  launcher's first Wine process. It takes effect only for a new session,
  and every process of the session follows the wineserver.
- A per-game compatdb `env` rule cannot turn msync off reliably: the game
  only starts the wineserver when no session is running yet.
- With msync on, the wineserver prints `msync: up and running.` on its
  standard error and registers a launchd service named
  `wine-<dev>-<inode>-msync` (`launchctl print gui/<uid>` lists it for a
  wineserver of the GUI login session).
- When it cannot register that service, it prints an `msync: error:` line
  and runs the session without msync. The messages are in
  [`server/msync.c`](https://github.com/athei/wine/blob/arm64-patched/server/msync.c).

`WINENCPU` limits the number of processors Wine reports, for programs that
misbehave with many.

- Set it where the session starts, like `WINEMSYNC`. Each process limits
  the count it reports itself, but the `CentralProcessor` registry keys
  and `NUMBER_OF_PROCESSORS` are written once per session, and FEX's
  emulated CPUID reads those keys, so a value set later for one game
  leaves them at the session's count.
- A value of 0, one that is not a number, or one above the real count is
  ignored.

## FEX

FEX runs the x86 code of a process; Wine loads it as the emulator DLLs of
`aarch64-windows` and `i386-windows`. FEX is
[athei/FEX](https://github.com/athei/FEX) at the ref in
[`fex-src.ref`](../pins/fex-src.ref): upstream FEX plus our commits. Two
of its modes differ from upstream. They are emulator settings, changed
with FEX's own variables, not compatdb settings.

- Memory ordering. With `FEX_HARDWARE_TSO` unset or `1`, FEX runs x86 code
  with the hardware x86 memory ordering (TSO) instead of emulating it with
  ordered loads and stores. It switches a thread to TSO when the thread
  enters x86 code and back at its syscalls and unix calls, since native
  code runs slower in it. `0` makes FEX emulate the ordering; any other
  value logs an error and does the same. The [loader app](#loader-app)'s
  entitlement gates the mode.
- AFP. x86_64 programs, which run as ARM64EC, run without AFP (the Arm
  floating-point behaviour FEX uses to match x86): the floating-point mode
  would have to change at every call between x86 and native code, which
  costs more than AFP saves. `enableafp` in `FEX_HOSTFEATURES` turns it
  back on. i386 programs keep AFP.
- FEX reads its options from the Windows environment block, which
  compatdb.so has already rewritten, so a compatdb `env` entry sets one
  for one program. A FEX config file does not override such an entry.

## Wine SDK

PE code built outside Wine links against Wine's `libwinecrt0.a` and
`libntdll.a` and runs `winebuild` to mark itself a builtin.
`build-wine.sh` stages all of it in one directory, in the layout of an
installed Wine, with a `WINE_COMMIT` file naming the Wine source commit.
What goes in, and why, is the Wine SDK step of
[`build-wine.sh`](../bin/build-wine.sh).

- A full bundle carries the same files, so the bundle is an installed Wine
  with its SDK. [mtld3d](https://github.com/athei/mtld3d) builds against
  an installed Wine; its
  [`docs/BUILDING.md`](https://github.com/athei/mtld3d/blob/main/docs/BUILDING.md)
  lists what that build needs.
- `build-dxmt.sh` and the bundle step refuse an SDK whose `WINE_COMMIT` is
  not the Wine source's current commit.

### Graphics tests

The SDK and a full bundle carry Wine's graphics conformance tests as
`lib/wine/tests/<pe-dir>/<name>_test.exe`, to test the bundle's own stack
(mtld3d, DXMT, wined3d, the Vulkan loader) with nothing but the bundle.
mtld3d's conformance suite runs the `d3d9_test.exe` of both directories.

- The tests are `GRAPHICS_TESTS` in [`profile.sh`](../bin/lib/profile.sh),
  built for `TEST_PE_DIRS`, the x86 architectures games use. An x86_64
  test runs as ARM64EC, as an x64 game does.
- They are plain executables, outside the directories Wine searches for
  modules.
- `lib/wine/tests/VERSION` holds the `wine --version` string of the Wine
  they were built from, so a copy of `lib/wine/tests` taken alone still
  names its build. mtld3d's conformance runner reads its first line.
- `d3d12_test.exe` needs the `dxmt` tree ([vkd3d-proton](#vkd3d-proton)).
  On the `wined3d` tree the loader stops it with `c0000135` (a missing
  DLL) and it exits with 53.

To run one in a prefix of its own, give its path to the bundle's `wine`,
with the names of the parts to run (all without a name; the names are the
`START_TEST` entries in the test's sources):

```bash
WINEPREFIX=/tmp/gfx-prefix dist/wine/bin/wine \
    dist/wine/lib/wine/tests/x86_64-windows/d3d9_test.exe device
```

The test picks its Direct3D tree through compatdb as a game does, so a
`WINE_COMPATDB` rule for `d3d9_test.exe` puts it on another tree
([recipes](https://docs.mach9.app/compatdb_core/#recipes)).
It prints a summary line per part and exits with the number of failed
checks, at most 255.

## DXMT

DXMT is [athei/dxmt](https://github.com/athei/dxmt) at the ref in
[`dxmt-src.ref`](../pins/dxmt-src.ref): upstream DXMT plus our commits,
among them the `dxgi.dll` support for vkd3d-proton's swapchains. How it is
built (ARM64X and i386 DLLs, `winemetal.so` from the same meson tree, the
LLVM its shader converter links) is the header of
[`build-dxmt.sh`](../bin/build-dxmt.sh).

The DLLs and `winemetal.so` share a private call interface, so they come
from one checkout; the bundle step checks the recorded commit, version and
Wine commit (`check_dxmt_build`).

## vkd3d-proton

Direct3D 12 is [athei/vkd3d-proton](https://github.com/athei/vkd3d-proton)
at the ref in [`vkd3d-proton-src.ref`](../pins/vkd3d-proton-src.ref):
upstream vkd3d-proton plus our commits. How it is built is the header of
[`build-vkd3d-proton.sh`](../bin/build-vkd3d-proton.sh).

- It translates D3D12 to Vulkan on KosmicKrisp, whose features it needs
  are always on in our Mesa ([Vulkan](#vulkan)).
- Our commits release a swapchain's frame latency handle once the GPU has
  finished a present, not once the present is on screen, which on
  KosmicKrisp is one or more refresh cycles later. Otherwise a game that
  waits on the handle at the default latency of 1 starts a frame only every
  few refresh cycles, with the GPU idle and at a low clock in between.
  vkd3d-proton's own frame latency limit still waits for the display.
  `smoke/d3d12.c` runs a waitable swapchain, and `check_d3d12_runs` checks
  that vkd3d-proton reports this for it.
- `d3d12.dll` and `d3d12core.dll` go into the `dxmt` tree. vkd3d-proton
  has no DXGI of its own: DXMT's `dxgi.dll` presents its Vulkan swapchain
  into the window, so the two are always loaded together.
- No other tree has D3D12 (Step 3b of
  [`bundle-wine.sh`](../bin/bundle-wine.sh)): under `dxgi=wined3d`,
  `d3d12.dll` does not load (error 126).

## mtld3d

mtld3d comes from the release tarball pinned in
[`redist.env`](../pins/redist.env), not built here. Step 4 of
[`bundle-wine.sh`](../bin/bundle-wine.sh) says what it takes from the
tarball. It refuses one whose 64-bit DLLs are not ARM64X, since
compatdb.so gives native ARM64 programs the mtld3d tree too
([native ARM64 programs](compatdb.md#native-arm64-programs)).

## Vulkan

Wine's Vulkan (`vulkan-1.dll`, `winevulkan`) runs on KosmicKrisp, Mesa's
Vulkan driver on Metal, through the Khronos Vulkan loader. mtld3d and DXMT
go to Metal directly; D3D12 uses Vulkan through vkd3d-proton, and wined3d
can, per game.

- Mesa is [athei/mesa](https://github.com/athei/mesa) at the ref in
  [`mesa-src.ref`](../pins/mesa-src.ref): upstream Mesa plus our topic
  commits, originally from
  [Hadron's Mesa patches](https://github.com/Broyojo/hadron/tree/main/patches/mesa).
  Commits of upstream merge requests not merged yet are carried under
  their original authors and dropped once upstream has them
  ([rebasing](maintenance.md#mesa-and-the-vulkan-loader)).
- KosmicKrisp is built in two stages so the driver links system libraries
  only, and the loader is built from source for the floor; both are in the
  header of [`build-mesa.sh`](../bin/build-mesa.sh).
- `win32u.so` loads the loader the first time something in the process
  needs Vulkan. That includes the display device refresh `explorer.exe`
  runs when a session starts, which takes the GPU's name, memory size and
  UUID for the registry from KosmicKrisp.
- The loader finds the driver through the manifest in
  `share/vulkan/icd.d/`, whose `library_path` is relative, so the bundle
  stays relocatable. ntdll sets `VK_DRIVER_FILES` to that directory unless
  the caller set `VK_DRIVER_FILES`, `VK_ICD_FILENAMES` or
  `VK_ADD_DRIVER_FILES`. A caller that sets one chooses the drivers
  itself, and has to name the bundled one to use it. A portability driver
  loads through `VK_DRIVER_FILES` with its stock manifest.
- The features our commits add (device-generated commands, transform
  feedback, sparse resources), which D3D12 and Zink need, are always
  advertised. `MESA_KK_EXPERIMENTAL` keeps only Mesa's own experimental
  flags
  ([`kosmickrisp.rst`](https://gitlab.freedesktop.org/mesa/mesa/-/blob/main/docs/drivers/kosmickrisp.rst));
  the bundle sets none.
- wined3d (the `wined3d` trees, D3D8 and DirectDraw) keeps its OpenGL
  renderer, which reaches Direct3D 11 on Zink ([OpenGL](#opengl)) and a
  lower feature level on Apple's OpenGL. Its Vulkan renderer is not the
  default because it still crashes in many of Wine's conformance tests and
  does not fall back to OpenGL. A compatdb rule turns it on for one game
  ([`Rule::wined3d`](https://docs.mach9.app/compatdb_core/struct.Rule.html#method.wined3d)).
- DXVK is not covered. A program that wants Vulkan itself gets it.

## OpenGL

OpenGL runs on Zink on KosmicKrisp through Mesa's EGL wherever
KosmicKrisp has a GPU, instead of on Apple's OpenGL, which stops at 4.1
core and 2.1 compatibility.

- `build-mesa.sh` builds `libEGL.1.dylib` and the gallium library with
  Zink, without GLES, GLX and GBM; the bundle ships both in
  `lib/external/`, where Zink finds the bundled Vulkan loader
  (`check_egl` in [`bundle-wine.sh`](../bin/bundle-wine.sh)).
- `build-wine.sh` passes `--with-opengl` and configure finds this EGL
  through the `egl.pc` in `MESA_DIST`, so a build without it fails.
  `SONAME_LIBEGL` is patched to `lib/external/` like the Vulkan loader.
- Zink reports OpenGL above 2.1 only with transform feedback, which our
  KosmicKrisp always advertises.
- The Mac driver takes the choice from compatdb's `opengl` setting
  ([`Rule::opengl`](https://docs.mach9.app/compatdb_core/struct.Rule.html#method.opengl)).
- With `egl` it does not fall back: where EGL does not start, as on a Mac
  without a GPU of the Metal 4 family, it logs a `winediag` message and
  the process gets no OpenGL.

## GStreamer

`winegstreamer` decodes audio and video for DirectShow, Media Foundation
and the DMOs games use (the WMA decoder of `wmadmod.dll` among them) on a
GStreamer the bundle carries, so no GStreamer needs to be installed.

- [`build-gstreamer.sh`](../bin/build-gstreamer.sh) builds it from source
  for the floor, LGPL only, with just the plugins and FFmpeg decoders
  games need; its header has the lists and the checks it runs on what it
  stages. Homebrew's GStreamer and FFmpeg are GPL and are not used.
- `build-wine.sh` passes `--with-gstreamer` and configure finds it through
  the `.pc` files in `GSTREAMER_DIST`, so a build without it fails.
  `winegstreamer.so` links its libraries directly, by their paths under
  `lib/external/` ([relocation](maintenance.md#relocation)).
- ntdll sets `GST_PLUGIN_SYSTEM_PATH_1_0` to the bundle's
  `lib/external/gstreamer-1.0/` and unsets
  `GST_PLUGIN_PATH` and `GST_PLUGIN_PATH_1_0`, unless the caller set
  `GST_PLUGIN_SYSTEM_PATH` or `GST_PLUGIN_SYSTEM_PATH_1_0`. A plugin of
  another GStreamer would load a second GStreamer into the process.
- In that case it also points `GST_REGISTRY_1_0` at a file of its own
  under `XDG_CACHE_HOME` (or `~/.cache`), unless the caller set
  `GST_REGISTRY` or `GST_REGISTRY_1_0`, so another GStreamer does not
  write the same registry.
- There is no `gst-plugin-scanner`: the first process of a new bundle that
  uses GStreamer reads the plugins itself.

## Loader app

The loader needs `com.apple.developer.cross-architecture-support`, and Wine
does not run without it. macOS honours it only through a provisioning
profile embedded in an app bundle and a signature by a certificate of the
profile's team. The entitlement:

- leaves most of the 4 GB `__PAGEZERO` as a mapping Wine can replace,
  which Wine claims as its low reserved area for i386 code under FEX;
- covers the space between 4 GB and the randomly placed loader the same
  way, which the loader reserves so that an x64 executable built for
  0x100000000 without relocations can load there;
- lets a process run with 4K pages instead of 16K;
- gates the hardware x86 memory ordering of [FEX](#fex).

How the loader uses it:

- It execs itself with 4K pages before Wine sets up its address space
  (`exec_with_4k_pages` in
  [`loader/main.c`](https://github.com/athei/wine/blob/arm64-patched/loader/main.c)),
  and ntdll starts new Wine processes with 4K pages directly
  (`preloader_exec` in
  [`dlls/ntdll/unix/loader.c`](https://github.com/athei/wine/blob/arm64-patched/dlls/ntdll/unix/loader.c)).
- Everything in a Wine process, FEX included, then runs on 4K pages, so
  page protections, guard pages, decommit and FEX's tracking of
  self-modifying code work as on Windows. The wineserver and other plain
  Unix programs keep the default page size.
- When the kernel placed the loader too close to 4 GB for such an
  executable, the loader execs itself again for a new placement, a bounded
  number of times, and warns and keeps its placement if that fails (the
  threshold and limit are in `loader/main.c`).
- A loader without the entitlement would be killed in that exec without a
  message, so it checks first and exits with an error saying it must run
  from the signed `wine.app`. There is no fallback to 16K pages.

The app:

- The bundle step wraps the loader into `lib/wine/aarch64-unix/wine.app`
  and links the entry points to it (`package_loader` in
  [`bundle-wine.sh`](../bin/bundle-wine.sh)).
- It is named Mach9, the launcher this Wine runs under, and carries the
  games category and Game Mode keys, so a game the launcher starts gets
  Game Mode. The bundle identifier stays the one the profile grants.
- The signature claims only what the loader needs; `package_loader` and
  `LOADER_RUNTIME_ENTITLEMENTS` list each entitlement with its reason.
- It is signed once as a whole with the hardened runtime and a secure
  timestamp, as notarization and Mach9's install verification require.
  `LOADER_TIMESTAMP=0` signs without the timestamp for an offline local
  build, which cannot be notarized.
- An app that embeds the bundle leaves `wine.app` as it is: re-signing it
  or editing its `Info.plist` loses the profile or breaks the signature.
  Entitlements take effect only on a main executable, and every Wine
  process runs as this one, so the other Mach-O files of the bundle need
  none; the app signs them for notarization with the hardened runtime.

## What the bundle step checks

The preflight runs before anything is deleted, Step 5 after assembly, and
the smoke test last, in [`bundle-wine.sh`](../bin/bundle-wine.sh), with
`check_pe64_native` in [`profile.sh`](../bin/lib/profile.sh). Each
function's comment says what it checks and why; each program in
[`smoke/`](../smoke) says what it does in its header.

| Checks | Where |
|--------|-------|
| macOS SDK, formulae, Wine SDK, each staged component current against its pin (which needs the network), llvm-mingw tools, provisioning profile and identity | Preflight: `check_*_build`, `check_wine_sdk`, `loader_preflight` |
| Inherited `com.apple.quarantine` removed, nothing else | Step 4f |
| No reference to `LEAK_PREFIXES`, no `@rpath` dependency outside the bundle, every dylib loads ([`smoke/dlopen.c`](../smoke/dlopen.c)), GStreamer loads only from the bundle | `check_no_leaks`, `check_rpath_deps`, `check_dylibs_load`, `check_gstreamer` |
| Copied-in components in place, mtld3d's DLLs the release's, FEX as `xtajit*.dll`, marker machines, native 64-bit PE ([Direct3D trees](maintenance.md#direct3d-trees)) | Step 5, `check_trees_arm64x`, `check_pe64_native` (`profile.sh`) |
| No PE debug sections or symbol tables (stripped by Step 4e), no dangling symlinks | Step 5 |
| Vulkan driver and manifest, EGL with Zink, ntdll's compatdb hooks | `check_vulkan`, `check_egl`, `check_compat_hooks` |
| `wine.app` signature, entitlements and `Info.plist` | `check_loader_app` |
| `minos` of every Mach-O at most the floor, no import of `FLOOR_NEWER_FUNCS` ([macOS floors](maintenance.md#macos-floors)) | `check_minos`, `check_floor_newer_imports` |
| A fresh prefix boots with msync and compatdb, and the programs in `smoke/` run on every arch and tree: Direct3D trees, WGL, Vulkan, D3D12, wined3d, WMA decoding, and EGL and GStreamer outside Wine | `smoke_test` and the `check_*` functions it calls |

On a Mac without a Metal 4 GPU, such as a hosted CI runner, the checks
that need a KosmicKrisp device or Zink change or are left out, and so are
those that need Apple's accelerated OpenGL on a Mac without it
([`smoke/metal4.m`](../smoke/metal4.m),
[`smoke/opengl.m`](../smoke/opengl.m)). The step says which.

## Homebrew libraries and the floor

The libraries in `lib/external/` other than Mesa's and GStreamer's are
Homebrew's: `DEPS_FORMULAE` and their runtime closure. Homebrew builds each
bottle for one macOS major version, with the deployment target at its
`.0` release, and pours the bottle of the host's version, so the host
decides whether those libraries meet the floor.

- On a host of the floor's major version, every bottle meets the floor. A
  CI or release build runs on such a host and passes `--strict-floor`
  ([releases](maintenance.md#releases)).
- On a newer macOS, the bundle step warns about each newer bottle. That is
  expected for a local build, which is not distributed.
- A formula with no bottle for the host is compiled there and carries the
  host's full version as `minos`, which can exceed the floor even on its
  major version. `--strict-floor` stops such a bundle.
- Before moving the build to another macOS version, check that version's
  bottles: `formulae.brew.sh/api/formula/<name>.json` lists every tag's
  bottle (`brew fetch --bottle-tag` may know only the host's). Unpack each
  and read its dylibs' `minos` with `vtool -show-build`.

Static linking:

- Nothing from Homebrew is linked statically into a file the bundle
  ships.
- Wine's unix modules load Homebrew's libraries by soname.
- The libraries that are linked (GStreamer's, DXMT's LLVM, DXMT's and
  Mesa's own archives) are built here for the floor, with Homebrew off
  `PATH` for Mesa and GStreamer.
- `minos` does not show a static object that needs a newer macOS; only
  the linker's warning that an object `was built for newer` macOS does.
  After adding a static dependency, search a clean build's log for it.

## Limitations

- A native ARM64 program cannot load a Direct3D name whose marker in
  `aarch64-windows` is AMD64: the loader fails with error 193
  (`ERROR_BAD_EXE_FORMAT`). The bundle step therefore stamps hybrid
  markers and requires every 64-bit module of every tree to be ARM64X
  ([Direct3D trees](maintenance.md#direct3d-trees)).
- A tree changed after the bundle step, such as by a development install
  of mtld3d, has to keep to that, or a native ARM64 program cannot load
  its module. Which tree such a program gets is compatdb's choice
  ([native ARM64 programs](compatdb.md#native-arm64-programs)).

## Pinned inputs

A `*-src.ref` file names a branch, tag or commit of the component's
`*_REPO`.

| Input | Pinned in |
|-------|-----------|
| FEX | [`fex-src.ref`](../pins/fex-src.ref) |
| Mesa (KosmicKrisp) | [`mesa-src.ref`](../pins/mesa-src.ref) |
| DXMT | [`dxmt-src.ref`](../pins/dxmt-src.ref) |
| vkd3d-proton | [`vkd3d-proton-src.ref`](../pins/vkd3d-proton-src.ref); its submodules at the commits that ref records |
| mtld3d | [`redist.env`](../pins/redist.env): URL and SHA-256 of the release tarball |
| LLVM for `winemetal.so` | [`redist.env`](../pins/redist.env): tag and commit |
| Vulkan loader and headers | [`redist.env`](../pins/redist.env): tag and commit of each, from one SDK release |
| Python modules of Mesa's build scripts | [`redist.env`](../pins/redist.env): pip requirements with exact versions |
| GStreamer, glib and FFmpeg | [`redist.env`](../pins/redist.env): the monorepo's tag and commit, and the commit of the FFmpeg port branch its wrap names; the other wraps pin their tarballs by checksum |
| Wine | Not pinned: whatever `WINE_SRC` has checked out. A CI build checks out the branch [`wine-src.ref`](../pins/wine-src.ref) names. The SDK and DXMT record the commit, and the bundle step refuses either when it is not the current one. All uncommitted states count as one (`-dirty`), so the bundle step cannot tell them apart: rebuild after changing uncommitted changes |
| Homebrew libraries | Not pinned ([Homebrew libraries and the floor](#homebrew-libraries-and-the-floor)); a CI build installs [`brew-formulae`](../pins/brew-formulae) at Homebrew's newest versions ([Homebrew formulae](maintenance.md#homebrew-formulae)) |
| llvm-mingw | A CI build: [`redist.env`](../pins/redist.env), version and SHA-256. A local build: `MINGW_DIR` |
| Provisioning profile | [`signing/loader.provisionprofile`](../signing/loader.provisionprofile); replace it when it expires or the App ID changes |
