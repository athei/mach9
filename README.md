# wine-build

Builds an x86_64 WoW64 (i386 and x86_64) Wine for macOS from
[athei/wine](https://github.com/athei/wine), CrossOver Wine with custom
patches, and bundles it into a self-contained, relocatable `wine/` tree.
Direct3D goes through per-process selectable implementations (D3DMetal, DXMT,
mtld3d, wined3d), chosen by `compatdb.so`, a library built from
[`compatdb/`](compatdb).

- [docs/compatdb.md](docs/compatdb.md): rules, `WINE_COMPATDB`, recipes,
  logging.
- [docs/maintenance.md](docs/maintenance.md): internals, floors, pins, the
  Wine-side contract, upgrade checklist.
- [docs/arm64.md](docs/arm64.md): the native arm64 build, from upstream
  Wine with our own series and FEX, local only.

## Getting a build

| [Release](https://github.com/athei/wine-build/releases) tag | What it is | Use it? |
|-----|------------|---------|
| `cx-*` | Full Wine build: `wine-<tag>-macos-x86_64.tar.xz` | Yes |
| `compatdb-*` | A published `cx-*` bundle (the newest, by default) with `compatdb.so` rebuilt from the tag; same asset name pattern | Yes |
| `deps-*` | The x86_64 dependency prefix CI builds against | No, build input |
| `gptk-*` | Mirror of Apple's Game Porting Toolkit dmg | No, build input |

- Both bundle kinds have the same `wine/` layout. GitHub may mark a
  `compatdb-*` release as Latest, so filter by the `cx-` prefix to find the
  newest Wine build.
- The bundle is x86_64 and needs Rosetta on Apple Silicon.
- The PE files ship without debug info or symbol tables. A release that
  strips them carries them as the optional asset
  `wine-<tag>-macos-x86_64-symbols.tar.xz`: the unstripped files under the
  same paths, for symbolizing crash reports.
- A browser download marks the tarball as quarantined; remove it after
  unpacking with `xattr -drs com.apple.quarantine wine`.
- The minimum macOS is `MACOS_FLOOR` of the `intel` profile in
  [`profile.sh`](profile.sh). D3DMetal needs a newer macOS
  (`GPTK_FLOOR` in [`bundle-wine.sh`](bundle-wine.sh)); below it, processes
  that would get D3DMetal get DXMT instead.

## Bundle layout

| Path | Contents |
|------|----------|
| `bin/` | Exactly what `make install` creates: `bin/wine` is the preloader, the other programs are symlinks to it |
| `bin/x87sidecar` | [x87sidecar](https://github.com/athei/x87sidecar), started by ntdll for i386 processes under Rosetta |
| `lib/wine/<arch>-windows/`, `lib/wine/x86_64-unix/` | Wine's modules; for the Direct3D DLLs only fake-module markers |
| `lib/wine/dxgi/<impl>/`, `lib/wine/d3d9/<impl>/` | One tree per Direct3D implementation (see [Direct3D backends](#direct3d-backends)) |
| `lib/wine/x86_64-unix/compatdb.so` | The rule library ntdll loads into every process |
| `lib/external/` | Bundled dylibs, `D3DMetal.framework`, `libd3dshared.dylib`, third-party licenses |
| `lib/wine/tests/{i386,x86_64}-windows/d3d9_test.exe` | Wine's D3D9 tests as plain PEs, outside the loader's search path |

## Runtime configuration

### Environment variables

| Variable | Effect |
|----------|--------|
| `WINE_COMPATDB` | Extra or changed compatdb rules for the whole process tree. Format: [docs/compatdb.md](docs/compatdb.md#wine_compatdb) |
| `ROSETTA_X87_PATH` | Unset: i386 processes use the bundled `bin/x87sidecar`. A path: that binary instead. Empty: no process gets a sidecar, whatever the rules say |
| `WINE_FORCE_NX` | x86_64 build: unset, no-execute is permanently on under Rosetta; `0`, a process may turn it off, as on Windows, on any host; any other value, permanently on, on every host. [arm64 build](docs/arm64.md): unset or `0`, Wine's behavior; any other value, permanently on |
| `CX_APPLEGPTK_LIBD3DSHARED_PATH` | Overrides the `libd3dshared.dylib` ntdll loads (default: the one in `lib/external/`) |
| `WINEDLLOVERRIDES` | Works as in Wine, but cannot undo an override compatdb adds; use a rule ([recipes](docs/compatdb.md#recipes)) |

### Direct3D backends

| Family | APIs | Rule value | Tree | Arch | Default for |
|--------|------|------------|------|------|-------------|
| `dxgi` | D3D10, D3D11, D3D12 | `gptk` (Apple D3DMetal) | `lib/wine/dxgi/gptk/` | x86_64 | x86_64 |
| `dxgi` | D3D10, D3D11 | `dxmt` ([DXMT](https://github.com/3Shain/dxmt)) | `lib/wine/dxgi/dxmt/` | i386, x86_64 | i386 |
| `dxgi` | D3D10, D3D10.1, D3D11 | `wined3d` | `lib/wine/dxgi/wined3d/` | i386, x86_64 | none |
| `d3d9` | D3D9 | `mtld3d` ([mtld3d](https://github.com/athei/mtld3d)) | `lib/wine/d3d9/mtld3d/` | i386, x86_64 | i386, x86_64 |
| `d3d9` | D3D9 | `wined3d` | `lib/wine/d3d9/wined3d/` | i386, x86_64 | none |

- compatdb.so prepends one tree per family to every process's builtin search
  path, so a process sees one coherent set of modules. To pick a tree per
  game, see [recipes](docs/compatdb.md#recipes).
- On a macOS where D3DMetal does not load, a process that would get `gptk`,
  from the default or from a rule, gets `dxmt` instead
  ([gptk availability](docs/compatdb.md#gptk-availability)). There, and in
  every i386 process, D3D12 is not available.
- D3D10 to D3D12 form one family because they all create their device
  through one `dxgi.dll` and only work with their own implementation's copy.
- The `gptk` tree carries Wine's `d3d10_1`, which fails on D3DMetal's DXGI,
  so a D3D10.1 program needs `wined3d` (as the Rockstar launcher rule does).
  D3D8 and DirectDraw stay on wined3d. Which modules each tree holds is in
  Step 4 of [`bundle-wine.sh`](bundle-wine.sh).
- wined3d draws through OpenGL. In the [arm64 build](docs/arm64.md#vulkan)
  the compatdb setting `wined3d` can put it on Vulkan for one game; it
  applies to every wined3d user in the process
  ([wined3d on Vulkan](docs/compatdb.md#wined3d-on-vulkan)).

### Defaults that differ from Wine

Each row decided by a rule is a built-in compatdb rule that matches every
process and can be changed per game or globally. No-execute (both builds)
and synchronization (arm64 build) are decided by the patched Wine.

| Behavior | Default in this bundle | Decided by |
|----------|------------------------|------------|
| DPI awareness | Every process is system-aware | Rule `dpi-aware` ([recipes](docs/compatdb.md#recipes)) |
| Address space | Every 32-bit process gets 4 GB, even without the large-address-aware flag | Rule `large-address-aware` ([recipes](docs/compatdb.md#recipes)) |
| x87sidecar | Every i386 process under Rosetta starts under it | Rule `x87-sidecar` ([recipes](docs/compatdb.md#recipes)) |
| x87 precision under FEX ([arm64 build](docs/arm64.md) only) | FEX emulates every program's x87 code with reduced precision | Rule `fex-x87-reduced-precision` ([arm64 Wine](docs/compatdb.md#arm64-wine), [recipes](docs/compatdb.md#recipes)) |
| Synchronization ([arm64 build](docs/arm64.md) only) | CrossOver's msync is on: synchronization objects use Mach semaphores and shared memory | wineserver; `WINEMSYNC=0` where the session starts turns it off ([arm64 runtime](docs/arm64.md#runtime)) |
| Vulkan loader (x86_64 build only) | Disabled (`vulkan-1=`); that bundle ships no Vulkan. The [arm64 build](docs/arm64.md#vulkan) ships Wine's Vulkan on KosmicKrisp and leaves the loader alone | Rule `no-vulkan` ([recipes](docs/compatdb.md#recipes)) |
| KosmicKrisp's experimental features ([arm64 build](docs/arm64.md#vulkan) only) | Advertised to every process: the ones the rule names in `MESA_KK_EXPERIMENTAL` | Rule `kosmickrisp-experimental` ([arm64 Wine](docs/compatdb.md#arm64-wine), [recipes](docs/compatdb.md#recipes)) |
| Mono and Gecko | Disabled (`mscoree,mshtml=`): no install prompt, and an installed wine-mono, Gecko or native .NET stays disabled too | Rule `no-mono-gecko` ([recipes](docs/compatdb.md#recipes)) |
| No-execute | x86_64 build: follows the main executable's `NX_COMPAT` flag alone, as Windows does ([#3](https://github.com/athei/wine-build/issues/3)). Permanently on under Rosetta: a program that tries to turn it off gets `STATUS_ACCESS_DENIED`. The [arm64 build](docs/arm64.md) keeps Wine's behavior unless `WINE_FORCE_NX` keeps no-execute permanently on | ntdll, `WINE_FORCE_NX` ([Wine-side contract](docs/maintenance.md#wine-side-contract)) |

## Building

### Requirements

- Apple Silicon with Rosetta, or an Intel Mac. Wine is built for x86_64.
- x86_64 Homebrew in `/usr/local/` (not the ARM one in `/opt/homebrew/`)
  with the formulae in `ROOTS` in [`package-deps.sh`](package-deps.sh), and
  a `pkg-config`.
- [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) at `MINGW_DIR`. The
  scripts append its `bin/` to `PATH`; never prepend it, since its
  unprefixed `clang` breaks configure's host compiler detection.
- Rust with the `x86_64-apple-darwin` target, for `compatdb.so`.

The arm64 build needs a different set: see
[docs/arm64.md](docs/arm64.md#requirements).

### Commands

```bash
./build-wine.sh    # configure and compile
./bundle-wine.sh   # assemble the bundle (see Layout)
```

| Script | Flag | Effect |
|--------|------|--------|
| `build-wine.sh` | none | Incremental build; configure runs only when the build directory has no Makefile |
| `build-wine.sh` | `--clean` | Wipes the build directories first; the only way to pick up changed configure flags or deployment target |
| `build-wine.sh`, `bundle-wine.sh` | `--arm64` | The native arm64 profile instead of the x86_64 one; see [docs/arm64.md](docs/arm64.md) |
| `bundle-wine.sh` | `--dest <dir>` | Writes `<dir>/wine` instead of the profile's bundle in the [layout](#layout). Either way it asks before it deletes an existing one, unless `--yes` |
| `bundle-wine.sh` | `--yes` | Deletes an existing bundle without asking, for an unattended run. A directory that is not empty and has no `bin/wine` is still refused |
| `bundle-wine.sh` | `--runtime-only` | No development files and no test binaries; the flavor to put into an application |
| `bundle-wine.sh` | `--compatdb-only` | Rebuilds `compatdb.so` into the existing bundle only (the profile's, or `<dir>/wine` with `--dest`); no prompt, no `make install`, no Wine build needed, works on an unpacked release tarball; `--runtime-only` is ignored. With `--arm64` it refuses a bundle without Vulkan ([docs/arm64.md](docs/arm64.md#what-the-bundle-step-checks)) |
| `bundle-wine.sh` | `--strict-floor` | A Mach-O that needs a newer macOS than its floor is an error instead of a warning |
| `bundle-wine.sh` | `--symbols` | Also keeps the PE files as they were before their symbols were stripped, in `wine-symbols` next to the bundle; the release workflow passes it |

`usage()` and the argument parsing in [`bundle-wine.sh`](bundle-wine.sh) are
authoritative. What the bundle step checks is in
[docs/maintenance.md](docs/maintenance.md#what-bundle-winesh-verifies).

### Layout

Everything lives beside this repository, in the directory that holds it
(the root; `WINE_BUILD_ROOT` names another, for a checkout kept elsewhere):

| Path | What it is |
|------|------------|
| `scripts/` | This repository |
| `src/<profile>/` | The profile's Wine source: `src/intel` is the checkout of athei/wine, `src/arm64` a linked worktree of it |
| `src-fex/` | The FEX checkout |
| `src-mesa/` | The Mesa checkout, for the arm64 build's Vulkan driver |
| `cache/` | Downloaded redistributables |
| `build/<profile>/` | Every intermediate of a profile: the Wine build and, for arm64, the SDK and its link-library tree, FEX, DXMT and Mesa with the Vulkan loader |
| `dist/<profile>/wine` | The profile's bundle |
| `dist/<profile>/wine-symbols` | With `--symbols`, the bundle's PE files before their symbols were stripped (Step 4e of [`bundle-wine.sh`](bundle-wine.sh)); not part of the bundle |

`<profile>` is `intel` or `arm64`. The name of each tree is set in
[`profile.sh`](profile.sh), with the variable that overrides it. The
scripts mark each build tree they make with a `.wine-build-tree` file and
refuse to fill or delete a directory that is not empty and has no such
marker, so an override cannot point them at something else
(`claim_tree` in [`profile.sh`](profile.sh)). The exception is an unmarked
tree from before markers existed, which is adopted when its parent is
`build/<profile>/` or another tree of the same profile and it is not a git
checkout.

### Paths

| Variable | What it is | Used by |
|----------|------------|---------|
| `WINE_SRC` | athei/wine checkout | build |
| `BUILD_DIR` | Wine build tree; with `--compatdb-only` only cargo's target directory, `$BUILD_DIR/compatdb` | build, bundle |
| `MINGW_DIR` | llvm-mingw | build, bundle |
| `CACHE_DIR` | Downloaded redistributables | bundle |

These are the x86_64 build's; the arm64 build has its own names, listed in
[docs/arm64.md](docs/arm64.md#paths). `MINGW_DIR` and `CACHE_DIR` are set at
the top of [`build-wine.sh`](build-wine.sh) and
[`bundle-wine.sh`](bundle-wine.sh), the others in [`profile.sh`](profile.sh).

### Configure policy

- Every optional dependency is `--without-*` unless re-enabled with
  `--with-*` in [`build-wine.sh`](build-wine.sh), so a missing package fails
  configure instead of silently changing the feature set. A profile adds to
  that list through `CONFIGURE_FEATURES_EXTRA` in [`profile.sh`](profile.sh).
- Never pass `--with-opengl` on macOS: it triggers an EGL probe that always
  fails there, and the Mac driver links `-framework OpenGL` on its own.
- Vulkan follows the profile's `WITH_VULKAN` in [`profile.sh`](profile.sh).
  The x86_64 build is configured `--without-vulkan`, since nothing in its
  bundle needs Vulkan, and the bundle step deletes the Vulkan modules such
  a build cannot back. The arm64 build has Vulkan
  ([docs/arm64.md](docs/arm64.md#vulkan)). Neither bundle ships Wine's own
  D3D12 modules.

## Releasing

| | `cx-*` | `compatdb-*` |
|-|--------|--------------|
| Workflow | [release.yml](.github/workflows/release.yml) | [release-compatdb.yml](.github/workflows/release-compatdb.yml) |
| Builds Wine | Yes, from the ref in [`wine-src.ref`](wine-src.ref) | No; takes the tarball of the newest published `cx-*` release |
| `compatdb.so` | Built from the tagged commit | Built from the tagged commit |
| Use for | Anything that changes the Wine side: a new ntdll or compatdb export, a new tree under `lib/wine` | A change confined to `compatdb/` |
| Output | `wine-<tag>-macos-x86_64.tar.xz` on a draft release | Same |

A `cx-*` release:

1. Push the source state to athei/wine.
2. Point `wine-src.ref` at it (branch, tag or commit) and push to `main`.
   That `main` must already contain any compatdb changes.
3. Tag and push `cx-<crossover version>-<build revision>`, the revision
   starting at 0.
4. Review and publish the draft release.

A `compatdb-*` release: push the change to `main`, tag and push
`compatdb-<date>`, then review and publish the draft release. To use a
different base or rebuild an existing tag, run the workflow by hand:

```bash
gh workflow run release-compatdb.yml -f tag=compatdb-<date> -f base=cx-<version>-<n>
```

`base` is optional and must be a published `cx-*` release; drafts and
pre-releases are never used. `gh release create` refuses a tag that already
has a release, so delete the old draft before running again for the same
tag. Pins, floors and the dependency prefix: see
[docs/maintenance.md](docs/maintenance.md).
