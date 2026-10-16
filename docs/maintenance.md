# Maintenance

Internals of the build and bundle scripts, the inputs they pin, and what to
check when something upstream changes.

## Relocation

- `build-wine.sh` patches the `SONAME_LIB*` defines of the libraries the
  allowlist enables (`BUNDLED_SONAMES`) to
  `@loader_path/../../external/<lib>`, so Wine's `.so` modules in
  `lib/wine/<arch>-unix/` find the bundled dylibs in `lib/external/` through
  dyld alone. The placeholder defines configure writes for libraries it did
  not find (`FALLBACK_SONAMES`) stay as they are, and any other `SONAME_LIB*`
  define stops the build: it is a library the allowlist does not account
  for.
- `config.status` silently regenerates `config.h` after a source update, so
  the script checks the patch before every build and again after `make`,
  re-patching and rebuilding when it was lost.
- `bundle-wine.sh` takes the direct dependencies from the patched sonames in
  `config.h` (plus `libSDL3`, which sdl2-compat loads at run time), copies
  them and their `DEPS_PREFIX` closure into `lib/external/`, rewrites their
  install names to `@loader_path/`, and fails if any reference to one of the
  profile's `LEAK_PREFIXES` survives. The arm64 profile re-signs the
  rewritten dylibs ad hoc, since arm64 code does not load with a broken
  signature.
- There is no launcher binary and no `DYLD_*` variable.

## Direct3D trees

- Which trees a build has is `DXGI_IMPLS` and `D3D9_IMPLS` in
  [`profile.sh`](../profile.sh), in the order a default-dir marker prefers
  them.
- Each tree under `lib/wine/dxgi/` and `lib/wine/d3d9/` has
  `<arch>-windows/` and, where the implementation has a unix half,
  `<arch>-unix/`. D3DMetal's unix modules are symlinks to
  `lib/external/libd3dshared.dylib`; DXMT's and mtld3d's single `.so`
  also serves the i386 PE modules through wow64 entry points. Which modules
  go into which tree is in Step 4 of [`bundle-wine.sh`](../bundle-wine.sh).
- Every module a family's trees supply is removed from the default
  `<arch>-windows/` directory and replaced by a fake-module marker made from
  the first tree that has it.

## What bundle-wine.sh verifies

A full x86_64 bundle (the arm64 one: [arm64.md](arm64.md#what-the-bundle-step-checks)):

- Removes inherited `com.apple.quarantine` from the tree (and nothing else),
  so macOS does not block a copied library; a failure stops the bundle step.
- No dylib in `lib/external/` and no `.so` module, the tree unix modules
  included, references `/usr/local/`.
- The Direct3D files and the x87sidecar are in place, and the mtld3d tree's
  `d3d9.dll` is byte-identical to mtld3d's.
- No PE file has `.debug_*` sections or a COFF symbol table; Step 4e
  stripped them, keeping the unstripped files in `wine-symbols` next to the
  bundle with `--symbols`. The exception is the files from
  the GPTK image, which Step 4e leaves alone and which must be
  byte-identical to the image.
- No symlink in the tree dangles.
- `compatdb.so` exports `compatdb_query_env` and, in the x86_64 bundle,
  `compatdb_query_x87` (the arm64 library has no x87 query, see
  [arm64 Wine](compatdb.md#arm64-wine)).
- The minimum macOS (`minos`) of every executable, `.dylib` and `.so` (the
  newest slice of a universal file) is at most its floor, see
  [macOS floors](#macos-floors). Without `--strict-floor` each file over its
  floor is a warning; with it, an error. A missing `otool`, or a run in
  which no file had a `minos`, is an error either way.
- No unix module (Apple's `libd3dshared.dylib` aside), loader or
  `wineserver` imports a function of the profile's `FLOOR_NEWER_FUNCS`
  while the floor is below its version (see [macOS floors](#macos-floors)).
  That catches a Wine build tree configured before the override, which
  `minos` does not show.
- Boots a throwaway prefix and requires compatdb.so to report the default
  trees (`dxgi = gptk`, or `dxmt` on a macOS below `GPTK_FLOOR`, and
  `d3d9 = mtld3d`).
- Starts the i386 `cmd.exe` in that prefix with `ROSETTA_X87_PATH` and
  `WINE_COMPATDB` unset. It requires `bin/x87sidecar` to be an arm64
  executable, ntdll's launch line for the bundled sidecar, and a clean exit
  within a time limit. That proves ntdll started the program under the
  sidecar, not that the sidecar's x87 hook took effect.
- Fails if ntdll lacks the `compatdb_preinit_query` export: the Wine tree is
  missing the ntdll side of the x87sidecar.
- Runs [`smoke/hello.c`](../smoke/hello.c), built for x86_64 and for i686,
  in that prefix once per Direct3D tree that has modules for the arch, the
  tree picked with a `WINE_COMPATDB` rule. compatdb.so must report the tree
  and the program must load every module in it. This is what shows that the
  prefix placeholders made from the default-dir markers let each tree in.
  The `gptk` run is skipped below `GPTK_FLOOR`.

With `--compatdb-only`:

- Builds `compatdb.so` and replaces the installed copy by copy and rename,
  so a running process keeps a valid mapping.
- Removes quarantine from `compatdb.so` only, and only when it is there.
- Checks that the installed file matches the build output, the
  `/usr/local/` references and `minos` of `compatdb.so` only (the rest was
  not built here), the query exports, and the prefix boot.
- Skips the x87sidecar check when the bundle's ntdll lacks
  `compatdb_preinit_query`, since such an ntdll does not start the bundled
  sidecar on its own.

## macOS floors

| Setting | Defined in | Applies to | Constraint |
|---------|------------|------------|------------|
| `MACOS_FLOOR` of each profile | [`profile.sh`](../profile.sh) | Exported as `MACOSX_DEPLOYMENT_TARGET` for everything the scripts build (Wine, `compatdb.so`, FEX, DXMT), and the floor every Mach-O in the bundle is checked against | The `intel` value equals the `.cargo/config.toml` row |
| `MACOSX_DEPLOYMENT_TARGET` | [`.cargo/config.toml`](../.cargo/config.toml) | `compatdb.so` built by a plain `cargo build`; the scripts override it | Same as the `intel` `MACOS_FLOOR` |
| `GPTK_FLOOR` | [`bundle-wine.sh`](../bundle-wine.sh) | The `minos` check of `lib/external/D3DMetal.framework`, and which `dxgi` line the smoke test expects on the build host. D3DMetal links `libdxccontainer.dylib` and ships unmodified, so it cannot be rebuilt for a lower floor | Equal to `GPTK_MIN` |
| `GPTK_MIN`, `GPTK_MIN_TEXT`, `GPTK_MIN_DARWIN` | [`compatdb/src/macos.rs`](../compatdb/src/macos.rs) | Whether compatdb.so lets a process load D3DMetal, and the log text | Move together, with `GPTK_FLOOR` |
| `MACOS_FLOOR` | [`package-deps.sh`](../package-deps.sh) | The `deps-*` dependency prefix | Not newer than the `intel` `MACOS_FLOOR`; changing it means a new `deps-*` release |

- Without the pin, clang takes the deployment target from the build host, so
  the floor would follow the runner image. Check that it still holds when the
  runner image changes.
- Only the minimum is fixed; the SDK is whatever the build host has.
- An SDK newer than the floor can declare functions Wine's configure checks
  for that macOS only has above the floor. configure finds them, Wine calls
  them without an availability check, and the linker weak-imports them, so
  on an older macOS the call jumps to address 0. `minos` does not show
  this, but the compiler warns `'<function>' is only available on macOS
  <version> or newer`. A profile lists such functions in
  `FLOOR_NEWER_FUNCS` in [`profile.sh`](../profile.sh), and configure is
  told each is missing while the floor is below its version; a change
  takes effect on the next `build-wine.sh --clean`, and until then the
  bundle step fails on a module that still imports one. After a new SDK,
  search a clean build's log for that warning.
- Wine picks up a changed pin on the next `build-wine.sh --clean`, which is
  how CI always builds.
- A `MACOSX_DEPLOYMENT_TARGET` in the environment overrides `MACOS_FLOOR`
  and the `.cargo/config.toml` value.
- `libd3dshared.dylib` is held to the normal floor: ntdll loads it in every
  process that loads a native PE module, such as a game's executable.
- Both release workflows pass `--strict-floor`, since their libraries come
  from the `deps-*` tarball. A local build copies whatever Homebrew has, and
  a formula compiled on that machine carries its macOS as `minos`, so a local
  bundle can exceed the floor. It is not distributed, so a warning is enough.

## Pinned inputs

| Input | Pinned in | To update |
|-------|-----------|-----------|
| Direct3D redistributables, x87sidecar and its license | [`redist.env`](../redist.env), one URL and one SHA-256 each | Change both lines |
| Apple's Game Porting Toolkit dmg | A `gptk-<version>` release on this repository, named by `redist.env` | Download the image by hand, attach it to a new `gptk-*` release, repoint `redist.env`; raise the GPTK floors if the new D3DMetal needs a newer macOS |
| x86_64 dependency prefix | A `deps-*` release, named by `DEPS_URL` and `DEPS_SHA256` in [release.yml](../.github/workflows/release.yml) | See below |
| Runner image, llvm-mingw | The workflows in [`.github/workflows/`](../.github/workflows) | Edit the workflow; re-check the floors |
| Wine source | [`wine-src.ref`](../wine-src.ref) | Point it at a branch, tag or commit of athei/wine |
| Sources the arm64 build compiles (FEX, DXMT, LLVM for DXMT) and its provisioning profile | See [arm64.md](arm64.md#pinned-inputs) | |

- `bundle-wine.sh` downloads each artifact into `CACHE_DIR` and reuses the
  cached copy while its checksum matches the pin. A fresh download that does
  not match is an error, never a silent retry. CI caches the directory keyed
  on the hash of `redist.env`.
- The GPTK image is mirrored because Apple's download needs an Apple ID
  session. `gptk-*` and `deps-*` tags trigger no workflow.
- Apple's license (shipped as `lib/external/D3DMetal-License.rtf`) allows
  distributing the Redistributables unmodified for non-commercial purposes.
  The files are therefore copied byte for byte: no `install_name_tool`, no
  re-signing, and the dylib closure walk never sees them, which is why the
  Direct3D step runs after the dylib step.
- The x87sidecar archive holds only the ad-hoc signed binary, installed as
  it comes. Its license is pinned separately from the same tag and shipped
  as `lib/external/x87sidecar-LICENSE`. It attaches in cooperative mode, so
  it needs no entitlements and the bundle stays notarizable.

### The dependency prefix

- CI does not install Homebrew: Homebrew has stopped shipping x86_64 macOS
  bottles for some of the formulae, and gmp's source cannot be fetched from
  the runners.
- [`package-deps.sh`](../package-deps.sh) builds the prefix on a machine with
  an Intel Homebrew: the formulae in `ROOTS` plus their runtime closure.
- gmp has no x86_64 bottle, so the script rebuilds it against `MACOS_FLOOR`
  (`GMP_VERSION` has to match the installed formula). It refuses to write the
  archive if any Mach-O is newer than `MACOS_FLOOR`; that check also catches
  another formula that loses its bottle.
- To refresh: `arch -x86_64 /usr/local/bin/brew upgrade` the formulae, run
  `./package-deps.sh`, attach the archive it writes to `dist/intel/` to a new
  `deps-<date>` release, and point `DEPS_URL` and `DEPS_SHA256` at it.
- `pkg-config` is not in the prefix; it comes from the runner image, and the
  workflow fails if it is missing. Add `pkgconf` to `ROOTS` rather than
  installing it in the workflow.

### Release notes

- A `cx-*` release names the value of `wine-src.ref` and this repository's
  commit. The resolved Wine commit is in the workflow log.
- A `compatdb-*` release names this repository's commit, the base `cx-*`
  release and the first line of that release's notes.

## Wine-side contract

The patched Wine is [athei/wine](https://github.com/athei/wine), at the
branch, tag or commit named by [`wine-src.ref`](../wine-src.ref). Changing
anything below needs a `cx-*` release.

- winemac exports `macdrv_functions`, the window, Metal and monitor helpers
  that D3DMetal and DXMT `dlsym` out of `winemac.so`. Both get the window's
  client view through its `get_win_data(hwnd)` entry, which only finds
  windows the calling process created.
- ntdll exports `__wine_unix_call`, which D3DMetal's PE halves look up at
  run time.
- ntdll loads `libd3dshared.dylib` on the first native PE load and records
  its `__TEXT` range, which routes calls from Apple code to the ms-ABI unix
  call entry; without it they take the sysv entry and crash. The default is
  `<ntdll_dir>/../../external/libd3dshared.dylib`;
  `CX_APPLEGPTK_LIBD3DSHARED_PATH` overrides it.
- ntdll exports for compatdb.so:
  - `prepend_dll_path`, for the Direct3D trees.
  - `add_load_order_override`, for `dll_overrides`.
  - `set_compat_dpi_awareness`, which only stores the value; win32u reads it
    through `ntdll_get_compat_dpi_awareness` when it first sets the process
    awareness.
  - `set_compat_large_address_aware`; ntdll raises a 32-bit process's
    address limit after the library returns.
  - On an ntdll without one of them, compatdb.so logs that and skips the
    setting.
- compatdb.so exports for ntdll: `compatdb_query_x87` (x86_64 library
  only, see [arm64 Wine](compatdb.md#arm64-wine)) and
  `compatdb_query_env`. ntdll calls them in the parent before it starts a
  child, and in the first process of a launch before ntdll is initialized.
  `compatdb_query_env` writes into a buffer ntdll provides, since ntdll may
  `dlclose` the library right after an early query.
- For that early call ntdll loads the library itself and exports
  `compatdb_preinit_query`, which tells the library's initializer to do
  nothing. CrossOver's own ntdll lacks the export, so the initializer runs
  as usual there.
- Under an ntdll without `compatdb_query_env`, `env` entries are still set
  when the library loads into the process, but too late for the x87sidecar
  and Rosetta.
- The x86_64 build's ntdll decides no-execute from the main executable
  alone and keeps it permanently on under Rosetta, unless `WINE_FORCE_NX`
  says otherwise ([#3](https://github.com/athei/wine-build/issues/3)). The
  arm64 Wine keeps Wine's behavior unless `WINE_FORCE_NX` keeps no-execute
  permanently on ([arm64](arm64.md)).
- `wine.inf` registers the fake DLLs D3DMetal needs.

## d3d9 conformance tests

- `build-wine.sh` builds the `d3d9_test.exe` binaries explicitly, and a
  full bundle copies them to `lib/wine/tests/{i386,x86_64}-windows/`, outside
  the directories the loader searches. They are plain PEs.
- Wine's D3D9 tests are the de facto D3D9 conformance suite, so a consumer
  (mtld3d's CI) can gate its own `d3d9.dll` against them with nothing but the
  bundle.

## Upgrade checklist

- `macdrv_functions` is a private contract with no version field. winemac
  asserts the sizes of `struct macdrv_functions_t` and
  `struct d3dmetal_macdrv_win_data`; a mismatch with a newer GPTK or DXMT is
  a crash inside their code, not an error message. Check it on every GPTK,
  DXMT or CrossOver bump.
- Neither D3DMetal nor DXMT can present into another process's window, so a
  CEF GPU process drawing into the browser window still needs
  `--in-process-gpu`.
- A builtin only loads if its placeholder exists in the prefix:
  - wineboot stamps placeholders for what is in `lib/wine/<arch>-windows`
    when the prefix is created.
  - The default directories hold fake-module markers (made with
    `winebuild --fake-module`) for every DLL the trees supply, so a new
    prefix is complete.
  - A builtin name added later (a development install of mtld3d into
    `lib/wine/d3d9/mtld3d/`, or a mod) needs `wineboot -u` only if the name
    is new.
  - A bundle redeploy wipes `lib/wine`, so such additions have to be
    reinstalled after one.
