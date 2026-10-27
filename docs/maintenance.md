# Maintenance

Internals of the build and bundle scripts, the inputs they pin, and what to
check when something upstream changes.

## Relocation

- `build-wine.sh` patches the `SONAME_LIB*` defines of the libraries the
  allowlist enables (`BUNDLED_SONAMES`) to
  `@loader_path/../../external/<lib>`, so Wine's `.so` modules in
  `lib/wine/aarch64-unix/` find the bundled dylibs in `lib/external/`
  through dyld alone. The placeholder defines configure writes for
  libraries it did not find (`FALLBACK_SONAMES`) stay as they are, and any
  other `SONAME_LIB*` define stops the build: it is a library the allowlist
  does not account for.
- `config.status` silently regenerates `config.h` after a source update, so
  the script checks the patch before every build and again after `make`,
  re-patching and rebuilding when it was lost.
- `bundle-wine.sh` takes the direct dependencies from the patched sonames in
  `config.h` (plus `libSDL3`, which sdl2-compat loads at run time), copies
  them and their `DEPS_PREFIX` closure into `lib/external/`, rewrites their
  install names to `@loader_path/`, re-signs them ad hoc, since arm64 code
  does not load with a broken signature, and fails if any reference to one
  of the `LEAK_PREFIXES` in [`profile.sh`](../bin/lib/profile.sh) survives.
- The closure includes the `@rpath/` dependencies some Homebrew libraries
  have. The step finds each one through the `LC_RPATH` entries of the
  library where Homebrew installed it, copies it and rewrites the reference
  to `@loader_path/` like the others. It then removes every `LC_RPATH`
  entry of the copied libraries that does not name a directory inside the
  bundle, since from `lib/external/` those would point at the bundle's
  parent or at nothing. Step 5 fails on an `@rpath/` dependency that is
  still left and loads every library in `lib/external/` with `dlopen`, so a
  dependency the step missed fails the bundle step instead of Wine
  silently running without the library.
- The Vulkan loader's soname is one of the patched ones, but the bundle
  step copies the loader from `MESA_DIST`, where `build-mesa.sh` staged it
  built for the floor, and not from `DEPS_PREFIX`. KosmicKrisp goes into
  `lib/external/` the same way. The loader finds it through the manifest in
  `share/vulkan/icd.d/`, whose `library_path` is relative to the manifest,
  and ntdll points the loader at that directory
  ([internals.md](internals.md#vulkan)).
- `winegstreamer.so` is the one unix module that links libraries outside
  the system: GStreamer's, which `build-gstreamer.sh` staged in
  `GSTREAMER_DIST` under the install names
  `@loader_path/../../external/<name>`, so the linker records the path the
  module loads each by in the bundle. Wine links its modules without room
  in their headers for longer names, so `install_name_tool` cannot rewrite
  their references afterwards. The bundle step copies the libraries and
  plugins, as they are, into `lib/external/` and
  `lib/external/gstreamer-1.0/`, gives the libraries `@loader_path/<name>`
  as their own install names like every bundled dylib, and stops on a
  reference of `winegstreamer.so` by any other name, or on a GStreamer
  library with the name of one Step 3 already copied from Homebrew
  ([GStreamer](internals.md#gstreamer)).
- There is no `DYLD_*` variable. The loader app is the only entry point
  ([loader app](internals.md#loader-app)).

## Direct3D trees

- Which trees a build has is `DXGI_IMPLS` and `D3D9_IMPLS` in
  [`profile.sh`](../bin/lib/profile.sh), in the order a default-dir
  marker prefers them.
- Each tree under `lib/wine/dxgi/` and `lib/wine/d3d9/` has
  `<arch>-windows/` and, where the implementation has a unix half,
  `aarch64-unix/`. DXMT's and mtld3d's single `.so` also serves the i386 PE
  modules through wow64 entry points. Which modules go into which tree is
  in Step 4 of [`bundle-wine.sh`](../bin/bundle-wine.sh).
- Every module a family's trees supply is removed from the default
  `<arch>-windows/` directory and replaced by a fake-module marker, which
  wineboot turns into the prefix's placeholder. The marker carries no
  exports; only its machine matters.
- An i386 marker is i386. A 64-bit marker serves every 64-bit process and
  every tree of its family, and its machine decides who can load the name:
  an x64 process, which runs as ARM64EC, accepts an AMD64 placeholder and an
  ARM64 one with ARM64EC (CHPE) metadata in its load config, and a native
  ARM64 process accepts only the ARM64 one. compatdb.so gives both kinds
  of process either tree, so every 64-bit name gets such a hybrid marker
  (`winebuild --fake-module -b aarch64-windows -marm64x`), and every 64-bit
  DLL of every tree has to be ARM64X, so that one module serves both kinds
  of process, each from its own half. Step 4 stops on a tree DLL that is not
  (`check_trees_arm64x` in [`bundle-wine.sh`](../bin/bundle-wine.sh)), such
  as an x86_64 build, and Step 5 checks every marker's machine. No AMD64
  marker is stamped any more; a native ARM64 program could not load a name
  with one ([limitations](internals.md#limitations)).
- Beyond the trees, nothing in any `aarch64-windows` directory, the
  default one included, may be code an x64 process would load but not run
  natively. Step 5's `check_pe64_native` in
  [`profile.sh`](../bin/lib/profile.sh) holds every real PE file there to
  an ARM64EC view (ARM64X, or ARM64EC: the AMD64 machine with ARM64EC
  metadata, as FEX's `libarm64ecfex.dll` is). Plain ARM64 passes only for
  programs (`.exe`, `.com`), files without code (no code or executable
  section and no entry point: Wine's resource-only DLLs, `.tlb`,
  `.msstyles`), and the WoW64 host DLLs in `PE64_WOW64_HOST`, which only
  the native half of a 32-bit process loads. A new plain ARM64 DLL with
  code fails the step; if it really is never loaded by an x64 process, it
  goes into that list with the reason.

What the bundle step checks is in
[internals.md](internals.md#what-the-bundle-step-checks).

## macOS floors

| Setting | Defined in | Applies to | Constraint |
|---------|------------|------------|------------|
| `MACOS_FLOOR` | [`profile.sh`](../bin/lib/profile.sh) | Exported as `MACOSX_DEPLOYMENT_TARGET` for everything the scripts build (Wine, `compatdb.so`, FEX, DXMT, KosmicKrisp and the Vulkan loader, GStreamer, though not KosmicKrisp's build tools, which only run on the build host), and the floor every Mach-O in the bundle is checked against | The loader's cross-architecture entitlement needs it; equals the `.cargo/config.toml` row |
| `MACOSX_DEPLOYMENT_TARGET` | [`.cargo/config.toml`](../compatdb/.cargo/config.toml) | `compatdb.so` built by a plain `cargo build` in `compatdb/`, where cargo finds the file; the scripts override it | Same as `MACOS_FLOOR` |

- Without the pin, clang takes the deployment target from the build host.
- Only the minimum is fixed; the SDK is whatever the build host has, and
  must be at least the floor.
- An SDK newer than the floor can declare functions Wine's configure checks
  for that macOS only has above the floor. configure finds them, Wine calls
  them without an availability check, and the linker weak-imports them, so
  on an older macOS the call jumps to address 0. `minos` does not show
  this, but the compiler warns `'<function>' is only available on macOS
  <version> or newer`. Such functions are listed in `FLOOR_NEWER_FUNCS` in
  [`profile.sh`](../bin/lib/profile.sh), and configure is told each is
  missing while the floor is below its version; a change takes effect on
  the next `build-wine.sh --clean`, and until then the bundle step fails on
  a module that still imports one. After a new SDK, search a clean build's
  log for that warning. `build-gstreamer.sh` turns the same functions off
  in the `config.h` files of its meson tree, compiles with that warning as
  an error, and fails on a library that imports one.
- Wine picks up a changed pin on the next `build-wine.sh --clean`.
- A `MACOSX_DEPLOYMENT_TARGET` in the environment may raise `MACOS_FLOOR`
  but not lower it, and overrides the `.cargo/config.toml` value.
- A CI or release build passes `--strict-floor`. A local build copies
  whatever Homebrew has: a bottle carries the macOS it was poured for as
  `minos`, and a formula compiled on that machine carries its macOS, so a
  local bundle can exceed the floor. It is not distributed, so a warning is
  enough. When the libraries meet the floor is in
  [internals.md](internals.md#homebrew-libraries-and-the-floor).

## Pinned inputs

What is pinned, and where, is in [internals.md](internals.md#pinned-inputs).

- `bundle-wine.sh` downloads each artifact in [`redist.env`](../pins/redist.env)
  into `CACHE_DIR` and reuses the cached copy while its checksum matches
  the pin. A fresh download that does not match is an error, never a
  silent retry. To update one, change both its URL and its SHA-256.
  [`bin/ci/llvm-mingw.sh`](../bin/ci/llvm-mingw.sh) handles the llvm-mingw
  of a CI build the same way; to update it, change its version and SHA-256.
- The sources pinned by commit are cloned by the script that builds them,
  which fails when a pinned tag is not at its commit.

## Releases

An `arm64-<Wine version>-<n>` tag makes
[`release.yml`](../.github/workflows/release.yml) build the bundle and
attach it and its symbols to a draft release of the tag. Its steps call the
build and bundle scripts and the scripts in [`bin/ci/`](../bin/ci), each of
which says in its header what it does. The `cx-*` releases are the frozen
x86_64 Wine this repository built before, which runs under Rosetta.

- `release.yml` builds on a hosted runner of the floor's major macOS
  version, from the branch of athei/wine that
  [`wine-src.ref`](../pins/wine-src.ref) names and the pins of the tagged
  commit, and runs the bundle step with `--strict-floor` and `--symbols`.
  The runner starts without build trees; what it takes from earlier runs is
  in [other caches](#other-caches).
- `release.yml` also runs on `main`: on a push that changes a file the
  build reads (its `paths` filter), on a schedule, and by hand. Such a run
  builds and bundles the same and runs `package.sh --notes-only`, which
  runs every check `package.sh` makes before it packs, writes the notes,
  and packs only a small part of each tree with the same code, then lists
  and deletes those archives. A change that breaks `package.sh` shows
  before a tag needs it. Only a tag packs the whole trees. Such a run saves
  its caches for the tag builds and publishes nothing. A newer push to
  `main` cancels the run of `main` still in progress; a run on a tag is
  never cancelled.
- `release.yml` also runs on every pull request to `main`, with no `paths`
  filter, since its check is required for merging and a filtered run would
  never report it. Such a run does what a run on `main` does, in the
  environment `pull-request` ([signing identity](#signing-identity)), and
  waits until the owner approves it there. It restores the caches of
  `main` and saves none. A newer push to the pull request cancels its run
  still in progress. A pull request from a fork gets no secrets, so its run
  fails at its first step, before the build; push the branch to this
  repository instead.
- A release's notes start with one bullet per source it was built from,
  with its ref and commit (`package.sh` in [`bin/ci/`](../bin/ci)). The
  changes are added by hand below them, as short bullets, before the draft
  is published.
- [`docs.yml`](../.github/workflows/docs.yml) publishes the rustdoc of
  `compatdb-core` to
  [GitHub Pages](https://athei.github.io/wine-build/compatdb_core/); its
  header says when it runs, what it checks and the repository setting
  Pages needs.

### Homebrew formulae

`release.yml` bundles the newest versions Homebrew has, not the ones the
runner image came with. The steps are the local action
[`.github/actions/brew`](../.github/actions/brew/action.yml), and the
commands and the manifest are described in the header of
[`brew.sh`](../bin/ci/brew.sh).

- Every run updates Homebrew's formula data first and writes a manifest of
  the formulae in [`brew-formulae`](../pins/brew-formulae) and everything
  they need at run time, each with the version and bottle Homebrew has and
  what the image has installed of it. A formula without a bottle for the
  runner's macOS stops the run there, since building it from source would
  take hours.
- The kegs a run adds to the image are cached under a key that ends in the
  manifest's hash. A run whose manifest has a cache unpacks it and installs
  nothing. Otherwise it unpacks the newest cache made by the same scripts,
  leaving out the kegs that are older than the manifest's versions or of
  formulae the manifest no longer names, then upgrades and installs only
  the formulae of the manifest that are still behind or missing, never the
  rest of the image's, and saves the result under its own key.
- Either way the step fails when a formula of the manifest is not installed
  and linked at the version it names, or `brew outdated` names one. A
  bottle Homebrew rebuilt at the same version is only poured where the
  runner lacks the formula; Homebrew ships fixes as new versions or
  revisions, which this check catches.
- [`brew-cache.yml`](../.github/workflows/brew-cache.yml) runs the same
  steps on the default branch every day, on demand, and on a push to
  `main` that changes the scripts the key hashes. Caches made there can be
  read by the runs on every tag and pull request, so a release build usually
  finds the kegs of its own manifest, or of a recent one, and downloads
  little. A run of `release.yml` on a pull request passes the action's
  `save` input as false, so it saves no kegs.
- GitHub disables the scheduled workflows of a public repository after 60
  days without activity in it; re-enable `brew-cache.yml` in the Actions
  tab then.

### Other caches

The runner also caches the staged directories of Mesa, GStreamer, FEX,
DXMT and vkd3d-proton, the compiler output of ccache, DXMT's LLVM
libraries, the downloads `redist.env` pins, cargo's registry and the Xcode
`host.sh` chose.
Each key changes only with what its cache was made from:

- Each staged directory: the runner image and the hash of the `KEY` line
  its build script would write into it, which the script prints with
  `--print-key` ([commands](internals.md#commands)). That line names the commit
  the component's ref resolves to now (for GStreamer the pinned one), the
  hash of the build script and of [`profile.sh`](../bin/lib/profile.sh),
  the versions of the compilers,
  the macOS SDK and the tools the script records, and for DXMT the Wine
  commit and the hash of `build-wine.sh`. A directory restored under its
  key is complete and current, so the script builds nothing; any change of
  those makes a new key, the script builds, and the run saves the result
  under the hash of the `KEY` that build wrote. The script compares the
  whole `KEY` before it skips anything, so a directory built by another
  script or toolchain is built again; the bundle step compares each
  directory's commits (`COMMIT`, and Mesa's `LOADER_COMMIT`, GStreamer's
  `FFMPEG_COMMIT` and DXMT's `WINE_COMMIT`) with the pins and the Wine
  source once more.
- ccache's directory: the runner image, then the run's ID and attempt.
  Every run restores the newest directory a run on the default branch saved
  (the key's prefix), so the objects of every source file that did not
  change are reused even when a commit changed. Only a run on the default
  branch saves its own, right after the build step, even when that fails:
  a tag run's could serve later runs on that tag only, and each directory
  is a full one, so saving every run's would crowd the repository's cache
  out. ccache itself checks each object against the compiler, its
  arguments and the sources, and `CCACHE_MAXSIZE` in `release.yml` bounds
  the directory. Its statistics for the run follow the build step.
- DXMT's LLVM: the LLVM pin in [`redist.env`](../pins/redist.env), the
  floor, the Xcode version and the runner image. A change to how
  `build-dxmt.sh` builds LLVM does not change it, just as a local build
  keeps its LLVM while the pinned commit stays the same. To rebuild LLVM
  after such a change, delete the `dxmt-llvm15-*` caches
  (`gh cache list --key dxmt-llvm15-`, then `gh cache delete <id>`) or
  change the pin.
- The downloads: the pins of the downloads the cache holds, the ones the
  `pins-key.sh` call for this cache in `release.yml` names
  ([`pins-key.sh`](../bin/ci/pins-key.sh) hashes their lines of
  `redist.env`). A new download pinned in `redist.env` needs its prefix
  added to that call.
- cargo's registry: `compatdb/Cargo.lock`.
- The Xcode choice: the runner image and its version, and the hashes of
  [`host.sh`](../bin/ci/host.sh) and [`profile.sh`](../bin/lib/profile.sh).
  `host.sh` searches again when the Xcode it names is gone;
  `brew-cache.yml` uses the same key.

A run on a tag writes caches that only later runs on the same tag can read,
so a failed release that is run again reuses them; it writes no ccache
directory. A run on `main` writes caches every run can read, so a tag build
builds only the components that changed since the last run on `main` and
compiles with that run's ccache directory. A push that changes a pin or a
script starts such a run. A new commit on a branch a pin names, such as the
Wine branch, does not; the next scheduled run picks it up, or start a run
on `main` by hand before tagging. GitHub evicts caches that no run used for
a week; the scheduled runs use the current ones more often than that, so
they outlast a quiet week, but only up to the 60 days without activity
after which GitHub disables the scheduled workflows of a public
repository; re-enable `release.yml` in the Actions tab then, as
`brew-cache.yml`.

A run on a pull request reads the caches of `main` and writes none, not
even the Xcode choice or the Homebrew kegs, so what a change that is not
merged built never reaches another run, and a pull request cannot fill the
repository's cache budget.

### Signing identity

The loader is signed in `release.yml`, by the identity of the
[provisioning profile](internals.md#loader-app)'s team, from the secrets of the
GitHub environment `release`, or of the environment `pull-request` in a run
on a pull request. `bin/ci/keychain.sh` imports it into a keychain of its
own for the bundle step and deletes that keychain at the end of the job.

- Create the environment `release` in the repository settings, and limit its
  deployments to the tag pattern `arm64-*` and the branch `main`, whose
  builds sign the loader too. A run from any other ref cannot use the
  secrets, so a pull request that edits the job's `environment` line to
  name `release` gets none.
- Create the environment `pull-request` with the owner as its required
  reviewer, so each pull request run waits for an approval before it gets
  the secrets. It holds a copy of the same two secrets. A pull request from
  a fork gets no secrets from either environment.
- Export the identity: in Keychain Access, under My Certificates, select the
  "Developer ID Application" certificate of the profile's team with its
  private key (it must be one of the certificates the profile lists, which
  the bundle step checks), choose File > Export Items, save it as a `.p12`
  file and give it a password.
- Store both as secrets of both environments, and delete the file:

  ```bash
  base64 -i identity.p12 | gh secret set MACOS_SIGNING_P12 --env release
  gh secret set MACOS_SIGNING_P12_PASSWORD --env release
  base64 -i identity.p12 | gh secret set MACOS_SIGNING_P12 --env pull-request
  gh secret set MACOS_SIGNING_P12_PASSWORD --env pull-request
  rm identity.p12
  ```

  The commands that set the password ask for it.
- When the certificate is renewed, or the profile replaced with one that
  lists another certificate, export and store the new identity in both
  environments the same way.

## Wine-side contract

The patched Wine is the
[`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) branch
of athei/wine. The scripts and `compatdb.so` rely on it for:

- ntdll dlopens `<ntdll_dir>/compatdb.so` in every process before any PE
  code runs, and exports for it:
  - `prepend_dll_path`, for the Direct3D trees.
  - `add_load_order_override`, for `dll_overrides`.
  - `set_compat_dpi_awareness`, which only stores the value; win32u reads it
    through `ntdll_get_compat_dpi_awareness` when it first sets the process
    awareness.
  - `set_compat_large_address_aware`; ntdll raises a 32-bit process's
    address limit after the library returns.
  - `set_compat_gcmouse`, which only stores the value; winemac reads it
    through `ntdll_get_compat_gcmouse` when it reads its settings, to decide
    whether raw mouse input comes from GameController
    ([`Rule::gcmouse`](https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.gcmouse)).
  - `set_compat_force_nx`, which only stores the value; ntdll reads it when
    the process asks for or changes its execute flags and, for 1, keeps
    no-execute permanently on
    ([`Rule::force_nx`](https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.force_nx)).
  - `set_compat_opengl`, which only stores the value; winemac reads it
    through `ntdll_get_compat_opengl` when the process first uses OpenGL,
    and uses Mesa's EGL for 1 and Apple's OpenGL otherwise
    ([`Rule::opengl`](https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.opengl)).
  - On an ntdll without one of them, compatdb.so logs that and skips the
    setting. The bundle step checks that `ntdll.so` exports
    `set_compat_force_nx` and `set_compat_opengl`
    ([what the bundle step checks](internals.md#what-the-bundle-step-checks)).
- winemac exports `macdrv_functions`, the window, Metal and monitor helpers
  DXMT's `winemetal.so` uses. It gets the window's client view through its
  `get_win_data(hwnd)` entry, which only finds windows the calling process
  created.
- ntdll sets `VK_DRIVER_FILES` to the bundle's `share/vulkan/icd.d` unless
  the caller chose the Vulkan drivers ([Vulkan](internals.md#vulkan)); the
  bundle step checks that `ntdll.so` names the variable.
- ntdll points GStreamer at the bundle's `lib/external/gstreamer-1.0` and
  its registry at a file of its own unless the caller chose them
  ([GStreamer](internals.md#gstreamer)); the smoke test checks that the
  registry lands there.
- The WMA decoder DMO's stream info and input status calls return `S_OK`;
  the smoke test's `wmadec.c` checks them.
- The wineserver runs CrossOver's msync and prints the lines the smoke test
  looks for ([runtime](internals.md#runtime)).
- The loader checks for its entitlement and re-execs itself with 4K pages
  ([loader app](internals.md#loader-app)).

## Upgrade checklist

- `macdrv_functions` is a private contract with no version field. winemac
  asserts the layout of its function table and of the window data it hands
  out (in
  [`dlls/winemac.drv/window.c`](https://github.com/athei/wine/blob/arm64-patched/dlls/winemac.drv/window.c));
  a mismatch with a newer DXMT is a crash inside its code, not an error
  message. Check it on every DXMT bump.
- DXMT cannot present into another process's window, so a CEF GPU process
  drawing into the browser window still needs `--in-process-gpu`.
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
  - Modules a new bundle adds, such as `vulkan-1.dll` and `winevulkan.dll`
    in the first bundle with Vulkan, reach an existing prefix without
    `wineboot -u`. `make install` gives `wine.inf` a new modification time,
    and wineboot, which the first process of every session runs, installs
    `wine.inf` again whenever that time differs from the one in the
    prefix's `.update-timestamp`, stamping placeholders for every builtin.
    Only a prefix whose `.update-timestamp` says `disable` needs
    `wineboot -u` then.

### Mesa and the Vulkan loader

- Mesa is a fork: upstream Mesa plus our topic commits, originally from
  Hadron's patches, at the ref in [`mesa-src.ref`](../pins/mesa-src.ref)
  ([internals.md](internals.md#vulkan)). To move it to a newer Mesa main:
  - Rebase the topic series onto the new upstream main with `git rerere`
    enabled, so a conflict resolved once is resolved again on the next
    rebase.
  - Drop the commits upstream has merged in the meantime, such as those of
    an upstream merge request the series carries under its original author,
    and resolve what their upstream form changes in the commits on top of
    them.
  - Check the always-on feature list: device-generated commands, transform
    feedback and sparse resources must still be advertised without
    `MESA_KK_EXPERIMENTAL` (in `kk_get_device_extensions` and the sparse
    features of the driver's physical device). An upstream change that
    puts one of them behind an experimental flag again turns it off
    without a word; the bundle's Vulkan smoke test only notices it for
    transform feedback, so check the other two in the `vulkaninfo`
    comparison below.
  - Check that the two-stage build still holds: the stage 1 targets and
    options and the stage 2 options in
    [`build-mesa.sh`](../bin/build-mesa.sh), the disabled optional
    dependencies included, must still exist under those names, and the
    driver must still pass the script's checks (system libraries only, no
    flat-namespace imports, `minos` at the floor). A new optional
    dependency in Mesa's stage 2 meson log gets disabled there too, and a
    program found there ("Program ... found: YES") must be one the native
    file names, the Python environment's or the system's, never Homebrew's.
  - The same holds for EGL and Zink, built in stage 2 with the gallium,
    EGL and GL options there: `libEGL.1.dylib` and exactly one
    `libgallium-*.dylib` must still be installed, and Zink must still load
    the Vulkan loader by `@rpath` (the script checks the string and the
    `@loader_path` `LC_RPATH`). Without
    transform feedback Zink reports OpenGL 2.1, which the bundle's WGL
    smoke test fails on.
  - Keep a copy of `MESA_DIST`, rebuild with `build-mesa.sh`, and compare
    `vulkaninfo` for the old and the new driver, each with
    `VK_DRIVER_FILES` naming its own manifest and no
    `MESA_KK_EXPERIMENTAL`: extensions, features and limits that disappear
    are a regression to explain before the bundle takes the new driver.
  - Push the branch to athei/mesa before building without `MESA_REPO`
    pointing at the local clone; the default builds what the remote has.
- The Vulkan loader and headers are pinned together, from one Vulkan SDK
  release, in [`redist.env`](../pins/redist.env). Bump both tags and commits
  together; `build-mesa.sh` refuses a tag that is not at its commit.
- A Wine build tree configured without Vulkan has no `SONAME_LIBVULKAN`,
  and one configured before EGL was built has no `SONAME_LIBEGL`;
  `build-wine.sh` stops and asks for `--clean`.

### GStreamer

GStreamer, with glib and FFmpeg, is pinned in
[`redist.env`](../pins/redist.env) ([GStreamer](internals.md#gstreamer)). On
a bump of the tag:

- Pin the commit of the FFmpeg port branch that the new tag's
  `subprojects/FFmpeg.wrap` names, branch and commit together;
  `build-gstreamer.sh` refuses a wrap that names another branch.
- Check that every meson option the script sets still exists under its
  name: the monorepo's and each module's `meson.options`, and FFmpeg's
  `meson_options.txt` for the decoders, parsers and bitstream filters.
  The script fails on a decoder, parser or filter missing from FFmpeg's
  `config_components.h` and on a plugin set that differs from its list,
  but an option that disappeared in a module meson accepts silently.
- A new optional dependency in the meson log that auto_features does not
  turn off gets disabled in the script too, and a new decoder an
  existing one builds on (FFmpeg's `configure`, `*_select`) goes into its
  list.
- Search the build's output for `pipe2` and the other functions of
  `FLOOR_NEWER_FUNCS`: a module whose `config.h` names one under another
  define than `HAVE_<FUNCTION>` fails the build with the availability
  error, and needs that define turned off as well.
- Rebuild with `build-gstreamer.sh` and run a full bundle step; its smoke
  test creates the elements `winegstreamer` needs and decodes WMA through
  Wine's DMO.
- A Wine build tree configured without GStreamer has an empty
  `GSTREAMER_LIBS` in its `Makefile`; `build-wine.sh` stops and asks for
  `--clean`.
- `build-wine.sh` compiles and links `winegstreamer` again whenever the
  dist's `KEY` differs from the one it was last built against, and the
  bundle step refuses a `winegstreamer.so` built against another dist.

### DXMT and vkd3d-proton

Both are forks: upstream plus our commits, at the refs in
[`dxmt-src.ref`](../pins/dxmt-src.ref) and
[`vkd3d-proton-src.ref`](../pins/vkd3d-proton-src.ref)
([DXMT](internals.md#dxmt), [vkd3d-proton](internals.md#vkd3d-proton)). They are
upgraded together when the swapchain interface between them changes.

- To move either to a newer upstream main (DXMT) or master (vkd3d-proton),
  rebase our commits onto it with `git rerere` enabled, as for Mesa, and
  drop the ones upstream has merged in the meantime.
- DXMT's `dxgi.dll` presents vkd3d-proton's swapchains through the
  interfaces of vkd3d-proton's `include/vkd3d_swapchain_factory.idl`, of
  which DXMT carries its own copy. A change to that file upstream has to be
  made in DXMT's copy too, in the same upgrade; neither build notices a
  mismatch, which shows as a crash or a failed swapchain at run time.
- DXMT: check `macdrv_functions` ([upgrade checklist](#upgrade-checklist)),
  that `enable_d3d12` still exists in its `meson.options`, and that the
  LLVM version in [`redist.env`](../pins/redist.env) is still the one
  DXMT's airconv wants.
- vkd3d-proton: check that `build-arm64x.txt`, `build-win32.txt` and the
  `enable_tests` option still exist, that the submodules still build with
  llvm-mingw, and that the DLLs still export what `build-vkd3d-proton.sh`
  checks. A new tool its build runs belongs in
  [`brew-formulae`](../pins/brew-formulae).
- Push the branch to athei/dxmt or athei/vkd3d-proton before building
  without `DXMT_REPO` or `VKD3D_PROTON_REPO` pointing at the local clone;
  the default builds what the remote has.
- Run a full bundle step: its smoke test runs `d3d12.exe` on every arch.
  Run Wine's `d3d12_test.exe`, `dxgi_test.exe` and `d3d11_test.exe` from
  the bundle's [graphics tests](internals.md#graphics-tests) before and after,
  and explain any new failure before the bundle takes the new build.
