# The arm64 build

`build-wine.sh` and `bundle-wine.sh` build and bundle a native arm64 Wine
that runs x86 programs through [FEX](https://github.com/FEX-Emu/FEX). The
settings they share are in [`profile.sh`](../bin/lib/profile.sh). In short:

- The Wine source is upstream Wine with our own series on top, the branch
  [`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) of
  athei/wine. Some of its commits come from Hadron, and it carries
  CrossOver's msync (see [Runtime](#runtime)). It is configured against the
  arm64 Homebrew with the allowlist in
  [`build-wine.sh`](../bin/build-wine.sh), plus the configure results
  the floor overrides (`FLOOR_NEWER_FUNCS`,
  [macOS floors](maintenance.md#macos-floors)).
- `WINE_ARCHS` in `profile.sh` names the PE architectures: aarch64 with
  arm64ec, so that x64 code runs as ARM64EC, and i386 for WoW64.
- No-execute follows upstream Wine by default. With no-execute off, every
  readable page is executable, so FEX has to treat all writable memory as
  possible code and translates it again whenever it changes. `WINE_FORCE_NX`
  set to anything but `0` keeps no-execute permanently on, so the process
  cannot turn it off
  ([environment variables](../README.md#environment-variables)). For a
  program whose executable lacks the `NX_COMPAT` flag and that runs slowly
  for that reason, a compatdb rule sets the variable
  ([recipes](compatdb.md#recipes)).
- FEX is upstream FEX with our own commits on top, from
  [athei/FEX](https://github.com/athei/FEX) at the ref
  [`fex-src.ref`](../pins/fex-src.ref) names. compatdb's built-in
  `fex-x87-reduced-precision` rule asks FEX for faster x87 emulation
  ([defaults](../README.md#defaults-that-differ-from-wine),
  [FEX and KosmicKrisp](compatdb.md#fex-and-kosmickrisp)).
- DXMT is built from source against this Wine (see [DXMT](#dxmt)); mtld3d
  comes from its release. The trees the bundle carries are `DXGI_IMPLS` and
  `D3D9_IMPLS` in `profile.sh`.
- Wine's Vulkan is on, on KosmicKrisp, Mesa's Vulkan driver on Metal, and
  a Vulkan loader built for the floor (see [Vulkan](#vulkan)).
- A [Wine SDK](#wine-sdk) is staged for the PE code built outside Wine.
- The loader is wrapped in a signed app bundle, and Wine processes run with
  4K pages (see [Loader app](#loader-app)).

## Requirements

- An Apple Silicon Mac and a macOS SDK at least as new as `MACOS_FLOOR` in
  [`profile.sh`](../bin/lib/profile.sh).
- The arm64 Homebrew at `DEPS_PREFIX` with the formulae in `DEPS_FORMULAE`
  (both in `profile.sh`), plus `cmake`, `ninja` and `meson`, and what the
  preflight of [`build-mesa.sh`](../bin/build-mesa.sh) checks for,
  among it the formulae KosmicKrisp's build tools link. Only a host on the
  floor's major macOS version gets bottles that meet the floor (see
  [Homebrew libraries and the floor](#homebrew-libraries-and-the-floor)).
- Xcode's Metal toolchain (`xcrun metal`), for DXMT's shaders.
- [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) at `MINGW_DIR`,
  including its arm64ec toolchain. The scripts append its `bin/` to `PATH`;
  never prepend it, since its unprefixed `clang` breaks configure's host
  compiler detection.
- Rust with the `aarch64-apple-darwin` target (`RUST_TARGET` in
  `profile.sh`), for `compatdb.so`.
- In the keychain, a code signing identity of the team that
  [`signing/loader.provisionprofile`](../signing/loader.provisionprofile)
  belongs to, whose certificate the profile lists.
- Network access for the bundle step, which resolves `fex-src.ref` and
  `mesa-src.ref` and gets the loader's secure timestamp. `build-fex.sh` and
  `build-mesa.sh` fetch their ref from `FEX_REPO` and `MESA_REPO` on every
  run, which needs the network unless the variable names a local clone.
  `build-mesa.sh` also needs it whenever it builds the Vulkan loader or the
  Python environment of Mesa's build scripts anew (a first run, or changed
  pins), to clone the loader and headers and install the Python modules.

## Commands

| Script | What it does |
|--------|--------------|
| `build-wine.sh` | Runs `build-mesa.sh`, builds Wine against the loader it staged, stages the [Wine SDK](#wine-sdk), then runs `build-fex.sh` and `build-dxmt.sh`; `--clean` is passed on to all three |
| [`build-mesa.sh`](../bin/build-mesa.sh) | Checks out the ref in [`mesa-src.ref`](../pins/mesa-src.ref) in `MESA_SRC` (refusing a tree with local changes), builds the Vulkan loader from the tags pinned in [`redist.env`](../pins/redist.env) and KosmicKrisp, and stages both with the driver's manifest and their commits in `MESA_DIST` (see [Vulkan](#vulkan)). Runs alone too |
| [`build-fex.sh`](../bin/build-fex.sh) | Checks out the ref in [`fex-src.ref`](../pins/fex-src.ref) in `FEX_SRC` (refusing a tree with local changes), builds FEX's emulator DLLs and their unix halves, and records the commit. Runs alone too |
| [`build-dxmt.sh`](../bin/build-dxmt.sh) | Builds DXMT from the commit pinned in [`redist.env`](../pins/redist.env) against the Wine build and SDK, and stages it with its commit and `git describe` in `DXMT_DIST`. Runs alone too, after a Wine build |
| `bundle-wine.sh` | Assembles the bundle from the Wine build, the SDK, FEX, DXMT, the Vulkan loader and KosmicKrisp, and the mtld3d release, signs the loader and runs the smoke test |

The flags are in the [README](../README.md#commands).

## Paths

Every tree lives under `build/` and the bundle in `dist/wine`, in the
[layout](../README.md#layout). Each variable below overrides the default
of the same name; the defaults are set in
[`profile.sh`](../bin/lib/profile.sh), except `MINGW_DIR` and
`CACHE_DIR`, which are set at the top of the scripts that use them. A
`MACOSX_DEPLOYMENT_TARGET` may raise the floor but not lower it.

| Variable | What it is |
|----------|------------|
| `WINE_SRC` | The Wine source, a checkout of athei/wine |
| `BUILD_DIR` | The Wine build tree; with `--compatdb-only` only cargo's target directory, `$BUILD_DIR/compatdb` |
| `X86LIBS_BUILD_DIR` | The x86 tree for the x86_64 archives and the [graphics tests](#graphics-tests) of the SDK |
| `WINE_SDK_DIR` | The [Wine SDK](#wine-sdk) |
| `FEX_REPO`, `FEX_SRC` | FEX repository and the local clone `build-fex.sh` checks out |
| `FEX_BUILD_DIR` | FEX's CMake trees |
| `DXMT_BUILD_DIR` | DXMT's checkout, its meson trees (x86_64 and i386 PE, arm64 unix) and, by default, `DXMT_LLVM` and `DXMT_DIST` |
| `DXMT_LLVM` | The LLVM static libraries `winemetal.so` links |
| `DXMT_DIST` | All of DXMT as the bundle installs it, with the commits it was built from |
| `MESA_REPO`, `MESA_SRC` | Mesa repository and the local clone `build-mesa.sh` checks out |
| `MESA_BUILD_DIR` | The Vulkan loader's and headers' clones and trees, KosmicKrisp's two meson trees, the Python environment and, by default, `MESA_DIST` |
| `MESA_DIST` | The Vulkan loader, KosmicKrisp and its manifest as the bundle installs them, with the commits they were built from |
| `LOADER_PROFILE` | The provisioning profile the loader is signed for |
| `MINGW_DIR` | llvm-mingw |
| `CACHE_DIR` | Downloaded redistributables |

## Runtime

The variables are listed in the
[README](../README.md#environment-variables).

- msync is CrossOver's implementation of Windows synchronization objects
  for macOS, built on Mach semaphores and shared memory, which saves
  wineserver round trips.
- `WINEMSYNC=0` has to be in the environment of whatever starts the
  wineserver session, normally the launcher's first Wine process, and
  takes effect only for a new session. Every process of the session follows
  the wineserver's choice whatever its own environment says. A per-game
  compatdb `env` rule cannot turn msync off reliably: it reaches the game's
  process, which only starts the wineserver when no session is running yet.
- While msync is on, the wineserver prints `msync: up and running.` on its
  standard error when it starts and registers a launchd service named
  `wine-<dev>-<inode>-msync` while it runs; for a wineserver started from
  the GUI login session, `launchctl print gui/<uid>` lists it. With msync
  off neither appears.
- If the wineserver cannot register that service, it prints an
  `msync: error:` line saying msync is off for this wineserver and runs the
  session without msync. `msync: up and running.` then does not appear.
  The messages are in
  [`server/msync.c`](https://github.com/athei/wine/blob/arm64-patched/server/msync.c).
- `WINENCPU` belongs on the process that starts the session as well. Each
  process limits the count it reports itself, but the `CentralProcessor`
  registry keys and `NUMBER_OF_PROCESSORS` are written once per session,
  and FEX's emulated CPUID reads those keys, so a value set later for one
  game leaves them at the session's count.

## Wine SDK

PE code built outside Wine links against Wine's `libwinecrt0.a` (for the
`unix_lib.o` its unix calls go through) and `libntdll.a`, and runs
`winebuild` to mark itself a builtin. `build-wine.sh` stages one directory
with all of it, in the layout of an installed Wine, for every architecture
that code is built for, plus a `WINE_COMMIT` file naming the Wine source
commit. What goes in is the Wine SDK step of
[`build-wine.sh`](../bin/build-wine.sh).

- `bin/winebuild` and the `aarch64-windows` and `i386-windows` archives come
  from the build itself. Its `aarch64-windows` archives are ARM64X,
  carrying ARM64 and ARM64EC members, since arm64ec is paired with aarch64
  there.
- The `x86_64-windows` archives are what the arm64 build cannot make: x64
  objects do not link against ARM64EC ones. The x86 tree, a second
  configure of the same source for i386 and x86_64 that reuses the build's
  tools, builds just those and the [graphics tests](#graphics-tests).
  `build-wine.sh` configures that tree again from scratch whenever its
  configure arguments or `WINE_SRC` change, so neither needs a `--clean`.
- DXMT's PE build reads it through `-Dwine_install_path` (see
  [`build-dxmt.sh`](../bin/build-dxmt.sh)).
- A full bundle carries the same files, so the bundle is an installed Wine
  with its SDK. [mtld3d](https://github.com/athei/mtld3d) builds against
  an installed Wine, not the SDK directory; its
  [`docs/BUILDING.md`](https://github.com/athei/mtld3d/blob/main/docs/BUILDING.md)
  names the variables and lists what else that build needs.
- `build-dxmt.sh` and the bundle step refuse an SDK whose `WINE_COMMIT` is
  not the Wine source's current commit.

### Graphics tests

The SDK and a full bundle carry Wine's graphics conformance tests, the
programs of `dlls/<name>/tests` in the Wine source, as
`lib/wine/tests/<pe-dir>/<name>_test.exe`. They exist to test the bundle's
own stack (mtld3d, DXMT, wined3d on OpenGL or on KosmicKrisp, the Vulkan
loader) with nothing but the bundle; mtld3d's conformance suite runs the
`d3d9_test.exe` of both directories.

- The list is `GRAPHICS_TESTS` in [`profile.sh`](../bin/lib/profile.sh):
  `ddraw`, `d3d8`, `d3d9`, `d3d10`, `d3d10_1`, `d3d10core`, `d3d11`,
  `d3d12`, `dxgi`, `d2d1`, `opengl32` and `vulkan-1`.
- Each is built for `i386-windows` and `x86_64-windows`, the x86 code games
  run, except `d3d8`, which is i386 only: Wine builds no 64-bit `d3d8.dll`.
  An x86_64 test runs as ARM64EC, as an x64 game does. There are no
  `aarch64-windows` tests: the games the bundle is for are x86 code, and a
  native ARM64 program gets wined3d anyway
  ([native ARM64 programs](compatdb.md#native-arm64-programs)).
- The main build has tests disabled, since its `make` would otherwise build
  every Wine test, and makedep writes test rules only for the architectures
  a tree is configured for. So the x86 tree builds them, by name.
- They are plain executables, not builtins, outside the directories Wine
  searches for modules, and Step 4e strips them like every PE file.
- `d3d12_test.exe` cannot start until a D3D12 implementation is bundled.
  It imports `d3d12.dll`, which the bundle leaves out ([Vulkan](#vulkan)),
  so the loader stops it with `c0000135` (a missing DLL) and it exits
  with 53.

To run one against the bundle, in a prefix of its own, give the test's path
to the bundle's `wine`, with the names of the parts to run (all of them
without a name; the names are the `START_TEST` entries in the test's
sources):

```bash
WINEPREFIX=/tmp/gfx-prefix dist/wine/bin/wine \
    dist/wine/lib/wine/tests/x86_64-windows/d3d9_test.exe device
```

The test picks its Direct3D tree as a game does, through compatdb, so a
`WINE_COMPATDB` rule for `d3d9_test.exe` puts it on another tree
([recipes](compatdb.md#recipes)). It prints a summary line per part and
exits with the number of failed checks, at most 255.

## DXMT

- DXMT is built from the upstream commit pinned in
  [`redist.env`](../pins/redist.env), not taken from a release: the release's
  Metal shaders do not compile with the Metal toolchain this build uses.
  The comment there says which fix the pin carries.
- The PE DLLs are x86 code, x86_64 and i386, built with DXMT's own x86 cross
  files against the [Wine SDK](#wine-sdk), so they run under FEX together
  with the game. An ARM64EC build of DXMT would be a different DXMT.
- `winemetal.so` is arm64, linked against this Wine's `winemac.so` and
  `ntdll.so`. DXMT builds its unix half as a native target whose architecture
  follows the cross file, so `build-dxmt.sh` configures a meson tree with
  DXMT's `build-arm64ec.txt` and builds that one target.
- The DLLs and the `.so` share a private call interface, so they come from
  one checkout. `build-dxmt.sh` records the commit, its `git describe` (which
  DXMT also builds into `d3d11.dll` as its version) and the Wine commit it
  linked against; the bundle step checks all three against the pin, the
  DLLs and the Wine source. A meson tree is configured afresh, and so
  relinked, whenever the DXMT, Wine or LLVM commit or its options change.
- `winemetal.so` contains DXMT's shader converter (airconv), which links
  LLVM; the PE side only uses its headers. `build-dxmt.sh` builds the LLVM
  static libraries from the tag pinned in
  [`redist.env`](../pins/redist.env) into `DXMT_LLVM`, records the commit
  there, and rebuilds them only when the pin changes; `--clean` keeps them.

## Vulkan

Wine's Vulkan (`vulkan-1.dll`, `winevulkan`) runs on KosmicKrisp, Mesa's
Vulkan driver on Metal, through the Khronos Vulkan loader. The default
Direct3D implementations, mtld3d and DXMT, go to Metal directly and do not
use it; wined3d can, per game (see below).

- Mesa is [athei/mesa](https://github.com/athei/mesa) at the ref
  [`mesa-src.ref`](../pins/mesa-src.ref) names: upstream Mesa plus our topic
  commits, originally from
  [Hadron's Mesa patches](https://github.com/Broyojo/hadron/tree/main/patches/mesa)
  and regrouped by topic. Commits of upstream merge requests that are not
  merged yet are carried under their original authors and dropped once
  upstream has them
  ([rebasing](maintenance.md#mesa-and-the-vulkan-loader)). Like FEX, the
  ref is fetched from `MESA_REPO`, so a commit that is only in the local
  clone is built only with `MESA_REPO` set to that clone.
- KosmicKrisp is built in two stages. The driver's build compiles its
  OpenCL C kernels with tools of Mesa's own, which link LLVM and the SPIR-V
  libraries, and Homebrew has those only for the host's macOS. The first
  stage builds just these tools, for the host; the second builds the driver
  for the floor with LLVM off and the tools of the first stage, so the
  driver links system libraries only. As with Wine's configure, the second
  stage disables every optional library the driver does not need, so what
  else Homebrew has installed cannot change the build.
- `build-mesa.sh --clean` builds the driver again from scratch. The loader,
  the Python environment and the first stage's tools are kept while what
  they were built from is unchanged, as `build-dxmt.sh` keeps its LLVM.
- The loader is built from source, from the Vulkan-Loader and
  Vulkan-Headers tags pinned in [`redist.env`](../pins/redist.env), because
  Homebrew's vulkan-loader bottle needs a newer macOS than the floor. The
  bundle step copies it from `MESA_DIST` into `lib/external/`, which is
  what keeps Homebrew's loader out of the bundle: `SONAME_LIBVULKAN` holds
  only the file name, whichever loader configure linked. `MESA_DIST/lib`
  comes first in the `LDFLAGS` of `profile.sh` so that configure's `-lvulkan`
  test succeeds against this loader, without Homebrew's installed, and
  does not link Homebrew's where it is.
- `win32u.so` loads the loader the first time something in the process
  needs Vulkan. That includes the display device refresh `explorer.exe`
  runs when a session starts: it loads the loader and KosmicKrisp and takes
  the GPU's name, memory size and UUID for the registry from them.
- The loader finds the driver through the manifest in
  `share/vulkan/icd.d/`, whose `library_path` is relative to the manifest,
  so the bundle stays relocatable. ntdll sets `VK_DRIVER_FILES` to that
  directory when the caller has set none of `VK_DRIVER_FILES`,
  `VK_ICD_FILENAMES` and `VK_ADD_DRIVER_FILES`, so the loader uses the
  bundled driver and no other driver installed on the Mac. A caller that
  sets one of them chooses the drivers itself: the bundled one is then not
  added, and has to be named there to be used
  ([environment variables](../README.md#environment-variables)).
- Our topic commits add features KosmicKrisp advertises only when
  `MESA_KK_EXPERIMENTAL` names them. compatdb's built-in
  `kosmickrisp-experimental` rule sets the variable in every process, with
  the flags in [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs). As
  with every `env` rule, compatdb.so sets it when it loads, before any PE
  code runs, so it is there before win32u loads the Vulkan loader and the
  driver reads it. Mesa describes the variable in
  [`docs/drivers/kosmickrisp.rst`](https://gitlab.freedesktop.org/mesa/mesa/-/blob/main/docs/drivers/kosmickrisp.rst);
  the flags our topic commits add are defined next to Mesa's own in the
  driver's source. The recipes to turn them off are in
  [compatdb.md](compatdb.md#recipes).
- wined3d, Wine's own Direct3D, which serves the `wined3d` trees of both
  families and D3D8 and DirectDraw, keeps Wine's default OpenGL renderer.
  Apple's OpenGL is old enough to hold that renderer to a low Direct3D
  feature level, and wined3d's Vulkan renderer reaches a higher one on
  KosmicKrisp. It is not the default because wined3d on KosmicKrisp still
  crashes in many of Wine's conformance tests, and wined3d does not fall
  back to OpenGL when Vulkan fails. A compatdb rule with `wined3d=vulkan`
  turns it on for one game, for every wined3d user in that process,
  D3D8 and DirectDraw included
  ([wined3d on Vulkan](compatdb.md#wined3d-on-vulkan)).
- Wine's own `d3d12.dll` and `d3d12core.dll` are left out of the bundle.
- Not covered yet: Direct3D 12 through vkd3d-proton, DXVK, and OpenGL
  through Zink on KosmicKrisp. A program that wants Vulkan itself gets it.

## Loader app

The loader needs `com.apple.developer.cross-architecture-support`. With it,
the kernel leaves most of the 4 GB `__PAGEZERO` as a mapping Wine can
replace, which Wine claims as its low reserved area for the i386 code that
runs under FEX. The kernel places the loader a random distance above 4 GB
and covers the space between 4 GB and the loader the same way, and the
loader reserves that area for Wine too, so that an x64 executable built for
0x100000000 without relocations can load there. The entitlement also lets a
process run with 4K pages instead of the host's 16K pages, and gates the
hardware x86 memory ordering FEX runs x86 code with (`FEX_HARDWARE_TSO`,
[environment variables](../README.md#environment-variables)). That
entitlement is honoured only through a provisioning profile embedded in an
app bundle and a signature by a certificate of the profile's team. Wine
does not run without it.

- Before Wine sets up its address space, the loader execs itself with 4K
  pages (`exec_with_4k_pages` in
  [`loader/main.c`](https://github.com/athei/wine/blob/arm64-patched/loader/main.c)).
  ntdll starts new Wine processes with 4K pages directly (`preloader_exec` in
  [`dlls/ntdll/unix/loader.c`](https://github.com/athei/wine/blob/arm64-patched/dlls/ntdll/unix/loader.c)),
  so they need no exec for the page size. Everything inside a Wine process,
  FEX included, then runs on 4K pages. Wine does not merge 4K Windows pages
  into 16K host pages, so page protections, guard pages, decommit and FEX's
  tracking of self-modifying code work on 4K pages, as on Windows. The
  wineserver and other plain Unix programs keep the default page size.
- When the kernel placed the loader too close to 4 GB to leave room for such
  an executable, the loader execs itself again for a new placement, keeping
  its 4K pages, a bounded number of times. This applies to every loader,
  whether ntdll or something else started it. If such an exec fails, the
  loader prints a warning and keeps its placement. The threshold and the
  limit are in
  [`loader/main.c`](https://github.com/athei/wine/blob/arm64-patched/loader/main.c).
- A loader without the entitlement would be killed in that exec without a
  message, so the loader checks for the entitlement first. Without it, the
  loader exits with an error saying it must run from the signed `wine.app`.
  There is no fallback to 16K pages.
- The bundle step moves the loader into `lib/wine/aarch64-unix/wine.app` and
  links both entry points to it: `lib/wine/aarch64-unix/wine`, which ntdll
  execs, and `bin/wine`.
- The app is named Mach9, the launcher this Wine runs under, and carries the
  games category and the Game Mode keys of Mach9's own `Info.plist`, so a
  game the launcher starts gets Game Mode. macOS takes the name in the app
  menu from the bundle of the running executable, which is `wine.app`. The
  bundle identifier stays the one the profile grants.
- The signature claims only what the loader needs: the application and team
  identifiers and the cross-architecture entitlement, all taken from the
  profile, the self-granted `com.apple.security.custom-x18-abi-toggle`, and
  the entitlements the hardened runtime needs for Wine and FEX, which
  `LOADER_RUNTIME_ENTITLEMENTS` in [`bundle-wine.sh`](../bin/bundle-wine.sh)
  lists with the reason for each. The bundle identifier is the application
  identifier without the team prefix. The identity of the profile's team in
  the keychain signs, passed by SHA-1.
- The app is signed once as a whole, without `--deep`, with the hardened
  runtime and a secure timestamp, as notarization and Mach9's install
  verification require. The timestamp needs Apple's timestamp server;
  `LOADER_TIMESTAMP=0` signs without one for an offline local build, and the
  bundle step reports which was used. Such a bundle cannot be notarized.
- `--compatdb-only` replaces `compatdb.so` outside the app, so the signature
  stays valid.

## What the bundle step checks

Before anything is deleted, the preflight checks the macOS SDK, the
formulae, the Wine SDK (staged from the current Wine source and, for a full
bundle, holding every [graphics test](#graphics-tests)), the FEX build
(complete, and built from the commit `fex-src.ref` names now, which it asks
`FEX_REPO` for with `git ls-remote`, so the step needs the network), the
staged DXMT (complete, from the pinned commit, linked against the current
Wine source, its DLLs carrying the recorded version), the staged Vulkan
loader and KosmicKrisp (complete, Mesa built from the commit `mesa-src.ref`
names now, asked of `MESA_REPO` the same way, and the loader from the
pinned commit), the llvm-mingw compilers and tools the smoke test and Step
4e use, and the provisioning profile (not expired, granting the
entitlement, with an identity of its team in the keychain).

After assembly:

- Inherited `com.apple.quarantine` is removed from the tree (and nothing
  else), so macOS does not block a copied library; a failure stops the
  bundle step.
- No dylib in `lib/external/`, no `.so` module, the tree unix modules
  included, and not the loader references any of the `LEAK_PREFIXES` in
  `profile.sh`.
- The Direct3D trees, FEX, the Vulkan files, the loader app and, in a full
  bundle, the x86_64 link libraries and every graphics test are in place;
  the mtld3d tree's
  `d3d9.dll` is byte-identical to mtld3d's, and `xtajit.dll` and
  `xtajit64.dll` are FEX's.
- No PE file has `.debug_*` sections or a COFF symbol table; Step 4e
  stripped them, keeping the unstripped files in `wine-symbols` next to the
  bundle with `--symbols`.
- No symlink in the tree dangles.
- The Vulkan loader, KosmicKrisp, its manifest and Wine's Vulkan modules
  are in place, the manifest's `library_path` resolves from its directory,
  the loader and the driver export their entry points, and `ntdll.so`
  names `VK_DRIVER_FILES`.
- `wine.app` verifies with `codesign --verify --strict`, is signed by the
  expected identity and team with the hardened runtime and, unless
  `LOADER_TIMESTAMP=0`, a secure timestamp, and carries exactly the
  entitlements above and the Mach9 keys in its `Info.plist`.
- The minimum macOS (`minos`) of every executable, `.dylib` and `.so` (the
  newest slice of a universal file) is at most the floor, see
  [macOS floors](maintenance.md#macos-floors). Without `--strict-floor`
  each file over the floor is a warning; with it, an error. A missing
  `otool`, or a run in which no file had a `minos`, is an error either way.
- No unix module, loader or `wineserver` imports a function of
  `FLOOR_NEWER_FUNCS` while the floor is below its version (see
  [macOS floors](maintenance.md#macos-floors)). That catches a Wine build
  tree configured before the override, which `minos` does not show.
- The smoke test, with the caller's `WINE_COMPATDB` and `WINE_D3D_CONFIG`
  and every `WINEMSYNC*`, `FEX_*`, `VK_*`, `MESA_*`, `MTL_*` and `METAL_*`
  variable dropped: `wine --version`; a fresh prefix boots
  (`cmd /c exit`), with `compatdb:` lines and the wineserver's
  `msync: up and running.` line and no `msync: error:` line (with
  `--compatdb-only`, a wineserver without msync skips that check);
  winedbg's crash dialog is turned off in the prefix (`ShowCrashDialog`),
  so a program that crashes exits with a backtrace in the log instead of
  waiting for the time limit; the i386 `cmd.exe` runs; and
  [`smoke/hello.c`](../smoke/hello.c), compiled for each architecture in
  `SMOKE_PE_ARCHS`, prints its marker and reports whether it
  loads `dxgi.dll` and `d3d9.dll`. Each one's compatdb lines must name the
  trees for its arch (the x86 defaults, or `wined3d` for the native one),
  and the x86 programs must load both DLLs; the native one cannot (see
  [Limitations](#limitations)). Then the x86_64 and i686 programs load
  every module of each Direct3D tree in turn, the tree picked with a
  `WINE_COMPATDB` rule, which shows that the prefix placeholders made from
  the default-dir markers let each tree in. Last,
  [`smoke/vulkan.c`](../smoke/vulkan.c), compiled for x86_64 and i686,
  creates a Vulkan instance without a window and must find a device; its
  compatdb lines must carry `MESA_KK_EXPERIMENTAL`. Then
  [`smoke/wined3d.c`](../smoke/wined3d.c), compiled for x86_64 and i686,
  must create a device for a hidden window through wined3d twice, with the
  `winediag` channel on: with the default renderer, where wined3d must not
  report the Vulkan renderer, and with a `wined3d=vulkan` rule for the
  program, where it must. The i686 program uses Direct3D 8. Wine builds no
  64-bit `d3d8.dll`, so the x86_64 one uses Direct3D 9, with a rule giving
  it the `wined3d` d3d9 tree in both runs. Every run has a time limit.

With `--compatdb-only`, the bundle step:

- refuses a bundle that lacks the Vulkan loader, KosmicKrisp or its
  manifest, such as one from before this build had Vulkan, and names what
  is missing. compatdb.so does not disable `vulkan-1`, so in such a bundle
  a game's own `vulkan-1.dll` would load and find no driver, and the smoke
  test's Vulkan checks need Vulkan anyway. Such a bundle needs a full
  bundle step;
- builds `compatdb.so` and replaces the installed copy by copy and rename,
  so a running process keeps a valid mapping;
- removes quarantine from `compatdb.so` only, and only when it is there;
- checks that the installed file matches the build output, the
  `LEAK_PREFIXES` references and `minos` of `compatdb.so` only (the rest
  was not built here), and runs the smoke test.

## Homebrew libraries and the floor

The libraries in `lib/external/` other than the Vulkan loader and
KosmicKrisp are Homebrew's: the formulae in `DEPS_FORMULAE` and their
runtime closure, copied from `DEPS_PREFIX`. Homebrew builds each bottle for
one macOS major version, the bottle tag, with the deployment target at that
version's `.0` release, and pours the bottle of the host's version. The
host therefore decides whether those libraries meet the floor:

- On a host of the floor's major version, every bottle's `minos` is below
  the floor. A CI or release build has to run on such a host, takes the
  libraries from Homebrew and passes `--strict-floor`, so a file above the
  floor fails the bundle step.
- On a newer macOS, Homebrew pours that version's bottles, and the bundle
  step warns about each of them. That is expected for a local build, which
  is not distributed.
- A formula with no bottle for the host's version is compiled on the host
  and carries the host's full version as `minos`, which can be above the
  floor even on a host of its major version. `--strict-floor` stops such a
  bundle.
- Before moving such a build to another macOS version, check the bottles of
  that version: the formula's JSON at
  `formulae.brew.sh/api/formula/<name>.json` lists the bottle of every tag.
  `brew fetch --bottle-tag` may not know other tags, since brew's own API
  data can carry the host's tag only. Unpack each bottle and read its
  dylibs' `minos` with `vtool -show-build`.

Nothing from Homebrew is linked statically into a file the bundle ships.
Wine's unix modules load the libraries above by soname and link none of
them. KosmicKrisp's driver is built with pkg-config searching nowhere, and
`build-mesa.sh` checks that it and the loader load system libraries only.
The static libraries that `winemetal.so` and the driver do link (DXMT's
LLVM, and DXMT's and Mesa's own archives) are built here for the floor. A
linked binary's `minos` does not show an object inside it that needs a
newer macOS; only the linker's warning that an object file `was built for
newer` macOS than the one being linked does. After adding a static
dependency, search a clean build's log for it.

## Limitations

- A native ARM64 program cannot load `dxgi.dll`, `d3d9.dll` or the other
  modules of the Direct3D trees; the loader fails with error 193
  (`ERROR_BAD_EXE_FORMAT`). The prefix's placeholder for each of those names
  comes from the fake-module marker in `aarch64-windows`, and the
  placeholder's machine decides which module Wine looks for. Every 64-bit
  marker is an x64 one, even where the module behind it is one of Wine's
  ARM64X modules (`d3d10.dll` and `d3d10_1.dll` in every tree), because x64
  games are what the bundle is for and an x64 process needs them: with an
  ARM64 marker the native program loads Wine's ARM64X module and the x64
  program fails with the same error instead. compatdb.so still gives a
  native ARM64 program the `wined3d` trees
  ([native ARM64 programs](compatdb.md#native-arm64-programs)), the only
  ones with ARM64 code, so the choice is right once a placeholder lets it
  through.

## Pinned inputs

| Input | Pinned in |
|-------|-----------|
| FEX | [`fex-src.ref`](../pins/fex-src.ref): a branch, tag or commit of `FEX_REPO` |
| mtld3d | [`redist.env`](../pins/redist.env), URL and SHA-256 of the release tarball |
| DXMT | [`redist.env`](../pins/redist.env), by commit |
| LLVM for DXMT's `winemetal.so` | [`redist.env`](../pins/redist.env), tag and commit; `build-dxmt.sh` fails when the tag is not at the commit |
| Mesa (KosmicKrisp) | [`mesa-src.ref`](../pins/mesa-src.ref): a branch, tag or commit of `MESA_REPO` |
| Vulkan loader and headers | [`redist.env`](../pins/redist.env), tag and commit of each, from one SDK release; `build-mesa.sh` fails when a tag is not at its commit |
| Python modules of Mesa's build scripts | [`redist.env`](../pins/redist.env), as pip requirements with exact versions |
| Wine | Whatever `WINE_SRC` has checked out, normally [`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched); nothing pins a commit. The SDK and DXMT record the commit they were built against (with `-dirty` when tracked files differ, which counts all uncommitted states as one), and the bundle step refuses either when it is not the current one |
| Homebrew libraries | Not pinned: whatever the host's Homebrew has ([Homebrew libraries and the floor](#homebrew-libraries-and-the-floor)) |
| Provisioning profile | [`signing/loader.provisionprofile`](../signing/loader.provisionprofile); replace the file when it expires or the App ID changes |
