# Maintenance

Relocation, the Direct3D trees, floors, pins, CI, and what to check when
something upstream changes.

## Relocation

The bundle runs from any path through dyld alone. There is no `DYLD_*`
variable; the loader app is the only entry point
([loader app](internals.md#loader-app)).

- `build-wine.sh` patches the `SONAME_LIB*` defines of the bundled
  libraries to `@loader_path/../../external/<lib>`, stops on a define the
  allowlist does not account for, and re-patches when `config.status`
  regenerates `config.h` (its soname step).
- Step 3 of [`bundle-wine.sh`](../bin/bundle-wine.sh) copies those
  libraries and their Homebrew closure, `@rpath/` dependencies included,
  into `lib/external/`, rewrites their install names to `@loader_path/`,
  drops `LC_RPATH` entries outside the bundle and re-signs them ad hoc.
- The Vulkan loader, KosmicKrisp, EGL and the gallium library come from
  `MESA_DIST`, not from Homebrew ([Vulkan](internals.md#vulkan)).
- `winegstreamer.so` links GStreamer by the install names
  `GSTREAMER_DIST` gives it. Wine links its modules without room for
  `install_name_tool` to rewrite them, so the step stops on any other name
  ([GStreamer](internals.md#gstreamer)).
- Step 5 fails on a reference to `LEAK_PREFIXES`, a leftover `@rpath/`
  dependency or a dylib that does not `dlopen`, so a missed dependency
  fails the bundle step instead of Wine silently running without it.

## Direct3D trees

Each tree under `lib/wine/dxgi/` and `lib/wine/d3d9/` holds one
implementation; `DXGI_IMPLS` and `D3D9_IMPLS` in
[`profile.sh`](../bin/lib/profile.sh) list them, and Step 4 of
[`bundle-wine.sh`](../bin/bundle-wine.sh) fills them.

- Every module a family's trees supply is replaced in the default
  `<arch>-windows/` directory by a fake-module marker, which wineboot
  turns into the prefix's placeholder. Only the marker's machine matters.
- Every 64-bit marker is hybrid (ARM64 with ARM64EC metadata), since
  compatdb.so gives x64 and native ARM64 processes either tree and a
  native ARM64 process rejects an AMD64 placeholder
  ([limitations](internals.md#limitations)). So every 64-bit tree DLL has
  to be ARM64X (`check_trees_arm64x`); Step 4's comment has the details.
- Nothing in any `aarch64-windows` directory may be code an x64 process
  would load but not run natively (`check_pe64_native` in `profile.sh`).
  A new plain ARM64 DLL with code that no x64 process loads goes into
  `PE64_WOW64_HOST` with the reason.

What the bundle step checks is in
[internals.md](internals.md#what-the-bundle-step-checks).

## macOS floors

| Setting | Defined in | Applies to | Constraint |
|---------|------------|------------|------------|
| `MACOS_FLOOR` | [`profile.sh`](../bin/lib/profile.sh) | Exported as `MACOSX_DEPLOYMENT_TARGET` for everything the scripts build except KosmicKrisp's build tools, and the floor every Mach-O in the bundle is checked against | The loader's cross-architecture entitlement needs it; equals the `.cargo/config.toml` row |
| `MACOSX_DEPLOYMENT_TARGET` | [`.cargo/config.toml`](../compatdb/.cargo/config.toml) | `compatdb.so` built by a plain `cargo build` in `compatdb/`; the scripts override it | Same as `MACOS_FLOOR` |

- Without the pin, clang takes the deployment target from the build host.
- Only the minimum is fixed. The SDK is whatever the build host has, at
  least the floor.
- A changed pin reaches Wine on the next `build-wine.sh --clean`.
- A `MACOSX_DEPLOYMENT_TARGET` in the environment may raise `MACOS_FLOOR`
  but not lower it, and overrides the `.cargo/config.toml` value.
- A newer SDK can declare functions macOS has only above the floor. Wine
  then calls them weak-imported, and on an older macOS the call jumps to
  address 0; `minos` does not show this. `FLOOR_NEWER_FUNCS` in
  `profile.sh` lists such functions and hides them from configure while
  the floor is below their version. A change needs a clean Wine build
  (`build-wine.sh --clean`); until then the bundle step fails on a module
  that imports one. `build-gstreamer.sh` handles the same list.
- After a new SDK, search a clean build's log for the compiler warning
  `'<function>' is only available on macOS <version> or newer`.
- A CI or release build passes `--strict-floor`. A local bundle can exceed
  the floor through Homebrew's libraries and only warns
  ([Homebrew libraries and the floor](internals.md#homebrew-libraries-and-the-floor)).

## Pinned inputs

What is pinned, and where, is in [internals.md](internals.md#pinned-inputs).

- `bundle-wine.sh` downloads each artifact in
  [`redist.env`](../pins/redist.env) into `CACHE_DIR` and reuses it while
  its checksum matches; a fresh download that does not match is an error.
  To update one, change both its URL and its SHA-256.
- [`bin/ci/llvm-mingw.sh`](../bin/ci/llvm-mingw.sh) does the same for the
  llvm-mingw of a CI build; to update it, change its version and SHA-256.
- The script that builds a source pinned by tag and commit fails when the
  tag is not at the commit.

## Releases

An `arm64-<Wine version>-<n>` tag makes
[`release.yml`](../.github/workflows/release.yml) build the bundle and
attach it and its symbols to a draft release of the tag. Its steps call the
build and bundle scripts and the scripts in [`bin/ci/`](../bin/ci), each of
which says in its header what it does. The `cx-*` releases are the frozen
x86_64 Wine this repository built before, which runs under Rosetta.

| Trigger | What the run does |
|---------|-------------------|
| `arm64-*` tag | Builds, packs the trees and attaches them to a draft release; never cancelled |
| Push to `main` that changes a file the build reads, the schedule, by hand | Builds and bundles, runs `package.sh --notes-only`, saves its caches, publishes nothing |
| Pull request to `main` (no `paths` filter, since its check is required) | As on `main`, in the environment `pull-request` after the owner approves it ([signing identity](#signing-identity)); saves no cache |

- Every run is on a hosted runner of the floor's major macOS version,
  from the branch [`wine-src.ref`](../pins/wine-src.ref) names, with
  `--strict-floor` and `--symbols`. The header and step comments of
  `release.yml` have the details.
- A pull request from a fork gets no secrets and fails at its first step;
  push the branch to this repository instead.
- A release's notes start with one bullet per source it was built from
  (`package.sh`). Add the changes by hand below them, as short bullets,
  before publishing the draft.
- [`docs.yml`](../.github/workflows/docs.yml) publishes the rustdoc of
  `compatdb-core` to
  [GitHub Pages](https://athei.github.io/wine-build/compatdb_core/); its
  header says when it runs and what Pages needs.

### Homebrew formulae

`release.yml` bundles the newest versions Homebrew has, not the ones the
runner image came with. The steps are the local action
[`.github/actions/brew`](../.github/actions/brew/action.yml); the header of
[`brew.sh`](../bin/ci/brew.sh) describes the manifest, the keg cache and
the check.

- A formula without a bottle for the runner's macOS stops the run, since
  building it from source would take hours.
- The step fails when a formula of the manifest is not installed and
  linked at its version, or `brew outdated` names one.
- [`brew-cache.yml`](../.github/workflows/brew-cache.yml) keeps the keg
  cache current from the default branch, so a release build usually
  downloads little. A pull request saves no kegs.

### Other caches

| Cache | Key | Saved by |
|-------|-----|----------|
| Each staged directory | Runner image and the hash of its `KEY` ([commands](internals.md#commands)) | Runs on tags and `main`, under the `KEY` the build wrote |
| ccache directory | Runner image, run ID and attempt; every run restores the newest by prefix | Runs on `main` only |
| DXMT's LLVM | The LLVM pin, the floor, the Xcode version and the runner image | Runs on tags and `main` |
| Pinned downloads | The `redist.env` lines that `pins-key.sh` hashes for it in `release.yml` | Runs on tags and `main` |
| cargo's registry | `compatdb/Cargo.lock` | Runs on tags and `main` |
| Xcode choice | Runner image, its version, and the hashes of `host.sh` and `profile.sh`; `brew-cache.yml` shares it | Runs on tags and `main` |

A run on a tag writes caches only later runs on that tag can read. A run
on `main` writes caches every run can read, so a tag build builds only what
changed since the last run on `main`. A pull request reads `main`'s caches
and writes none.

- The LLVM key does not change with how `build-dxmt.sh` builds LLVM. To
  rebuild it after such a change, delete the `dxmt-llvm15-*` caches
  (`gh cache list --key dxmt-llvm15-`, then `gh cache delete <id>`) or
  change the pin.
- A new download pinned in `redist.env` needs its prefix added to the
  `pins-key.sh` call for the downloads cache in `release.yml`.
- A new commit on a branch a pin names, such as the Wine branch, starts no
  run. The next scheduled run picks it up; or start a run on `main` by
  hand before tagging.
- GitHub evicts caches no run used for a week, which the scheduled runs
  prevent, and disables the scheduled workflows of a public repository
  after 60 days without activity. Re-enable `release.yml` and
  `brew-cache.yml` in the Actions tab then.

### Signing identity

`release.yml` signs the loader with the identity of the
[provisioning profile](internals.md#loader-app)'s team, from the secrets of
the GitHub environment `release`, or of `pull-request` on a pull request.
`bin/ci/keychain.sh` imports it into a keychain of its own for the bundle
step and deletes that keychain at the end of the job.

- Create the environment `release` and limit its deployments to the tag
  pattern `arm64-*` and the branch `main`. A pull request that edits the
  job's `environment` line to name `release` then gets no secrets.
- Create the environment `pull-request` with the owner as its required
  reviewer, so each pull request run waits for approval. It holds a copy
  of the same two secrets.
- Export the identity: in Keychain Access, under My Certificates, select
  the "Developer ID Application" certificate of the profile's team (one
  the profile lists) with its private key, choose File > Export Items, and
  save it as a `.p12` file with a password.
- Store it in both environments and delete the file:

  ```bash
  base64 -i identity.p12 | gh secret set MACOS_SIGNING_P12 --env release
  gh secret set MACOS_SIGNING_P12_PASSWORD --env release
  base64 -i identity.p12 | gh secret set MACOS_SIGNING_P12 --env pull-request
  gh secret set MACOS_SIGNING_P12_PASSWORD --env pull-request
  rm identity.p12
  ```

  The commands that set the password ask for it.
- Repeat this when the certificate is renewed or the profile is replaced
  with one that lists another certificate.

## Wine-side contract

The scripts and `compatdb.so` rely on these parts of the
[`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) branch
of athei/wine.

ntdll dlopens `<ntdll_dir>/compatdb.so` in every process before any PE
code runs, and exports for it:

| Export | What happens with the value |
|--------|-----------------------------|
| `prepend_dll_path` | The Direct3D trees |
| `add_load_order_override` | `dll_overrides` |
| `set_compat_dpi_awareness` | Stored; win32u reads it through `ntdll_get_compat_dpi_awareness` when it first sets the process awareness |
| `set_compat_large_address_aware` | ntdll raises a 32-bit process's address limit after the library returns |
| `set_compat_gcmouse` | Stored; winemac reads it through `ntdll_get_compat_gcmouse` with its settings ([`Rule::gcmouse`][gcmouse]) |
| `set_compat_force_nx` | Stored; ntdll reads it when the process asks for or changes its execute flags ([`Rule::force_nx`][force_nx]) |
| `set_compat_opengl` | Stored; winemac reads it through `ntdll_get_compat_opengl` when the process first uses OpenGL ([`Rule::opengl`][opengl]) |

On an ntdll without one of them, compatdb.so logs that and skips the
setting; the bundle step checks the `force_nx` and `opengl` hooks
([what the bundle step checks](internals.md#what-the-bundle-step-checks)).

- winemac exports `macdrv_functions`, the window, Metal and monitor
  helpers DXMT's `winemetal.so` uses. Its `get_win_data(hwnd)` entry only
  finds windows the calling process created.
- ntdll sets `VK_DRIVER_FILES` ([Vulkan](internals.md#vulkan)) and the
  GStreamer variables ([GStreamer](internals.md#gstreamer)) unless the
  caller chose them; the bundle step checks both.
- The WMA decoder DMO's stream info and input status calls return `S_OK`;
  `smoke/wmadec.c` checks them.
- The wineserver runs CrossOver's msync and prints the lines the smoke
  test looks for ([runtime](internals.md#runtime)).
- The loader checks for its entitlement and re-execs itself with 4K pages
  ([loader app](internals.md#loader-app)).

[gcmouse]: https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.gcmouse
[force_nx]: https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.force_nx
[opengl]: https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.opengl

## Upgrade checklist

- `macdrv_functions` is a private contract with no version field. winemac
  asserts the layout of its function table and window data (in
  [`dlls/winemac.drv/window.c`](https://github.com/athei/wine/blob/arm64-patched/dlls/winemac.drv/window.c));
  a mismatch with a newer DXMT is a crash inside DXMT, not an error
  message. Check it on every DXMT bump.
- DXMT cannot present into another process's window, so a CEF GPU process
  drawing into the browser window still needs `--in-process-gpu`.
- A builtin only loads if its placeholder exists in the prefix. wineboot
  stamps placeholders for `lib/wine/<arch>-windows`, whose markers cover
  every DLL the trees supply, so a new prefix is complete.
  - A builtin name added later (a development install of mtld3d into
    `lib/wine/d3d9/mtld3d/`, or a mod) needs `wineboot -u` only if the
    name is new. A bundle redeploy wipes `lib/wine`, so reinstall it after
    one.
  - Modules a new bundle adds reach an existing prefix without
    `wineboot -u`: `make install` gives `wine.inf` a new modification
    time, and the first process of every session reinstalls it when that
    time differs from the prefix's `.update-timestamp`. Only a prefix whose
    `.update-timestamp` says `disable` needs `wineboot -u`.

### Mesa and the Vulkan loader

Mesa is our fork at the ref in [`mesa-src.ref`](../pins/mesa-src.ref)
([Vulkan](internals.md#vulkan)). To move it to a newer Mesa main:

- Rebase the topic series onto the new upstream main with `git rerere`
  enabled, so a conflict resolved once is resolved again next time.
- Drop the commits upstream has merged, such as those of an upstream merge
  request carried under its original author, and resolve what their
  upstream form changes in the commits on top.
- Check that device-generated commands, transform feedback and sparse
  resources are still advertised without `MESA_KK_EXPERIMENTAL` (in
  `kk_get_device_extensions` and the sparse features of the physical
  device). An upstream change that puts one behind an experimental flag
  turns it off silently; the Vulkan smoke test only notices transform
  feedback.
- Check that the stage 1 targets and options and the stage 2 options in
  [`build-mesa.sh`](../bin/build-mesa.sh), the disabled optional
  dependencies included, still exist under those names. Disable a new
  optional dependency from the stage 2 meson log there too. A program the
  log finds ("Program ... found: YES") must be the native file's, the
  Python environment's or the system's, never Homebrew's.
- `libEGL.1.dylib` and exactly one `libgallium-*.dylib` must still be
  installed, and Zink must still load the Vulkan loader by `@rpath`; the
  script checks both.
- Keep a copy of `MESA_DIST`, rebuild with `build-mesa.sh`, and compare
  `vulkaninfo` for the old and the new driver, each with `VK_DRIVER_FILES`
  naming its own manifest and no `MESA_KK_EXPERIMENTAL`. Extensions,
  features and limits that disappear are a regression to explain before
  the bundle takes the new driver.
- Push the branch to athei/mesa before building without `MESA_REPO`
  pointing at the local clone; the default builds what the remote has.
- The Vulkan loader and headers are pinned together, from one Vulkan SDK
  release, in [`redist.env`](../pins/redist.env). Bump both tags and
  commits together.
- A Wine build tree configured without Vulkan has no `SONAME_LIBVULKAN`,
  and one configured before EGL was built has no `SONAME_LIBEGL`;
  `build-wine.sh` stops and asks for `--clean`.

### GStreamer

GStreamer, with glib and FFmpeg, is pinned in
[`redist.env`](../pins/redist.env) ([GStreamer](internals.md#gstreamer)).
On a bump of the tag:

- Pin the commit of the FFmpeg port branch the new tag's
  `subprojects/FFmpeg.wrap` names, branch and commit together.
- Check that every meson option the script sets still exists: the
  monorepo's and each module's `meson.options`, and FFmpeg's
  `meson_options.txt`. The script catches a missing decoder, parser or
  filter and a changed plugin set, but meson accepts an option that
  disappeared from a module silently.
- Disable in the script a new optional dependency the meson log shows
  that auto_features does not turn off, and add to its lists a new decoder
  an existing one builds on (FFmpeg's `configure`, `*_select`).
- A module whose `config.h` names a function of `FLOOR_NEWER_FUNCS` under
  another define than `HAVE_<FUNCTION>` fails the build with the
  availability error; turn that define off as well.
- Rebuild with `build-gstreamer.sh` and run a full bundle step; its smoke
  test creates the elements `winegstreamer` needs and decodes WMA.
- A Wine build tree configured without GStreamer has an empty
  `GSTREAMER_LIBS` in its `Makefile`; `build-wine.sh` stops and asks for
  `--clean`.
- `build-wine.sh` relinks `winegstreamer` whenever the dist's `KEY`
  changes, and the bundle step refuses a `winegstreamer.so` built against
  another dist.

### DXMT and vkd3d-proton

Both are forks at the refs in [`dxmt-src.ref`](../pins/dxmt-src.ref) and
[`vkd3d-proton-src.ref`](../pins/vkd3d-proton-src.ref)
([DXMT](internals.md#dxmt), [vkd3d-proton](internals.md#vkd3d-proton)),
upgraded together when the swapchain interface between them changes.

- To move either to a newer upstream main (DXMT) or master
  (vkd3d-proton), rebase our commits with `git rerere` enabled, as for
  Mesa, and drop the ones upstream has merged.
- DXMT carries its own copy of vkd3d-proton's
  `include/vkd3d_swapchain_factory.idl`, through which its `dxgi.dll`
  presents vkd3d-proton's swapchains. An upstream change to that file has
  to go into DXMT's copy in the same upgrade; neither build notices a
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
  the bundle's [graphics tests](internals.md#graphics-tests) before and
  after, and explain any new failure before the bundle takes the new
  build.
