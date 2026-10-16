# Native arm64 build

`--arm64` builds and bundles a native arm64 Wine that runs x86 programs
through [FEX](https://github.com/FEX-Emu/FEX) instead of Rosetta. It is
local only: no workflow builds it, and the bundle is signed for the
machine's own developer team.

Every difference from the x86_64 build is a value or a flag of the `arm64`
profile in [`profile.sh`](../profile.sh); the scripts test the flags. In
short:

- The Wine source is upstream Wine with our own series on top, the branch
  [`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) of
  athei/wine, checked out in the arm64 worktree of the Wine checkout. Some
  of its commits come from Hadron, and it carries CrossOver's msync (see
  [Runtime](#runtime)). It is configured natively against the arm64
  Homebrew with the same allowlist as the x86_64 build, plus the options
  only that tree has (`CONFIGURE_FEATURES_EXTRA`) and the configure results
  the floor overrides (`FLOOR_NEWER_FUNCS`,
  [macOS floors](maintenance.md#macos-floors)).
- No-execute follows upstream Wine by default. The series does not carry
  the x86_64 build's no-execute change, which avoids a stall in Rosetta's
  fault handling; with no-execute off, FEX showed no such stall. Instead,
  `WINE_FORCE_NX` set to anything but `0` keeps no-execute permanently on,
  so the process cannot turn it off
  ([environment variables](../README.md#environment-variables)). With
  no-execute off, every readable page is executable, so FEX has to treat
  all writable memory as possible code and translates it again whenever it
  changes. For a program whose executable lacks the `NX_COMPAT` flag and
  that runs slowly for that reason, a compatdb rule sets the variable
  ([recipes](compatdb.md#recipes)).
- FEX is upstream FEX with our own commits on top, from
  [athei/FEX](https://github.com/athei/FEX) at the ref
  [`fex-src.ref`](../fex-src.ref) names.
- FEX replaces Rosetta, so there is no x87sidecar and compatdb.so has no x87
  query. Instead, compatdb's built-in `fex-x87-reduced-precision` rule asks
  FEX for faster x87 emulation
  ([defaults](../README.md#defaults-that-differ-from-wine),
  [arm64 Wine](compatdb.md#arm64-wine)).
- No D3DMetal: Apple ships it for x86_64 only. The trees the bundle carries
  are `DXGI_IMPLS` and `D3D9_IMPLS` in the profile.
- Wine's Vulkan is on, on KosmicKrisp, Mesa's Vulkan driver on Metal, and
  a Vulkan loader built for the floor (see [Vulkan](#vulkan)).
- No d3d9 test binaries.
- A [Wine SDK](#wine-sdk) is staged for the PE code built outside Wine.
- The loader is wrapped in a signed app bundle, and Wine processes run with
  4K pages (see [Loader app](#loader-app)).

## Requirements

- An Apple Silicon Mac and a macOS SDK at least as new as the profile's
  `MACOS_FLOOR`.
- The arm64 Homebrew at the profile's `DEPS_PREFIX` with the formulae in
  `DEPS_FORMULAE`, plus `cmake`, `ninja` and `meson`, and what the
  preflight of [`build-mesa.sh`](../build-mesa.sh) checks for, among it the
  formulae KosmicKrisp's build tools link.
- Xcode's Metal toolchain (`xcrun metal`), for DXMT's shaders.
- llvm-mingw at `MINGW_DIR`, as for the x86_64 build, including its arm64ec
  toolchain.
- Rust with the profile's `RUST_TARGET`.
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

```bash
./build-wine.sh --arm64    # Mesa and the Vulkan loader, Wine, the SDK, then FEX and DXMT
./bundle-wine.sh --arm64   # assemble the bundle
```

| Script | What it does |
|--------|--------------|
| `build-wine.sh --arm64` | Runs `build-mesa.sh`, builds Wine against the loader it staged, stages the [Wine SDK](#wine-sdk), then runs `build-fex.sh` and `build-dxmt.sh`; `--clean` is passed on to all three |
| [`build-mesa.sh`](../build-mesa.sh) | Checks out the ref in [`mesa-src.ref`](../mesa-src.ref) in `MESA_SRC` (refusing a tree with local changes), builds the Vulkan loader from the tags pinned in [`redist.env`](../redist.env) and KosmicKrisp, and stages both with the driver's manifest and their commits in `MESA_DIST` (see [Vulkan](#vulkan)). Runs alone too |
| [`build-fex.sh`](../build-fex.sh) | Checks out the ref in [`fex-src.ref`](../fex-src.ref) in `FEX_SRC` (refusing a tree with local changes), builds FEX's emulator DLLs and their unix halves, and records the commit. Runs alone too |
| [`build-dxmt.sh`](../build-dxmt.sh) | Builds DXMT from the commit pinned in [`redist.env`](../redist.env) against the Wine build and SDK, and stages it with its commit and `git describe` in `DXMT_DIST`. Runs alone too, after a Wine build |
| `bundle-wine.sh --arm64` | Assembles the bundle from the Wine build, the SDK, FEX, DXMT, the Vulkan loader and KosmicKrisp, and the mtld3d release, signs the loader and runs the arm64 smoke test |

### Paths

Every tree of this profile lives under `build/arm64/` and the bundle in
`dist/arm64/wine`, in the [layout](../README.md#layout) the x86_64 build
uses. The arm64 overrides have their own names, so a `WINE_SRC` or
`BUILD_DIR` exported for the x86_64 build cannot redirect this one. A
`MACOSX_DEPLOYMENT_TARGET` may raise the arm64 floor but not lower it.

| Variable | What it is | Default set in |
|----------|------------|----------------|
| `ARM64_WINE_SRC`, `ARM64_BUILD_DIR` | arm64 Wine source and build tree | [`profile.sh`](../profile.sh) |
| `ARM64_X86LIBS_BUILD_DIR` | The link-library tree for the x86_64 archives of the SDK | [`profile.sh`](../profile.sh) |
| `ARM64_WINE_SDK` | The [Wine SDK](#wine-sdk) | [`profile.sh`](../profile.sh) |
| `FEX_REPO`, `FEX_SRC` | FEX repository and the local clone `build-fex.sh` checks out | [`profile.sh`](../profile.sh) |
| `FEX_BUILD_DIR` | FEX's CMake trees | [`profile.sh`](../profile.sh) |
| `DXMT_BUILD_DIR` | DXMT's checkout, its meson trees (x86_64 and i386 PE, arm64 unix) and, by default, `DXMT_LLVM` and `DXMT_DIST` | [`profile.sh`](../profile.sh) |
| `DXMT_LLVM` | The LLVM static libraries `winemetal.so` links | [`profile.sh`](../profile.sh) |
| `DXMT_DIST` | All of DXMT as the bundle installs it, with the commits it was built from | [`profile.sh`](../profile.sh) |
| `MESA_REPO`, `MESA_SRC` | Mesa repository and the local clone `build-mesa.sh` checks out | [`profile.sh`](../profile.sh) |
| `MESA_BUILD_DIR` | The Vulkan loader's and headers' clones and trees, KosmicKrisp's two meson trees, the Python environment and, by default, `MESA_DIST` | [`profile.sh`](../profile.sh) |
| `MESA_DIST` | The Vulkan loader, KosmicKrisp and its manifest as the bundle installs them, with the commits they were built from | [`profile.sh`](../profile.sh) |
| `LOADER_PROFILE` | The provisioning profile the loader is signed for | [`profile.sh`](../profile.sh) |

## Layout differences

| Path | Contents |
|------|----------|
| `lib/wine/aarch64-unix/` | Wine's unix modules, `compatdb.so`, DXMT's `winemetal.so` and FEX's `lib*fex.so` |
| `lib/wine/aarch64-unix/wine.app/` | The loader, its `Info.plist` and the embedded provisioning profile |
| `lib/wine/aarch64-unix/wine`, `bin/wine` | Symlinks to `wine.app/Contents/MacOS/wine` |
| `lib/wine/aarch64-windows/xtajit.dll`, `xtajit64.dll` | FEX's WoW64 and ARM64EC DLLs under the names Wine loads by default; also present under their own names |
| `lib/wine/dxgi/<impl>/aarch64-windows/`, `lib/wine/d3d9/<impl>/aarch64-windows/` | The 64-bit modules of each tree. DXMT's and mtld3d's are x86_64 PE: an x64 process runs as ARM64EC, and Wine looks up its builtins in `aarch64-windows` |
| `lib/wine/x86_64-windows/` | Full bundle only: the SDK's x86_64 link libraries and nothing else, so the bundle carries the same link libraries as the SDK. Nothing reads the directory at run time |
| `lib/external/libvulkan.1.dylib`, `lib/external/libvulkan_kosmickrisp.dylib` | The Vulkan loader `win32u.so` loads, and KosmicKrisp |
| `share/vulkan/icd.d/` | KosmicKrisp's manifest, naming the driver by a path relative to the manifest |

## Runtime

Besides the variables of the x86_64 bundle
([environment variables](../README.md#environment-variables)), these apply
to this build only.

| Variable | Effect |
|----------|--------|
| `WINEMSYNC` | Unset or a non-zero number: msync is on. Any other value (`0`, empty, not a number): off, and Wine synchronizes through wineserver requests as upstream Wine does. Only the wineserver reads it, from the environment of the process that starts it, and every process of the session follows the wineserver's choice whatever its own environment says |
| `WINENCPU` | Limits the number of processors Wine reports to programs, for those that misbehave with many. A value of 0, one that is not a number, or one above the real count is ignored |
| `FEX_HARDWARE_TSO` | Unset or `1`: FEX runs x86 code with the hardware x86 memory ordering (TSO) instead of emulating it with ordered loads and stores. The ordering is a per-thread mode: FEX switches a thread to it whenever the thread enters x86 code and back at the syscalls and unix calls of x86 code, since native code runs slower in it. `0`: FEX emulates the ordering. Any other value logs an error and FEX emulates the ordering |
| `VK_DRIVER_FILES` | Unset: ntdll sets it to the bundle's `share/vulkan/icd.d`, so the Vulkan loader uses the bundled KosmicKrisp and no driver installed elsewhere on the Mac. Set: the loader uses the manifests it names instead. ntdll also leaves it unset when the caller sets `VK_ICD_FILENAMES` or `VK_ADD_DRIVER_FILES`; the loader then uses those, and the bundled driver is not added to them ([Vulkan](#vulkan)) |
| `MESA_KK_EXPERIMENTAL` | Which experimental features KosmicKrisp advertises. compatdb's built-in `kosmickrisp-experimental` rule sets it in every process, replacing a value from the environment; a rule changes or removes it ([Vulkan](#vulkan)) |
| `WINE_D3D_CONFIG` | wined3d's settings, as in Wine. `renderer=vulkan` puts wined3d on KosmicKrisp. compatdb's `wined3d=vulkan` sets it for one game, keeping the variable's other options; the programs that game starts inherit it unless a rule for them sets `wined3d` ([wined3d on Vulkan](compatdb.md#wined3d-on-vulkan)) |
| `FEX_HOSTFEATURES` | FEX's host feature overrides. x86_64 programs, which run as ARM64EC, run without AFP (the Arm floating-point behaviour FEX uses to match x86) by default, because the floating-point mode would have to change on every call between x86 code and native code, which costs more than AFP saves. `enableafp` turns it back on. i386 programs keep AFP |

- msync is CrossOver's implementation of Windows synchronization objects
  for macOS, built on Mach semaphores and shared memory, which saves
  wineserver round trips.
- `WINEMSYNC=0` has to be in the environment of whatever starts the
  wineserver session, normally the launcher's first Wine process, and
  takes effect only for a new session. A per-game compatdb `env` rule
  cannot turn msync off reliably: it reaches the game's process, which
  only starts the wineserver when no session is running yet.
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
`winebuild` to mark itself a builtin. `build-wine.sh --arm64` stages one
directory with all of it, in the layout of an installed Wine, for every
architecture that code is built for, plus a `WINE_COMMIT` file naming the
Wine source commit. What goes in is the `WITH_WINE_SDK` step of
[`build-wine.sh`](../build-wine.sh).

- `bin/winebuild` and the `aarch64-windows` and `i386-windows` archives come
  from the arm64 build itself. Its `aarch64-windows` archives are ARM64X,
  carrying ARM64 and ARM64EC members, since arm64ec is paired with aarch64
  there.
- The `x86_64-windows` archives are what the arm64 build cannot make: x64
  objects do not link against ARM64EC ones. A link-library tree, a second
  configure of the same source for x86_64 alone that reuses the build's
  tools, builds just those.
- DXMT's PE build reads it through `-Dwine_install_path` (see
  [`build-dxmt.sh`](../build-dxmt.sh)).
- A full bundle carries the same files, so the bundle is an installed Wine
  with its SDK. [mtld3d](https://github.com/athei/mtld3d) builds against
  installed Wines, not the SDK directory: the x86_64 profile's
  `dist/intel/wine` and this profile's `dist/arm64/wine`. Its
  [`docs/BUILDING.md`](https://github.com/athei/mtld3d/blob/main/docs/BUILDING.md)
  names the variables and lists what else that build needs.
- `build-dxmt.sh` and the bundle step refuse an SDK whose `WINE_COMMIT` is not
  the Wine source's current commit.

## DXMT

- DXMT is built from the upstream commit pinned in
  [`redist.env`](../redist.env), not taken from a release: the release's
  Metal shaders do not compile with the Metal toolchain this build uses, and
  no later release exists. The comment there says which fix the pin carries.
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
  static libraries from the tag pinned in [`redist.env`](../redist.env) into
  `DXMT_LLVM`, records the commit there, and rebuilds them only when the pin
  changes; `--clean` keeps them.

## Vulkan

Wine's Vulkan (`vulkan-1.dll`, `winevulkan`) runs on KosmicKrisp, Mesa's
Vulkan driver on Metal, through the Khronos Vulkan loader. The default
Direct3D implementations, mtld3d and DXMT, go to Metal directly and do not
use it; wined3d can, per game (see below).

- Mesa is [athei/mesa](https://github.com/athei/mesa) at the ref
  [`mesa-src.ref`](../mesa-src.ref) names: upstream Mesa plus our topic
  commits, originally from
  [Hadron's Mesa patches](https://github.com/Broyojo/hadron/tree/main/patches/mesa)
  and regrouped by topic. Commits of upstream merge requests that are not
  merged yet are carried under their original authors and dropped once
  upstream has them
  ([rebasing](maintenance.md#mesa-and-the-vulkan-loader-arm64)). Like FEX,
  the ref is fetched from `MESA_REPO`, so a commit that is only in the local
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
  Vulkan-Headers tags pinned in [`redist.env`](../redist.env), because
  Homebrew's vulkan-loader bottle needs a newer macOS than the floor. The
  bundle step copies it from `MESA_DIST` into `lib/external/`, which is
  what keeps Homebrew's loader out of the bundle: `SONAME_LIBVULKAN` holds
  only the file name, whichever loader configure linked. `MESA_DIST/lib`
  comes first in the profile's `LDFLAGS` so that configure's `-lvulkan`
  test succeeds against this loader, without Homebrew's installed, and
  does not link Homebrew's where it is.
- `win32u.so` loads the loader the first time something in the process
  needs Vulkan. That includes the display device
  refresh `explorer.exe` runs when a session starts: it loads the loader
  and KosmicKrisp and takes the GPU's name, memory size and UUID for the
  registry from them.
- The loader finds the driver through the manifest in
  `share/vulkan/icd.d/`, whose `library_path` is relative to the manifest
  (`../../../lib/external/...`), so the bundle stays relocatable. ntdll sets
  `VK_DRIVER_FILES` to that directory when the caller has set none of
  `VK_DRIVER_FILES`, `VK_ICD_FILENAMES` and `VK_ADD_DRIVER_FILES`, so the
  loader uses the bundled driver and no other driver installed on the Mac.
  A caller that sets one of them chooses the drivers itself: the bundled
  one is then not added, and has to be named there to be used
  ([Runtime](#runtime)).
- Our topic commits add features KosmicKrisp advertises only when
  `MESA_KK_EXPERIMENTAL` names them. compatdb's built-in
  `kosmickrisp-experimental` rule sets the variable in every process, with
  the flags in [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs). As
  with every `env` rule in this Wine, compatdb.so sets it when it loads,
  before any PE code runs, so it is there before win32u loads the Vulkan
  loader and the driver reads it. Mesa describes the variable in
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
- compatdb's `no-vulkan` rule, which disables `vulkan-1` in the x86_64
  Wine, does not exist here.
- Wine's own `d3d12.dll` and `d3d12core.dll` are left out of the bundle,
  as in the x86_64 build.
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
[Runtime](#runtime)). That entitlement is honoured only through a
provisioning profile embedded in an app bundle and a signature by a
certificate of the profile's team. Wine does not run without it.

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
  links both entry points to it.
- The app is named Mach9, the launcher this Wine runs under, and carries the
  games category and the Game Mode keys of Mach9's own `Info.plist`, so a
  game the launcher starts gets Game Mode. macOS takes the name in the app
  menu from the bundle of the running executable, which is `wine.app`. The
  bundle identifier stays the one the profile grants.
- The signature claims only what the loader needs: the application and team
  identifiers and the cross-architecture entitlement, all taken from the
  profile, the self-granted `com.apple.security.custom-x18-abi-toggle`, and
  the entitlements the hardened runtime needs for Wine and FEX, which
  `LOADER_RUNTIME_ENTITLEMENTS` in [`bundle-wine.sh`](../bundle-wine.sh)
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
formulae, the Wine SDK (staged from the current Wine source), the FEX build
(complete, and built from the commit `fex-src.ref` names now, which it asks
`FEX_REPO` for with `git ls-remote`, so the step needs the network), the
staged DXMT (complete, from the pinned commit, linked against the current
Wine source, its DLLs carrying the recorded version), the staged Vulkan
loader and KosmicKrisp (complete, Mesa built from the commit `mesa-src.ref`
names now, asked of `MESA_REPO` the same way, and the loader from the
pinned commit), the llvm-mingw compilers the smoke test uses, and the
provisioning profile (not expired, granting the entitlement, with an
identity of its team in the keychain).

After assembly, besides the self-containment, `minos`, floor-newer import,
symlink and copied-in file checks of the x86_64 bundle
([maintenance.md](maintenance.md#what-bundle-winesh-verifies)):

- No reference to any of the profile's `LEAK_PREFIXES` survives.
- `wine.app` verifies with `codesign --verify --strict`, is signed by the
  expected identity and team with the hardened runtime and, unless
  `LOADER_TIMESTAMP=0`, a secure timestamp, and carries exactly the
  entitlements above and the Mach9 keys in its `Info.plist`.
- FEX's DLLs and `.so` files are in place, and `xtajit.dll` and
  `xtajit64.dll` are FEX's.
- No `bin/x87sidecar`, no x87 export in `compatdb.so` or `ntdll.so`, and
  `compatdb.so` exports `compatdb_query_env`.
- The Vulkan loader, KosmicKrisp, its manifest and Wine's Vulkan modules
  are in place, the manifest's `library_path` resolves from its directory,
  the loader and the driver export their entry points, and `ntdll.so`
  names `VK_DRIVER_FILES`.
- The smoke test, with the caller's `WINE_COMPATDB` and `WINE_D3D_CONFIG`
  and every `WINEMSYNC*`, `FEX_*`, `VK_*`, `MESA_*`, `MTL_*` and `METAL_*`
  variable dropped: `wine --version`; a fresh prefix boots
  (`cmd /c exit`), with `compatdb:` lines, no trace of the x87sidecar, and
  the wineserver's `msync: up and running.` line and no `msync: error:`
  line (with `--compatdb-only`, a wineserver without msync skips that
  check); winedbg's crash dialog is turned off in the prefix
  (`ShowCrashDialog`), so a program that crashes exits with a backtrace in
  the log instead of waiting for the time limit; the i386 `cmd.exe` runs;
  and
  [`smoke/hello.c`](../smoke/hello.c), compiled for each architecture in
  the profile's `SMOKE_PE_ARCHS`, prints its marker and reports whether it
  loads `dxgi.dll` and `d3d9.dll`. Each one's compatdb lines must name the
  trees for its arch (the x86 defaults, or `wined3d` for the native one), and
  the x86 programs must load both DLLs; the native one cannot (see
  [Limitations](#limitations)). Then the x86_64 and i686 programs load every
  module of each Direct3D tree in turn, as for the x86_64 bundle. Last,
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

`--compatdb-only` refuses a bundle that lacks the Vulkan loader,
KosmicKrisp or its manifest, such as one from before this build had
Vulkan, and names what is missing. The arm64 library has no `no-vulkan`
rule, so in such a bundle a game's own `vulkan-1.dll` would load and find
no driver, and the smoke test's Vulkan checks need Vulkan anyway. Such a
bundle needs a full bundle step.

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
  ([arm64 Wine](compatdb.md#arm64-wine)), the only ones with ARM64 code, so
  the choice is right once a placeholder lets it through.

## Pinned inputs

| Input | Pinned in |
|-------|-----------|
| FEX | [`fex-src.ref`](../fex-src.ref): a branch, tag or commit of `FEX_REPO` |
| mtld3d | The same release tarball as the x86_64 build, [`redist.env`](../redist.env) |
| DXMT | [`redist.env`](../redist.env), by commit |
| LLVM for DXMT's `winemetal.so` | [`redist.env`](../redist.env), tag and commit; `build-dxmt.sh` fails when the tag is not at the commit |
| Mesa (KosmicKrisp) | [`mesa-src.ref`](../mesa-src.ref): a branch, tag or commit of `MESA_REPO` |
| Vulkan loader and headers | [`redist.env`](../redist.env), tag and commit of each, from one SDK release; `build-mesa.sh` fails when a tag is not at its commit |
| Python modules of Mesa's build scripts | [`redist.env`](../redist.env), as pip requirements with exact versions |
| Wine | Whatever the arm64 worktree has checked out, normally [`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched); nothing pins a commit. The SDK and DXMT record the commit they were built against (with `-dirty` when tracked files differ, which counts all uncommitted states as one), and the bundle step refuses either when it is not the current one |
| Provisioning profile | [`signing/loader.provisionprofile`](../signing/loader.provisionprofile); replace the file when it expires or the App ID changes |
