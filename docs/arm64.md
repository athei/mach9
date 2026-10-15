# Native arm64 build

`--arm64` builds and bundles a native arm64 Wine that runs x86 programs
through [FEX](https://github.com/athei/FEX) instead of Rosetta. It is local
only: no workflow builds it, and the bundle is signed for the machine's own
developer team.

Every difference from the x86_64 build is a value or a flag of the `arm64`
profile in [`profile.sh`](../profile.sh); the scripts test the flags. In
short:

- The Wine source is the arm64 worktree of the Wine checkout, configured
  natively against the arm64 Homebrew with the same allowlist as the x86_64
  build, plus the options only that tree has (`CONFIGURE_FEATURES_EXTRA`).
- FEX replaces Rosetta, so there is no x87sidecar and compatdb.so has no x87
  query. Instead, compatdb's built-in `fex-x87-reduced-precision` rule asks
  FEX for faster x87 emulation
  ([defaults](../README.md#defaults-that-differ-from-wine),
  [arm64 Wine](compatdb.md#arm64-wine)).
- No D3DMetal: Apple ships it for x86_64 only. The trees the bundle carries
  are `DXGI_IMPLS` and `D3D9_IMPLS` in the profile.
- No d3d9 test binaries.
- A [Wine SDK](#wine-sdk) is staged for the PE code built outside Wine.
- The loader is wrapped in a signed app bundle (see [Loader app](#loader-app)).

## Requirements

- An Apple Silicon Mac and a macOS SDK at least as new as the profile's
  `MACOS_FLOOR`.
- The arm64 Homebrew at the profile's `DEPS_PREFIX` with the formulae in
  `DEPS_FORMULAE`, plus `cmake`, `ninja` and `meson`.
- Xcode's Metal toolchain (`xcrun metal`), for DXMT's shaders.
- llvm-mingw at `MINGW_DIR`, as for the x86_64 build, including its arm64ec
  toolchain.
- Rust with the profile's `RUST_TARGET`.
- In the keychain, a code signing identity of the team that
  [`signing/loader.provisionprofile`](../signing/loader.provisionprofile)
  belongs to, whose certificate the profile lists.
- Network access for the bundle step, which resolves `fex-src.ref` and gets
  the loader's secure timestamp.

## Commands

```bash
./build-wine.sh --arm64    # Wine, the SDK, then FEX and DXMT
./bundle-wine.sh --arm64   # assemble the bundle
```

| Script | What it does |
|--------|--------------|
| `build-wine.sh --arm64` | Builds Wine, stages the [Wine SDK](#wine-sdk), then runs `build-fex.sh` and `build-dxmt.sh`; `--clean` is passed on to them |
| [`build-fex.sh`](../build-fex.sh) | Checks out the ref in [`fex-src.ref`](../fex-src.ref) in `FEX_SRC` (refusing a tree with local changes), builds FEX's emulator DLLs and their unix halves, and records the commit. Runs alone too |
| [`build-dxmt.sh`](../build-dxmt.sh) | Builds DXMT from the commit pinned in [`redist.env`](../redist.env) against the Wine build and SDK, and stages it with its commit and `git describe` in `DXMT_DIST`. Runs alone too, after a Wine build |
| `bundle-wine.sh --arm64` | Assembles the bundle from the Wine build, the SDK, FEX, DXMT and the mtld3d release, signs the loader and runs the arm64 smoke test |

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

## Loader app

FEX needs the hardware x86 memory ordering that macOS only grants to a
process signed with `com.apple.developer.cross-architecture-support`. That
entitlement is honoured only through a provisioning profile embedded in an
app bundle and a signature by a certificate of the profile's team.

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
Wine source, its DLLs carrying the recorded version), the llvm-mingw
compilers the smoke test uses, and the provisioning profile (not expired,
granting the entitlement, with an identity of its team in the keychain).

After assembly, besides the self-containment, `minos`, symlink and copied-in
file checks of the x86_64 bundle
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
- The smoke test, with the caller's `WINE_COMPATDB` and `FEX_*` variables
  dropped: `wine --version`; a fresh prefix boots (`cmd /c exit`), with
  `compatdb:` lines and no trace of the x87sidecar; the i386 `cmd.exe` runs;
  and [`smoke/hello.c`](../smoke/hello.c), compiled for each architecture in
  the profile's `SMOKE_PE_ARCHS`, prints its marker and reports whether it
  loads `dxgi.dll` and `d3d9.dll`. Each one's compatdb lines must name the
  trees for its arch (the x86 defaults, or `wined3d` for the native one), and
  the x86 programs must load both DLLs; the native one cannot (see
  [Limitations](#limitations)). Then the x86_64 and i686 programs load every
  module of each Direct3D tree in turn, as for the x86_64 bundle. Every run
  has a time limit.

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
| Wine | The arm64 worktree's checkout. The SDK and DXMT record the commit they were built against (with `-dirty` when tracked files differ, which counts all uncommitted states as one), and the bundle step refuses either when it is not the current one |
| Provisioning profile | [`signing/loader.provisionprofile`](../signing/loader.provisionprofile); replace the file when it expires or the App ID changes |
