# wine-build

Builds a native arm64 Wine for macOS from
[athei/wine](https://github.com/athei/wine), upstream Wine with our own
series, and bundles it into a self-contained, relocatable `wine/` tree. x86
programs, i386 and x86_64, run under [FEX](https://github.com/FEX-Emu/FEX)
in the same process. Direct3D goes through per-process selectable
implementations (DXMT, mtld3d, wined3d), chosen by `compatdb.so`, a library
built from [`compatdb/`](compatdb).

- [docs/arm64.md](docs/arm64.md): how the build works: the sources, FEX,
  DXMT, Vulkan, the signed loader app, what the bundle step checks.
- [docs/compatdb.md](docs/compatdb.md): rules, `WINE_COMPATDB`, recipes,
  logging.
- [docs/maintenance.md](docs/maintenance.md): internals, floors, pins,
  upgrade checklist.

## Getting a build

The bundle is built locally with the commands under [Building](#building).
No workflow publishes it yet. Its loader is signed by a certificate of the
team that the provisioning profile in [`signing/`](signing) belongs to
([loader app](docs/arm64.md#loader-app)).

The `cx-*` and `compatdb-*`
[releases](https://github.com/athei/wine-build/releases) are the x86_64
Wine this repository built before, run under Rosetta. They are frozen and
kept for comparison.

The minimum macOS is `MACOS_FLOOR` in [`profile.sh`](bin/lib/profile.sh).

## Bundle layout

| Path | Contents |
|------|----------|
| `bin/` | What `make install` creates; `bin/wine` is a symlink to the loader in `wine.app` |
| `lib/wine/aarch64-windows/`, `lib/wine/i386-windows/` | Wine's PE modules, FEX's emulator DLLs (also as `xtajit.dll` and `xtajit64.dll`, the names Wine loads), and fake-module markers for the Direct3D DLLs |
| `lib/wine/aarch64-unix/` | Wine's unix modules, `compatdb.so`, DXMT's `winemetal.so` and FEX's unix halves |
| `lib/wine/aarch64-unix/wine.app/` | The loader, its `Info.plist` and the embedded provisioning profile ([loader app](docs/arm64.md#loader-app)) |
| `lib/wine/dxgi/<impl>/`, `lib/wine/d3d9/<impl>/` | One tree per Direct3D implementation (see [Direct3D backends](#direct3d-backends)) |
| `lib/wine/x86_64-windows/` | Full bundle only: the x86_64 link libraries of the [Wine SDK](docs/arm64.md#wine-sdk). Nothing reads the directory at run time |
| `lib/wine/tests/i386-windows/`, `lib/wine/tests/x86_64-windows/` | Full bundle only: Wine's graphics conformance tests as `<name>_test.exe`, for testing the bundle's Direct3D, OpenGL and Vulkan stack ([graphics tests](docs/arm64.md#graphics-tests)) |
| `lib/wine/tests/VERSION` | Full bundle only: one line, the `wine --version` string of the Wine the tests were built from, the same as the bundle's own `bin/wine --version` |
| `lib/external/` | Bundled dylibs, the Vulkan loader and KosmicKrisp |
| `share/vulkan/icd.d/` | KosmicKrisp's manifest, naming the driver by a path relative to the manifest |

An x64 process runs as ARM64EC, and Wine looks up its builtins in
`aarch64-windows`, so the 64-bit modules of every Direct3D tree live under
`<impl>/aarch64-windows/`. They are all ARM64X, so the default-dir markers
of their names are hybrid ones, which x64 and native ARM64 processes both
accept, and the bundle step fails on any file in an `aarch64-windows`
directory that an x64 process would load but not run natively
([Direct3D trees](docs/maintenance.md#direct3d-trees)).

## Runtime configuration

### Environment variables

| Variable | Effect |
|----------|--------|
| `WINE_COMPATDB` | Extra or changed compatdb rules for the whole process tree. Format: [docs/compatdb.md](docs/compatdb.md#wine_compatdb) |
| `WINE_FORCE_NX` | Unset or `0`: Wine's no-execute behavior. Any other value: no-execute is permanently on, and the process cannot turn it off |
| `WINEDLLOVERRIDES` | Works as in Wine, but cannot undo an override compatdb adds; use a rule ([recipes](docs/compatdb.md#recipes)) |
| `WINEMSYNC` | Unset or a non-zero number: msync is on. Any other value (`0`, empty, not a number): off, and Wine synchronizes through wineserver requests as upstream Wine does. Only the wineserver reads it ([runtime](docs/arm64.md#runtime)) |
| `WINENCPU` | Limits the number of processors Wine reports to programs, for those that misbehave with many. A value of 0, one that is not a number, or one above the real count is ignored ([runtime](docs/arm64.md#runtime)) |
| `FEX_HARDWARE_TSO` | Unset or `1`: FEX runs x86 code with the hardware x86 memory ordering (TSO) instead of emulating it with ordered loads and stores. The ordering is a per-thread mode: FEX switches a thread to it whenever the thread enters x86 code and back at the syscalls and unix calls of x86 code, since native code runs slower in it. `0`: FEX emulates the ordering. Any other value logs an error and FEX emulates the ordering |
| `FEX_HOSTFEATURES` | FEX's host feature overrides. x86_64 programs, which run as ARM64EC, run without AFP (the Arm floating-point behaviour FEX uses to match x86) by default, because the floating-point mode would have to change on every call between x86 code and native code, which costs more than AFP saves. `enableafp` turns it back on. i386 programs keep AFP |
| `VK_DRIVER_FILES` | Unset: ntdll sets it to the bundle's `share/vulkan/icd.d`, so the Vulkan loader uses the bundled KosmicKrisp and no driver installed elsewhere on the Mac. Set: the loader uses the manifests it names instead. ntdll also leaves it unset when the caller sets `VK_ICD_FILENAMES` or `VK_ADD_DRIVER_FILES`; the loader then uses those, and the bundled driver is not added to them ([Vulkan](docs/arm64.md#vulkan)) |
| `MESA_KK_EXPERIMENTAL` | Which experimental features KosmicKrisp advertises. compatdb's built-in `kosmickrisp-experimental` rule sets it in every process, replacing a value from the environment; a rule changes or removes it ([Vulkan](docs/arm64.md#vulkan)) |
| `WINE_D3D_CONFIG` | wined3d's settings, as in Wine. `renderer=vulkan` puts wined3d on KosmicKrisp. compatdb's `wined3d=vulkan` sets it for one game, keeping the variable's other options; the programs that game starts inherit it unless a rule for them sets `wined3d` ([wined3d on Vulkan](docs/compatdb.md#wined3d-on-vulkan)) |

### Direct3D backends

| Family | APIs | Rule value | Tree | Default for |
|--------|------|------------|------|-------------|
| `dxgi` | D3D10, D3D11 | `dxmt` ([DXMT](https://github.com/3Shain/dxmt)) | `lib/wine/dxgi/dxmt/` | i386, x86_64, native ARM64 |
| `dxgi` | D3D10, D3D10.1, D3D11 | `wined3d` | `lib/wine/dxgi/wined3d/` | none |
| `d3d9` | D3D9 | `mtld3d` ([mtld3d](https://github.com/athei/mtld3d)) | `lib/wine/d3d9/mtld3d/` | i386, x86_64, native ARM64 |
| `d3d9` | D3D9 | `wined3d` | `lib/wine/d3d9/wined3d/` | none |

- compatdb.so prepends one tree per family to every process's builtin search
  path, so a process sees one coherent set of modules. To pick a tree per
  game, see [recipes](docs/compatdb.md#recipes).
- D3D10 and D3D11 form one family because they create their device through
  one `dxgi.dll` and only work with their own implementation's copy.
- DXMT's and mtld3d's 64-bit modules are ARM64X: an x64 game loads their
  ARM64EC half and a native ARM64 program their ARM64 half, so both run
  natively in either, and only their i386 modules run under FEX with the
  game ([DXMT](docs/arm64.md#dxmt), [mtld3d](docs/arm64.md#mtld3d)).
- A native ARM64 program gets the same defaults, DXMT for `dxgi` and
  mtld3d for `d3d9`, and a rule can give it either tree of each family
  ([native ARM64 programs](docs/compatdb.md#native-arm64-programs)). It
  can load a Direct3D name only where its marker is a hybrid one, which
  every name has while every tree is ARM64X
  ([limitations](docs/arm64.md#limitations)).
- D3D12 is not available: the bundle leaves out Wine's own D3D12. D3D8 and
  DirectDraw stay on wined3d. Which modules each tree holds is in Step 4 of
  [`bundle-wine.sh`](bin/bundle-wine.sh).
- wined3d draws through OpenGL. The compatdb setting `wined3d` can put it
  on Vulkan for one game; it applies to every wined3d user in the process
  ([wined3d on Vulkan](docs/compatdb.md#wined3d-on-vulkan)).

### Defaults that differ from Wine

Each row decided by a rule is a built-in compatdb rule that matches every
process and can be changed per game or globally. Synchronization is decided
by the patched Wine.

| Behavior | Default in this bundle | Decided by |
|----------|------------------------|------------|
| DPI awareness | Every process is system-aware | Rule `dpi-aware` ([recipes](docs/compatdb.md#recipes)) |
| Address space | Every 32-bit process gets 4 GB, even without the large-address-aware flag | Rule `large-address-aware` ([recipes](docs/compatdb.md#recipes)) |
| x87 precision under FEX | FEX emulates every program's x87 code with reduced precision | Rule `fex-x87-reduced-precision` ([FEX and KosmicKrisp](docs/compatdb.md#fex-and-kosmickrisp)) |
| Synchronization | CrossOver's msync is on: synchronization objects use Mach semaphores and shared memory | wineserver; `WINEMSYNC=0` where the session starts turns it off ([runtime](docs/arm64.md#runtime)) |
| KosmicKrisp's experimental features | Advertised to every process: the ones the rule names in `MESA_KK_EXPERIMENTAL` | Rule `kosmickrisp-experimental` ([FEX and KosmicKrisp](docs/compatdb.md#fex-and-kosmickrisp)) |
| Mono and Gecko | Disabled (`mscoree,mshtml=`): no install prompt, and an installed wine-mono, Gecko or native .NET stays disabled too | Rule `no-mono-gecko` ([recipes](docs/compatdb.md#recipes)) |

## Building

### Requirements

What the build needs is in [docs/arm64.md](docs/arm64.md#requirements): an
Apple Silicon Mac, the arm64 Homebrew with the formulae `profile.sh` names,
Xcode's Metal toolchain, llvm-mingw, Rust with the `aarch64-apple-darwin`
target, for `compatdb.so`, and the signing identity of the loader's
provisioning profile.

### Commands

```bash
bin/build-wine.sh    # Mesa and the Vulkan loader, Wine, the SDK, then FEX and DXMT
bin/bundle-wine.sh   # assemble the bundle (see Layout)
```

| Script | Flag | Effect |
|--------|------|--------|
| `build-wine.sh` | none | Incremental build; configure runs only when the build directory has no Makefile, and in the x86 tree of the [Wine SDK](docs/arm64.md#wine-sdk) also when its configure arguments or `WINE_SRC` (the source path) changed |
| `build-wine.sh` | `--clean` | Wipes the build directories first, and is passed on to `build-mesa.sh`, `build-fex.sh` and `build-dxmt.sh`; the only way to pick up changed configure flags or deployment target, except in the x86 tree of the [Wine SDK](docs/arm64.md#wine-sdk), which is configured again whenever its flags or `WINE_SRC` change |
| `bundle-wine.sh` | `--dest <dir>` | Writes `<dir>/wine` instead of the bundle in the [layout](#layout). Either way it asks before it deletes an existing one, unless `--yes` |
| `bundle-wine.sh` | `--yes` | Deletes an existing bundle without asking, for an unattended run. A directory that is not empty and has no `bin/wine` is still refused |
| `bundle-wine.sh` | `--runtime-only` | No development files; the flavor to put into an application |
| `bundle-wine.sh` | `--compatdb-only` | Rebuilds `compatdb.so` into the existing bundle only (the default one, or `<dir>/wine` with `--dest`); no prompt, no `make install`, no Wine build needed; `--runtime-only` is ignored. It refuses a bundle without Vulkan ([what the bundle step checks](docs/arm64.md#what-the-bundle-step-checks)) |
| `bundle-wine.sh` | `--strict-floor` | A Mach-O that needs a newer macOS than the floor is an error instead of a warning ([Homebrew libraries and the floor](docs/arm64.md#homebrew-libraries-and-the-floor)) |
| `bundle-wine.sh` | `--symbols` | Also keeps the PE files as they were before their symbols were stripped, in `wine-symbols` next to the bundle |

`usage()` and the argument parsing in
[`build-wine.sh`](bin/build-wine.sh) and
[`bundle-wine.sh`](bin/bundle-wine.sh) are authoritative.
`build-mesa.sh`, `build-fex.sh` and `build-dxmt.sh` also run alone
([commands](docs/arm64.md#commands)).

### Repository

| Path | What it holds |
|------|---------------|
| [`bin/`](bin) | The build and bundle scripts; [`bin/lib/profile.sh`](bin/lib/profile.sh) holds the settings and helpers they share |
| [`pins/`](pins) | The pinned inputs: [`redist.env`](pins/redist.env) and the refs of the sources the build checks out |
| [`compatdb/`](compatdb) | The Cargo workspace of `compatdb.so` and the `compatdb-table` crate ([docs/compatdb.md](docs/compatdb.md)); run cargo there |
| [`smoke/`](smoke) | The programs the bundle step's smoke test builds and runs |
| [`signing/`](signing) | The loader's provisioning profile |
| [`docs/`](docs) | The documentation this README links to |

### Layout

Everything lives beside this repository, in the directory that holds it
(the root; `WINE_BUILD_ROOT` names another, for a checkout kept elsewhere):

| Path | What it is |
|------|------------|
| `scripts/` | The checkout of this repository, [wine-build](https://github.com/athei/wine-build), named `scripts` locally |
| `src/` | The Wine source, a checkout of athei/wine |
| `src-fex/` | The FEX checkout |
| `src-mesa/` | The Mesa checkout, for the Vulkan driver |
| `cache/` | Downloaded redistributables |
| `build/<component>/` | One tree per intermediate: the Wine build, its SDK and the x86 tree for its link libraries and tests, FEX, DXMT and Mesa with the Vulkan loader |
| `dist/wine` | The bundle |
| `dist/wine-symbols` | With `--symbols`, the bundle's PE files before their symbols were stripped (Step 4e of [`bundle-wine.sh`](bin/bundle-wine.sh)); not part of the bundle |

The name of each tree is set in [`profile.sh`](bin/lib/profile.sh),
with the variable that overrides it ([paths](docs/arm64.md#paths)). The
scripts mark each build tree they make with a `.wine-build-tree` file and
refuse to fill or delete a directory that is not empty and has no such
marker, so an override cannot point them at something else (`claim_tree`
in [`profile.sh`](bin/lib/profile.sh)). The exception is an unmarked
tree from before markers existed, which is adopted when its parent is
`build/` or another tree and it is not a git checkout.

### Configure policy

- Every optional dependency is `--without-*` unless re-enabled with
  `--with-*` in [`build-wine.sh`](bin/build-wine.sh), so a missing
  package fails configure instead of silently changing the feature set.
- Never pass `--with-opengl` on macOS: it triggers an EGL probe that always
  fails there, and the Mac driver links `-framework OpenGL` on its own.
- Wine's Vulkan is on, on the loader and KosmicKrisp `build-mesa.sh` builds
  ([Vulkan](docs/arm64.md#vulkan)). The bundle does not ship Wine's own
  D3D12 modules.
