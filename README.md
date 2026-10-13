# wine-build

Scripts for building Wine from source on macOS as an x86_64 binary and
bundling it into a self-contained, relocatable distribution with WoW64
(i386+x86_64). Sources come from [athei/wine](https://github.com/athei/wine),
CrossOver Wine with custom patches on top. Direct3D does not go through
wined3d by default: the bundle ships Apple's D3DMetal, DXMT and mtld3d as
per-process selectable implementations, plus the small library that selects
them.

## Build pipeline

```bash
./build-wine.sh                       # Step 1: compile (arch -x86_64)
./bundle-wine.sh --dest /path/to/out  # Step 2: relocatable wine/ tree
```

`build-wine.sh` configures and compiles Wine into the build directory. It is
incremental; `--clean` wipes the build directory first, which is also the only
way to pick up changed configure flags. Before and after every build it makes
sure the sonames in `config.h` point at `@loader_path/../../external/`, since
`config.status` silently regenerates the file after a source update. It also
builds the two `d3d9_test.exe` binaries the bundle carries, and the ARM64X
link libraries: a second tree, configured with `arm64ec,aarch64` so the
archives carry both ARM64 and ARM64EC members, that yields only
`libwinecrt0.a` and `libntdll.a` under
`dist/wine-arm64x/lib/wine/aarch64-windows/`. They are for linking ARM64X PE
builtins that CrossOver's arm64 Wine will load, which ships no link archives
of its own. Nothing in that tree is runnable.

`bundle-wine.sh` turns the build output into a distributable `wine/` tree:
staged `make install`, prefix flattened, dylibs copied into `lib/external/`
with `@loader_path` install names, the Direct3D implementations, the
x87sidecar and `compatdb.so` installed, then a verification pass that ends by
booting a throwaway prefix and starting the i386 `cmd.exe` in it. That run
proves that ntdll started the program under `bin/x87sidecar` (it looks for
ntdll's launch line) and that the program exited cleanly within a time limit;
it does not prove that the sidecar's x87 hook took effect. A full bundle whose
ntdll lacks the `compatdb_preinit_query` export fails the check, since it was
built from a Wine tree without the ntdll side of the x87sidecar.
`--runtime-only` skips the SDK files and the test binaries; that is the flavor
that goes into an application. The verification pass also compares the
minimum macOS of every Mach-O in the tree with the floor it is held to and
warns about each file that needs a newer one; `--strict-floor` turns that
warning into an error (see [Releases](#releases)).

`--compatdb-only` updates the bundle already at `<dest>/wine` instead of
building a new one: it builds `compatdb.so`, replaces the installed copy,
checks the new library for `/usr/local/` references and its minimum macOS
(only that library, since the rest of the tree was not built here), and boots
a prefix as a full bundle does. The x87sidecar check is skipped when the
bundle's ntdll lacks the `compatdb_preinit_query` export, since an ntdll that
old does not start the bundled sidecar on its own; otherwise it runs as in a
full bundle, including the check that `bin/x87sidecar` is an arm64
executable. There is no prompt, nothing is deleted, `make install` does not
run, and the Wine build directory is not needed, so it works on an unpacked
release tarball as well. It keeps whatever flavor the bundle already has;
`--runtime-only` is accepted next to it and ignored.

The `d3d9_test.exe` binaries land in `lib/wine/tests/{i386,x86_64}-windows/`,
outside the directories the loader searches, and are plain PEs. Wine's
`dlls/d3d9/tests` is the de-facto D3D9 conformance suite, so a consumer
(mtld3d's CI) can gate its own `d3d9.dll` against it with nothing but this
bundle.

### Paths

| Variable    | Default           | Used by       |
|-------------|-------------------|---------------|
| `WINE_SRC`  | `../src`          | build         |
| `BUILD_DIR` | `../build`        | build, bundle |
| `MINGW_DIR` | `/opt/llvm-mingw` | build, bundle |
| `CACHE_DIR` | `../cache`        | bundle        |

With `--compatdb-only`, `BUILD_DIR` only hosts cargo's target directory,
`$BUILD_DIR/compatdb`.

### Requirements

- Apple Silicon with Rosetta, or an Intel Mac. Everything compiles under
  `arch -x86_64`.
- x86_64 Homebrew in `/usr/local/` (not the ARM one in `/opt/homebrew/`):
  `freetype gnutls sdl2-compat sdl3`, plus `bison` 3.0 or newer and `pkgconf`.
- [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) at `MINGW_DIR` for the
  PE modules. Its `bin/` is appended to `PATH`, never prepended: it carries an
  unprefixed `clang` targeting MinGW that would break configure's host compiler
  detection.
- A Rust toolchain with the `x86_64-apple-darwin` target, for `compatdb.so`.

Configure uses a strict allowlist: every optional dependency is `--without-*`
and only re-enabled with `--with-*`, so a missing package fails configure
loudly instead of silently changing the feature set. Never pass
`--with-opengl` on macOS; it triggers an EGL probe that always fails there,
and the Mac driver links `-framework OpenGL` on its own. Vulkan is off
(`--without-vulkan`): nothing in the bundle needs it, and wined3d serves D3D8
and DDraw through its GL backend.

## Relocation

`build-wine.sh` patches the `SONAME_LIB*` defines in `config.h` to
`@loader_path/../../external/<lib>` before compiling, so Wine's `.so` modules
in `lib/wine/x86_64-unix/` find the bundled dylibs in `lib/external/` through
dyld alone. The bundle step then copies each dylib and its `/usr/local`
closure into `lib/external/`, rewrites their install names to `@loader_path/`,
and verifies that no `/usr/local/` reference survives. There is no launcher
binary and no `DYLD_FALLBACK_LIBRARY_PATH`; `bin/` is exactly what
`make install` creates, `bin/wine` being the preloader and the convenience
programs symlinks to it.

## Releases

There are two kinds of release. Both attach a complete bundle as
`wine-<tag>-macos-x86_64.tar.xz` to a draft release, with the same `wine/`
layout, so a consumer does not need to tell them apart.

[.github/workflows/release.yml](.github/workflows/release.yml) builds Wine on
a GitHub-hosted Apple Silicon runner with Rosetta, the x86_64 dependency
prefix and llvm-mingw whenever a `cx-*` tag is pushed; the runner image and
the llvm-mingw version are set in the workflow. Tags are named after the
CrossOver version of the sources plus a build revision, so `cx-26.2.0-0` is
the first build from CrossOver 26.2.0. [`wine-src.ref`](wine-src.ref) pins the
branch, tag or commit of athei/wine that gets built. The bundle includes
`compatdb.so` built from the same commit of this repository, and the release
notes name that commit.

1. Push the desired source state to athei/wine.
2. Point `wine-src.ref` at it and push to `main`.
3. `git tag cx-26.2.0-0 && git push origin cx-26.2.0-0`.
4. Review and publish the draft release.

[.github/workflows/release-compatdb.yml](.github/workflows/release-compatdb.yml)
runs when a `compatdb-*` tag is pushed and does not build Wine. It downloads
the tarball of the newest published `cx-*` release (drafts and pre-releases
are never used), checks it against the digest GitHub recorded for the asset,
and runs `bundle-wine.sh --compatdb-only` on it with the tagged commit. It
takes a few minutes instead of hours. The release notes name the tagged commit
and the `cx-*` release the Wine came from. To use a different base, or to
build an existing tag again, run it by hand:

```bash
gh workflow run release-compatdb.yml -f tag=compatdb-2026-10-01 -f base=cx-26.3.0-6
```

`base` is optional and must be a published `cx-*` release. `gh release create`
refuses a tag that already has a release, so delete the old draft before
running it again for the same tag.

A change confined to `compatdb/` (new rules, a fix in the library) can go out
as `compatdb-*`. Anything that needs the Wine side to change, such as a new
ntdll export for compatdb to call, a new compatdb export for ntdll to call or
a new tree under `lib/wine`, needs a `cx-*` release. Tag `cx-*` from a
`main` that already contains the compatdb changes, since that release builds
`compatdb.so` from its own commit.

A published `compatdb-*` release can become the one GitHub marks as Latest.
Anything that fetches the latest release has to filter by tag prefix rather
than rely on that marker.

[`build-wine.sh`](build-wine.sh) exports a fixed `MACOSX_DEPLOYMENT_TARGET`,
and [`.cargo/config.toml`](.cargo/config.toml) sets the same value for
`compatdb.so`. Without the pin clang takes the deployment target from whatever
host is building, which made the tarball's macOS floor an accident of the
runner image: `cx-26.3.0-3` shipped `minos 15.0` off the `macos-15` runner
while a local build on macOS 27 produced `minos 26.0`. The SDK is still
whatever the build host has; only the minimum is fixed. Check this pin still
holds whenever the runner image is bumped.

The pin is the floor of the whole bundle except GPTK's `D3DMetal.framework`,
the default for 64-bit D3D10 to D3D12. D3DMetal links
`libdxccontainer.dylib`, which needs a newer macOS than the bundle's floor
(`GPTK_FLOOR` in [`bundle-wine.sh`](bundle-wine.sh)), and it is shipped
unmodified, so it cannot be built for a lower floor. On an older macOS,
compatdb.so gives the processes that would get D3DMetal DXMT instead (see
[compatdb.so](#compatdbso)). Wine picks up a changed pin on the next
`build-wine.sh --clean`, which is how CI always builds.

The workflow does not install Homebrew. Homebrew has stopped shipping x86_64
macOS bottles (gmp, sdl2-compat and sdl3 have none at all, the rest stop at the
sonoma tag), so a runner would have to compile them from source: that took 48
minutes on `cx-26.3.0-3` and then stopped working altogether, because the
runners resolve neither `ftpmirror.gnu.org` nor `gmplib.org` and gmp's source
cannot be fetched there at all.

Instead [`package-deps.sh`](package-deps.sh) builds the x86_64 prefix once on a
machine that has an Intel Homebrew, and the result is mirrored as a release
asset that the workflow unpacks into `/usr/local`. It ships the packages
listed in `ROOTS` in [`package-deps.sh`](package-deps.sh) plus their runtime
closure, mostly taken from Homebrew's sonoma bottles, whose minimum macOS is
below every floor this repository pins.

gmp is the exception. It has no x86_64 bottle at all, so Homebrew compiles it
on whatever machine runs `package-deps.sh` and stamps it with that machine's
macOS: one packaging run produced a `libgmp` at `minos 26.0`, which would not
have loaded on anything older and would have taken gnutls down with it. The
script rebuilds gmp against its own `MACOS_FLOOR`, and then refuses to write
the archive if any Mach-O in the tree is newer than that floor. If another
formula loses its bottle, that check is what will catch it.

The `MACOS_FLOOR` set in [`package-deps.sh`](package-deps.sh) must not be
newer than the deployment target in [`build-wine.sh`](build-wine.sh). It may
be lower, since libraries built for an older macOS load in a Wine built for a
newer one. Changing it means rebuilding and uploading the `deps-*` release.

To refresh it: `arch -x86_64 /usr/local/bin/brew upgrade` the formulae, run
`./package-deps.sh`, attach `dist/wine-deps-macos-x86_64.tar.xz` to a new
`deps-<date>` release, and point `DEPS_URL` and `DEPS_SHA256` in the workflow
at it. The one thing the prefix does not provide is `pkg-config`, which comes
from the runner image; the workflow fails loudly if it is missing rather than
building a Wine with the libraries silently absent.

`bundle-wine.sh` checks the minimum macOS (`minos`) of every Mach-O in the
finished bundle against the deployment target `build-wine.sh` builds with
(it reads the same `MACOSX_DEPLOYMENT_TARGET`, with the same default). The
files of Apple's `lib/external/D3DMetal.framework` are held to a separate
floor instead, `GPTK_FLOOR` in [`bundle-wine.sh`](bundle-wine.sh), which has
to equal `GPTK_MIN` in [`compatdb/src/macos.rs`](compatdb/src/macos.rs), the
version compatdb.so falls back to DXMT below. `libd3dshared.dylib` is not part
of that: ntdll loads it in every process that loads a native PE module, such
as a game's executable, so it has to meet the normal floor.
Without `--strict-floor` each file over its floor is printed as a warning and
the bundle step carries on; with it they are an error. A missing `otool`, or
a run in which no file had a `minos` to read, is an error either way. Both
release workflows pass `--strict-floor`: their libraries come from the
`deps-*` tarball, which already meets the floor. A local build copies whatever
Homebrew has in `/usr/local`, and a formula Homebrew compiled on that machine,
such as gmp, carries the minimum of the macOS it was built on. A bundle built
on a newer macOS can therefore exceed the floor; it is not distributed, so a
warning is enough there.

## No-execute

Wine turned data execution prevention off for a whole process as soon as any
loaded module lacked the `NX_COMPAT` flag, a DLL included. Turning it off
makes every readable mapping executable for the rest of the process, and
under Rosetta each fresh writable and executable page costs a Mach round trip
on first touch, so a 32-bit program with one 2000s-era DLL turns into a fault
storm on every allocation
([#3](https://github.com/athei/wine-build/issues/3)). The patched tree
decides from the main executable alone, as Windows does: a DLL without the
flag changes nothing.

Under Rosetta no-execute is also permanently on, like the Windows AlwaysOn
policy: an executable without the flag keeps it, and a program that tries to
turn it off gets `STATUS_ACCESS_DENIED`. `WINE_DISABLE_NX_COMPAT=0` restores
the Windows behaviour for a game that really executes its data; any other
value keeps no-execute permanently on, on every host. A compatdb `env` rule
sets it per game.

## x87sidecar

Rosetta translates x87 floating-point code slowly, and many older 32-bit
games are full of it. [x87sidecar](https://github.com/athei/x87sidecar)
attaches to an i386 process under Rosetta and runs that code faster. The
bundle ships it as `bin/x87sidecar`, from the release pinned in
[`redist.env`](redist.env), and ntdll starts every i386 process
under it unless compatdb.so says otherwise for that executable (the
`x87_sidecar` setting, see [compatdb.so](#compatdbso)). It attaches in
cooperative mode, so it needs no entitlements and the bundle stays
notarizable.

`ROSETTA_X87_PATH` decides which sidecar is used for the whole launch. Unset,
ntdll uses the bundled `bin/x87sidecar`. Set to a path, it uses that binary
instead, for example a development build. Set to an empty value, no process
gets a sidecar, whatever the rules say. ntdll skips a sidecar that is missing
or not executable, and a 64-bit process never gets one.

## Direct3D

Every Direct3D implementation lives in its own tree under `lib/wine`, and the
default `<arch>-windows` directories hold only fake-module markers for the
DLLs involved. `compatdb.so` prepends one tree per API family to the builtin
search path of every process, so a process sees exactly one coherent set of
modules, nothing is ever mixed, and switching is purely additive.

There are two families because D3D10, D3D10.1, D3D11 and D3D12 all create
their device through one `dxgi.dll` and only work with their own
implementation's copy, while neither D3D9 implementation touches DXGI at all.
D3D8 and DDraw have no alternative to wined3d and stay in the default dirs.

| Family | Tree | Contents | Default |
|--------|------|----------|---------|
| `dxgi` | `lib/wine/dxgi/gptk/` | Apple D3DMetal: dxgi, d3d10, d3d11, d3d12, nvapi64, nvngx; plus Wine's d3d10core and d3d10_1, which return `E_FAIL` on its dxgi but keep D3D10 probes from failing to load. x86_64 only, and only on a macOS D3DMetal loads on. | x86_64 |
| `dxgi` | `lib/wine/dxgi/dxmt/` | DXMT: dxgi, d3d10core, d3d11, winemetal for both arches; plus Wine's d3d10 and d3d10_1. | i386; x86_64 where D3DMetal does not load |
| `dxgi` | `lib/wine/dxgi/wined3d/` | Wine's dxgi, d3d10, d3d10core, d3d10_1, d3d11 for both arches. No d3d12, which needs Vulkan. | |
| `d3d9` | `lib/wine/d3d9/mtld3d/` | [mtld3d](https://github.com/athei/mtld3d): d3d9 and mtld3d for both arches, one x86_64 `mtld3d.so`. | both |
| `d3d9` | `lib/wine/d3d9/wined3d/` | Wine's d3d9 for both arches. Needs only `wined3d.dll`, which is in the default dir. | |

Each tree has `<arch>-windows/` and, where the implementation has a unix half,
`x86_64-unix/`. D3DMetal's unix modules are symlinks to
`lib/external/libd3dshared.dylib`; DXMT's and mtld3d's single x86_64 `.so`
also serves the i386 PE modules through wow64 entry points. `winemetal.so`
stays in `lib/wine/x86_64-unix/` as the shared DXMT backend. `nvngx` is
Apple's `nvngx-on-metalfx`, renamed because that is the name games load when
they probe for DLSS.

The bundle step deletes the Vulkan and D3D12 modules that a
`--without-vulkan` build cannot back, rather than ship modules that advertise
an API they cannot serve. The list is in [`bundle-wine.sh`](bundle-wine.sh).

### compatdb.so

`lib/wine/x86_64-unix/compatdb.so` is built from the [`compatdb/`](compatdb)
crate in this repository (a Cargo workspace: `compatdb-table` holds the pure
rule logic, `compatdb` the cdylib around it). Wine's ntdll dlopens
`<ntdll_dir>/compatdb.so` in every process before any PE code runs, the slot
CrossOver's own compat database uses. The library reads the process's image
name and version resource, resolves the rules that match, and applies them:

- `dxgi` and `d3d9`: which tree to prepend. Unset means the defaults in the
  table above. `gptk` cannot load everywhere: Apple ships no i386 D3DMetal,
  and the x86_64 one needs a newer macOS than the bundle's floor (`GPTK_MIN`
  in [`compatdb/src/macos.rs`](compatdb/src/macos.rs)). Where it cannot load,
  a process that would get it, from the x86_64 default or from a rule, gets
  `dxmt` instead, with a log line such as
  `dxgi = gptk not available on macOS 15.7.1 (needs macOS 26.4), using dxmt`.
  So `dxgi = gptk` in a rule reads as "GPTK where it loads". The library
  reads the version from `kern.osproductversion`. macOS puts a compatibility
  value (10.16 or 16.0) there for a process that runs with
  `SYSTEM_VERSION_COMPAT=1` or whose main binary was built against an old
  SDK, so when `kern.system_version_compat` is nonzero, the major version is
  10 or 16, or the value cannot be read, the Darwin release in
  `kern.osrelease` decides instead, and the log line names it
  (`on Darwin 25.3.0`). When neither is usable, the library assumes GPTK
  loads.
- `dll_overrides`: `WINEDLLOVERRIDES`-style `names=order` entries, added
  through ntdll's load-order hook.
- `dpi_aware`: `true` makes the process DPI-aware (system-aware), `false`
  makes it unaware. The value is exactly `true` or `false`, case-sensitive.
  win32u applies a rule's value before it looks at the `AppCompatFlags\Layers`
  registry value and the manifest, and the first setting wins, so the rule
  beats both. Unset leaves the decision to them.
- `large_address_aware`: `true` gives a 32-bit process the 4 GB address
  space even when its executable lacks the large-address-aware flag. `false`
  only stops forcing it: the executable's own flag,
  `WINE_LARGE_ADDRESS_AWARE` and the `AppDefaults` registry value then decide
  as they would without compatdb, so an executable that has the flag keeps
  it. It has no effect on a 64-bit process. ntdll applies the value after the
  library returns.
- `x87_sidecar`: `false` keeps the [x87sidecar](#x87sidecar) away from the
  executable, `true` attaches it. It only matters for an i386 process under
  Rosetta. Unlike the other settings it is not applied when the library loads
  into the process: ntdll asks for it through `compatdb_query_x87` before it
  starts the process, so the rules are matched against the executable's file
  (its basename and the version resource read from disk). For the first
  process of a launch, ntdll maps a DOS path such as `C:\...` through the
  prefix's `dosdevices` to a unix path first. When the path cannot be
  resolved, only the basename is used, and only rules that need no
  fingerprint can match.
- `arguments`: text appended to the command line unless already present,
  which leaves a Chromium child that inherited its parent's switches alone.
- `env`: `NAME=value` entries; an empty value removes the variable. ntdll
  asks for them through `compatdb_query_env` before it starts the process and
  sets them in the unix environment right before the exec, so whatever runs
  ahead of the program sees them too: its x87sidecar (the `X87_*` variables)
  and Rosetta. That query matches the executable's file the same way as for
  `x87_sidecar`. When the library loads into the process, it writes the
  entries into the environment block and the unix environment again.

Rules match on the image basename (case-insensitive, or `*` for every
process) plus optional case-insensitive substrings of the version resource's
`CompanyName`, `ProductName` and `OriginalFilename`. Every matching rule
applies, folded least specific first (wildcard, then basename, then
fingerprinted), so a scalar ends up with the most specific rule's value and
the lists accumulate.

The built-in rules are in
[`compatdb/src/builtin.rs`](compatdb/src/builtin.rs), each commented with the
reason it exists. Five of them, `no-vulkan`, `dpi-aware`,
`large-address-aware`, `x87-sidecar` and `no-mono-gecko`, match every process
(`exe` is `*`) and are described below.
The rest are pinned by version resource. The Rockstar Games Launcher needs a
real D3D10.1 device, which D3DMetal does not provide, so it gets `wined3d`.
The embedded Chromium (CEF) browsers of Steam and the Rockstar Social Club get
`--in-process-gpu`, because their GPU process cannot paint into another
process's window under winemac. Steam's helper is pinned to `dxmt` because
it runs better there than on the x86_64 default, and the Social Club helper
is pinned the same way to match it. Both helpers also turn the Metal HUD off
through `MTL_HUD_ENABLED`, and games do not inherit it because they start
from other processes. `gta-iv` adds a command-line switch that works around
the game's own video-memory detection. `nostale` makes NosTale's client
DPI-unaware: it draws its text with GDI into glyph atlases and UI boxes sized
for 96 DPI, so the fonts it gets at a retina DPI overrun both, while unaware it
gets 96 DPI fonts and a scaled window, as on Windows at 200%. Its version
resource names only the company, so the rule pins `CompanyName` alone.

`no-vulkan` adds the override `vulkan-1=`, which disables the Vulkan loader.
The bundle has no Vulkan: Wine is built `--without-vulkan` and the bundle
step deletes the Vulkan modules (the list is in
[`bundle-wine.sh`](bundle-wine.sh)). A Vulkan loader that a game ships next
to its executable can therefore only fail. With the override, `vulkan-1.dll`
fails to load, and a game that has another renderer uses it.

`dpi-aware` sets `dpi_aware = true`, so every process under this build is
system-aware by default, including a plain `wine foo.exe`. An unaware program
on a scaled desktop gets its window and mouse coordinates scaled while display
modes are not, so a game that sizes itself from the mode list draws at one
size and reads the pointer at another. The default also caps a program that
asks for per-monitor awareness, by manifest or by its own call, at
system-aware, because the first context set for a process wins. A rule for
one executable with `dpi_aware=false` makes that program unaware again, and
`name=dpi-aware;enabled=false` restores upstream behaviour for every process:
the registry and the manifest decide.

`large-address-aware` sets `large_address_aware = true`, so every 32-bit
program gets the 4 GB address space. Wine and the Direct3D layers live in the
same address space as the game, so an older game runs out of its 2 GB sooner
than it would on Windows. A rule for one executable with
`large_address_aware=false` stops forcing it for that program, and
`name=large-address-aware;enabled=false` stops forcing it for every process.

`x87-sidecar` sets `x87_sidecar = true`, so every i386 process under Rosetta
starts with the x87sidecar attached. A rule for one executable with
`x87_sidecar=false` keeps it away from that program. Dropping the rule with
`name=x87-sidecar;enabled=false` leaves the library without an opinion, and
ntdll then attaches the sidecar anyway; to keep it away from every process,
add a `*` rule with `x87_sidecar=false` or set `ROSETTA_X87_PATH` to an empty
value.

`no-mono-gecko` adds the override `mscoree,mshtml=`, which keeps Wine from
prompting to install Mono and Gecko, in wineboot and in any process that
loads either. It is a separate rule so that disabling `no-vulkan` does not
bring the prompts back. The override disables both modules, not only the
prompts: ntdll consults the overrides compatdb adds before the registry
`DllOverrides` keys, so a wine-mono or Gecko installed in the prefix, or a
native .NET selected through the registry, stays disabled as well. A prefix
that needs one has two ways out: `name=no-mono-gecko;enabled=false`, or a
rule that adds `mscoree=n,b` (see [WINE_COMPATDB](#wine_compatdb)).

Several `*` rules are not a conflict. They are folded in table order, the
built-in ones first, and `duplicate_matchers` does not report them. That
order is also how a launch-wide override works: a `*` rule added through
`WINE_COMPATDB` comes after the built-in ones, so
`name=global;exe=*;dpi_aware=false` beats the `dpi-aware` default for every
process that no more specific rule covers.

#### WINE_COMPATDB

Whatever starts the process tree can add or change rules through the
`WINE_COMPATDB` environment variable. The value is a format header line
(`HEADER` in [`compatdb/table/src/lib.rs`](compatdb/table/src/lib.rs))
followed by one rule per line; each rule is `key=value` fields joined by `;`:

```
v=5
name=my-game;exe=Game.exe;company=Some Vendor;d3d9=wined3d;dpi_aware=false;env=MTLD3D_CONFIG=adapter.spoof=amd
name=old-game;exe=OldGame.exe;large_address_aware=false;x87_sidecar=false
name=rockstar-launcher;dxgi=dxmt
name=steam-web-helper;enabled=false
```

Keys: `name` (required), `exe`, `company`, `product`, `original_filename`,
`dxgi`, `d3d9`, `dpi_aware`, `large_address_aware`, `x87_sidecar` (the last
three `true` or `false`), `dll_overrides`, `arguments`, `env` (repeatable,
value `NAME=value`) and `enabled`. A value the library does not know for
`dxgi`, `d3d9` or one of the three boolean keys drops that rule with a
diagnostic; the boolean keys take exactly `true` or `false`, so `TRUE` or `1`
is such a value. Inside a value, `%`, `;`, CR, LF and other control
characters are percent-encoded (`%3B` for `;`); everything else, including
`=`, passes through. A rule naming a built-in rule is merged into it (a set
scalar wins, lists append), so an override needs only the fields it changes;
`enabled=false` drops the rule of that name; a new rule needs an `exe`. A
malformed line is skipped, never fatal.

A header other than the one the library expects makes it ignore the whole
value with a diagnostic, which is what keeps a format change safe for
long-lived processes. It also means a library drops the whole table of a
launcher that still sends the previous header, so the launcher and
`compatdb.so` have to be updated together.

To let one game load its own Vulkan loader, give it a rule for its
executable that adds `vulkan-1=n`:

```
v=5
name=my-game;exe=Game.exe;dll_overrides=vulkan-1=n
```

The `*` rules are folded first, so the game's entry is added after
`vulkan-1=` and ntdll keeps the last entry for a module. To drop the built-in
rule for every process instead, use `name=no-vulkan;enabled=false`.
`WINEDLLOVERRIDES=vulkan-1=n` alone does not re-enable it: ntdll parses that
variable first, and an override compatdb adds replaces the entry for the same
module.

Mono and Gecko work the same way. A raw `WINEDLLOVERRIDES=mscoree=b` no
longer re-enables Mono, because the built-in `mscoree,mshtml=` replaces the
entry for the same module. A rule does:
`name=mono;exe=*;dll_overrides=mscoree=b` is folded after the built-in `*`
rules, and a rule for one executable is folded after every `*` rule. Use
`mscoree=n,b` instead to prefer a native .NET.
`name=no-mono-gecko;enabled=false` drops the built-in override for both
modules in every process, which hands the decision back to the registry.

The library writes `compatdb:` lines to wine's stderr: one block per process
with the image name, its version fingerprint, the rules that matched, the
trees it prepended, the overrides it added, the DPI awareness and large
address awareness it set, the `x87_sidecar (rule)` value for an i386 process,
and any parse diagnostics. Each `compatdb_query_x87` call adds one `x87 query`
line with the executable, its fingerprint and the `x87_sidecar (rule)` value
it answered with; it comes from the process that is starting the new one, so
it appears before that process's own block. Both show only what the rules say:
ntdll still skips the sidecar when `ROSETTA_X87_PATH` is empty, when there is
no executable sidecar, for a 64-bit process, or when it lacks the query. Each
`compatdb_query_env` call that hands entries to ntdll adds one `env query`
line with the executable, its fingerprint and the `NAME=value` entries in the
order ntdll sets them. It also comes from the process that is starting the
new one; a query that finds no entries logs nothing. A first launch by a DOS
path that ntdll cannot resolve to a unix path is matched by basename alone,
in both queries.
ntdll's own `attaching rosettax87` log line is what shows that the sidecar was
started. To confirm a tree took effect, look at the running process's mapped
files (`lsof -p <pid> | grep -i dxgi.dll`): a path under
`lib/wine/dxgi/<impl>/` or `lib/wine/d3d9/<impl>/` proves it.

### Where the files come from

[`redist.env`](redist.env) pins one URL and one SHA-256 per artifact.
`bundle-wine.sh` downloads each into `CACHE_DIR` and reuses the cached copy as
long as its checksum still matches the pin; a mismatch is an error, never a
silent re-download, so a bump changes both lines. The release workflow caches
the same directory keyed on the pin file's hash.

Before verification, the bundle step removes inherited `com.apple.quarantine`
attributes from the assembled `wine/` tree so macOS does not block a copied
library such as `winemetal.so`. Other attributes and code signatures are
preserved, and symlink targets outside the bundle are not touched. Failure to
remove quarantine stops the bundle step. Downloading the finished distribution
can apply quarantine again; this step only removes metadata inherited from
the build inputs.

DXMT, mtld3d and x87sidecar come from their GitHub releases. The x87sidecar
archive holds only the ad-hoc signed binary, so its MIT license is pinned
separately, from the same tag of its source repository, and shipped as
`lib/external/x87sidecar-LICENSE`. The binary is installed as it comes, with
its signature. Apple's Game Porting Toolkit download needs an Apple ID
session, so the unmodified dmg is attached to a `gptk-<version>` release on
this repository (those tags trigger neither release workflow) and the pin
points there; upgrading means downloading the new image by hand and creating a
new `gptk-*` release, and raising D3DMetal's floor if the new image needs a
newer macOS (see [Caveats](#caveats)). Apple's license (shipped as
`lib/external/D3DMetal-License.rtf`) allows distributing the Redistributables
unmodified for non-commercial purposes, which is why the files are copied byte
for byte: no `install_name_tool`, no re-signing, and the dylib closure walk
never touches them, which is why the Direct3D step runs after the dylib step.

### What the Wine side provides

The patched tree at athei/wine carries the glue:

- `dlls/winemac.drv/d3dmetal.c` exports `macdrv_functions`, the window, Metal
  and monitor helpers D3DMetal and DXMT `dlsym` out of `winemac.so`.
- `dlls/ntdll/ntdll.spec` exports `__wine_unix_call`, which D3DMetal's PE
  halves look up at runtime.
- `dlls/ntdll/unix/loader.c` `init_non_native_support()` dlopens
  `libd3dshared.dylib` and records its `__TEXT` range, which selects the
  ms-ABI unix call entry for calls arriving from Apple code; without it they
  take the sysv entry and crash. It defaults to
  `<ntdll_dir>/../../external/libd3dshared.dylib`, with
  `CX_APPLEGPTK_LIBD3DSHARED_PATH` as an override, and runs on the first
  native PE load.
- `dlls/ntdll/unix/loader.c` dlopens `compatdb.so` and exports
  `prepend_dll_path`, `add_load_order_override` and
  `set_compat_dpi_awareness` for it. The last one only stores the value;
  `dlls/win32u/sysparams.c` reads it through `ntdll_get_compat_dpi_awareness`
  when it first sets the process awareness. A `compatdb.so` running on an
  ntdll without the export logs that and ignores `dpi_aware`.
- `dlls/ntdll/unix/loader.c` also exports `set_compat_large_address_aware`,
  which compatdb.so calls with the `large_address_aware` value; ntdll raises a
  32-bit process's address limit after the library returns. Without the
  export the library logs that and ignores the setting.
- `dlls/ntdll/unix/loader.c` and `dlls/ntdll/unix/process.c` start i386
  processes under the [x87sidecar](#x87sidecar) and ask compatdb.so's
  `compatdb_query_x87` export first, in the parent before it starts a child,
  and in the first process of a launch before ntdll is initialized. For that
  early call ntdll loads the library by itself and exports
  `compatdb_preinit_query`, which tells the library's initializer to do
  nothing. CrossOver's own ntdll lacks the export, so the initializer runs as
  usual there.
- The same two files ask compatdb.so's `compatdb_query_env` export for the
  rule environment of a process before they start it, at the same two points
  (in the parent before the fork, and in the first process before ntdll is
  initialized), and set the entries in the unix environment right before the
  exec. The library writes the entries into a buffer ntdll provides, since
  ntdll may `dlclose` it right after an early query. Under an ntdll without
  this query the entries are still set when the library loads into the
  process, but too late for the x87sidecar and Rosetta.
- `dlls/ntdll/loader.c` decides no-execute from the main executable alone
  instead of turning it off for the process as soon as any module lacks
  `NX_COMPAT`, and `dlls/ntdll/unix/process.c` keeps it permanently on under
  Rosetta; see [No-execute](#no-execute).
- `loader/wine.inf.in` registers `atidxx64.dll`, `nvapi64.dll` and
  `nvngx.dll` as fake DLLs.

### Caveats

- `macdrv_functions` is a private contract with no version field.
  `d3dmetal.c` asserts the sizes of `struct macdrv_functions_t` and
  `struct d3dmetal_macdrv_win_data`; a mismatch with a newer GPTK or DXMT
  drop is a crash inside their code, not an error message. Check it on every
  GPTK, DXMT or CrossOver bump.
- `init_non_native_support()` is gated on Sonoma or later, and D3DMetal
  needs a newer macOS than the deployment target in
  [`build-wine.sh`](build-wine.sh) because it links `libdxccontainer.dylib`.
  compatdb.so gives DXMT to the processes that would get D3DMetal on an older
  macOS, and the bundle's minos check holds `D3DMetal.framework` to that
  newer floor instead of the deployment target. `GPTK_FLOOR` in
  [`bundle-wine.sh`](bundle-wine.sh) and the `GPTK_MIN` constants in
  [`compatdb/src/macos.rs`](compatdb/src/macos.rs) have to move together
  when a new GPTK image raises D3DMetal's minimum.
- D3DMetal and DXMT both get the window's client view through the
  `get_win_data(hwnd)` entry of `macdrv_functions`. winemac looks the window
  up in a table of the windows the calling process created, so neither can
  present into another process's window. Cross-process presentation (a CEF
  GPU process drawing into the browser window) still needs `--in-process-gpu`,
  which the Steam and Social Club rules add.
- A builtin only loads if its placeholder exists in the prefix, and wineboot
  stamps placeholders for what is in `lib/wine/<arch>-windows` when the prefix
  is created. The markers in the default dirs cover every DLL the trees
  supply, so a new prefix is complete; a builtin name added later (a
  development install of mtld3d into `lib/wine/d3d9/mtld3d/`, or a mod) needs
  a `wineboot -u` only if its name is new. A bundle redeploy wipes `lib/wine`,
  so such additions have to be reinstalled after one.
