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
| `MACOSX_DEPLOYMENT_TARGET` | [`.cargo/config.toml`](../.cargo/config.toml) | Every crate built by a plain `cargo build` in the repository; the scripts override it for `compatdb.so` | Same as `MACOS_FLOOR` |
| `LSMinimumSystemVersion` | [`launcher/app/Info.plist`](../launcher/app/Info.plist), [`launcher/docktile/Info.plist`](../launcher/docktile/Info.plist) | Mach9.app and its Dock tile plug-in | Same as `MACOS_FLOOR` |

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
  llvm-mingw of a CI build; to update it, change `LLVM_MINGW_VERSION` and
  `LLVM_MINGW_SHA256` in `redist.env`.
- [`fetch-sparkle.sh`](../launcher/scripts/fetch-sparkle.sh) downloads the
  Sparkle of Mach9.app and refuses a tarball whose checksum does not match;
  to update it, change `SPARKLE_VERSION` and `SPARKLE_SHA256` in
  `redist.env`.
- The script that builds a source pinned by tag and commit fails when the
  tag is not at the commit.

## Releases

A `vX.Y.Z` tag makes [`release.yml`](../.github/workflows/release.yml)
build the engine bundle and Mach9.app from one commit and attach both to a
draft release of the tag; `vX.Y.Z-rc.N` does the same for a release
candidate. Its steps call the build and bundle scripts, the scripts in
[`bin/ci/`](../bin/ci) and those in
[`launcher/scripts/`](../launcher/scripts), each of which says in its
header what it does. The `arm64-*` releases are earlier ones of the engine
alone, and the `cx-*` releases the frozen x86_64 Wine this repository
built before, which runs under Rosetta.

| Trigger | What the run does |
|---------|-------------------|
| `v[0-9]*` tag | Builds the engine and the app, notarizes the app, builds its appcast and attaches everything to a draft release; never cancelled |
| Push to `main` that changes a file the build or the app reads, the schedule, by hand | Builds and bundles, assembles and signs the app and its DMG, runs `package.sh --notes-only`, saves its caches when on `main`, publishes nothing |
| Pull request to `main` (no `paths` filter, since its check is required) | As on `main`, in the environment `pull-request` after the owner approves it ([signing identity](#signing-identity)); saves no cache |

- Every run is on a hosted runner of the floor's major macOS version,
  from the branch [`wine-src.ref`](../pins/wine-src.ref) names, with
  `--strict-floor` and `--symbols`. The header and step comments of
  `release.yml` have the details.
- A pull request from a fork gets no secrets and fails at its first step;
  push the branch to this repository instead.
- A release's notes are its `release-notes/X.Y.Z.md`, then one bullet per
  source it was built from (`package.sh`). Add what else the release page
  should say by hand, as short bullets, before publishing the draft.
- [`pages.yml`](../.github/workflows/pages.yml) deploys the GitHub Pages
  site at `mach9.app` ([updates in the field](#updates-in-the-field)),
  which also serves the [spec](https://mach9.app/compatdb_core/) and the
  landing page from [`site/`](../site). Its header says what the site
  holds and when it runs.
- [`compatdb.yml`](../.github/workflows/compatdb.yml) checks the
  formatting of the compatdb crates and tests, lints and builds the
  rustdoc of `compatdb-core` on Linux in minutes; its header says when it
  runs.
- [`launcher.yml`](../.github/workflows/launcher.yml) checks the
  formatting and lints and tests the launcher crates on macOS in minutes;
  its header says when it runs. A change to the launcher on `main` also
  runs `release.yml`, which assembles the app.
- [`conformance.yml`](../.github/workflows/conformance.yml) checks the
  formatting and lints, tests and builds the rustdoc of the `conformance`
  crate on macOS; its header says when it runs. The graphics tests
  themselves run locally only
  ([conformance runner](internals.md#conformance-runner)).

### Cutting a release

1. Commit the version bump to `main`: the workspace `version` in
   [`Cargo.toml`](../Cargo.toml), `Cargo.lock` (any cargo command updates
   it), and the notes `release-notes/X.Y.Z.md`
   ([README](../release-notes/README.md)).
2. Optionally tag a [release candidate](#release-candidates) first.
3. Push the tag `vX.Y.Z`.
4. Check the draft, add to its notes, and publish it with "Set as the
   latest release" checked. Publishing runs `pages.yml`, and only then
   do the copies in the field and the download link see it
   ([updates in the field](#updates-in-the-field)).

Only a `v*` release that carries `appcast.xml` and `Mach9.dmg` may be
marked Latest. With any other one, the download link gets a 404,
`pages.yml` fails and leaves the previous site deployed, and the next tag
run's preflight fails.

A tag run fails before the expensive build when:

- the tag is neither `vX.Y.Z` nor `vX.Y.Z-rc.N`;
- another `v*` draft exists;
- `appcast.sh --preflight` refuses: the tag is not the workspace version,
  the notes are missing, the version is not above the latest appcast's, or
  `SPARKLE_ED_KEY` is not the key the app ships with.

After the build, a tag run frees disk space, then signs, notarizes and
staples the app, packs its DMG, and builds the appcast with the deltas
into this release ([`make-release.sh`](../launcher/scripts/make-release.sh),
[`appcast.sh`](../launcher/scripts/appcast.sh)). The draft holds:

- the engine's tarballs, named after the tag;
- `Mach9.dmg`, the name the download link asks
  `releases/latest/download` for;
- the deltas from earlier releases into this one;
- `appcast.xml`, which lists this release and the earlier ones, each
  pointing at `Mach9.dmg` in its own release.

The appcast is built on the one of the latest published release, so
drafts have to be published in the order they were made, and a draft
blocks the next tag run until it is published or deleted. Tag runs wait
for each other, so a second tag pushed during a run meets its draft. To
re-run a failed tag run that already created its draft, delete that draft
first.

- `appcast.sh` downloads the DMG of every release a delta is made from,
  and `generate_appcast` unpacks each of them and diffs it against the new
  app. Disk use and time grow with the number of deltas; the log shows
  `df -h` before and after.
- A release that rebuilds Wine changes most files of the bundle, so its
  deltas are much larger than those of a release that changes only the
  launcher. `generate_appcast` leaves out a delta it cannot make and one
  larger than 7/8 of the new DMG, and a copy on that release downloads the
  whole DMG.
- A release that raises `LSMinimumSystemVersion` makes `generate_appcast`
  also build deltas into the newest release of the old floor, which belong
  to that earlier release. `appcast.sh` stops on them, since it attaches
  files to the new release only.

### Release candidates

A tag `vX.Y.Z-rc.N` runs the same steps on the commit with the version
bump to X.Y.Z and its notes. Its draft is a prerelease, which is never
Latest, so `pages.yml` never deploys its appcast and copies in the field
never see it. Its `CFBundleVersion` sorts above every earlier release and
below X.Y.Z (`bundle_version` in [`lib.sh`](../launcher/scripts/lib.sh)),
so a copy updated to it is offered X.Y.Z later.

To check the update path before users see it, update a copy of the latest
release to the candidate:

1. Install the latest release's DMG into a scratch folder. It has to be
   0.2.0 or later: 0.1.0 cannot show the update window
   ([updates in the field](#updates-in-the-field)). Until 0.2.0 is out,
   install 0.2.0-rc.2 or a later candidate instead.
2. Point every copy of the app on this Mac at the candidate's appcast:

   ```bash
   defaults write app.mach9 SUFeedURL https://github.com/athei/mach9/releases/download/vX.Y.Z-rc.N/appcast.xml
   ```

   Sparkle takes the feed URL from the app's user defaults before its
   Info.plist, and logs a warning that it does.
3. Choose Check for Updates in the Dock menu of the scratch copy: the update
   is offered, the delta applies, the app relaunches as the candidate and
   starts a game.
4. `defaults delete app.mach9 SUFeedURL`. The key is shared with the
   installed app.

### Updates in the field

Every copy polls the `SUFeedURL` of its
[`Info.plist`](../launcher/app/Info.plist), for good. Every release after
0.2.0-rc.2 polls the appcast on `mach9.app`, this repository's GitHub
Pages site, which [`pages.yml`](../.github/workflows/pages.yml) deploys;
its header says what the site holds and when it runs.

- The site needs the custom domain `mach9.app` and enforced HTTPS in the
  repository's Pages settings, the domain verified on the GitHub account,
  and the apex records of
  [GitHub's apex domain setup](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site#configuring-an-apex-domain)
  in the DNS of `mach9.app` at Gandi LiveDNS. No host of `mach9.app` is a
  web forward.
- Check what the copies see: `curl -sI https://mach9.app/appcast.xml`
  for the status, `curl -s https://mach9.app/appcast.xml | head` for the
  content. Pages' CDN caches the file for a few minutes (its
  `cache-control` header), so a new deployment takes that long to show.
- Marking another release Latest by hand starts no deployment. Redeploy
  then, or whenever the site is wrong:
  `gh workflow run pages.yml --repo athei/mach9`.
- `sparkle.mach9.app`, `updates.mach9.app` and `docs.mach9.app` no longer
  exist. 0.1.0 and 0.2.0-rc.1 poll `updates.mach9.app` and 0.2.0-rc.2
  polls `sparkle.mach9.app`, so Check for Updates shows them an error and
  their background checks fail silently. Their users update by
  downloading the DMG ([app docs](app.md#updates)).
- Never serve an appcast at `updates.mach9.app` again: 0.1.0 and
  0.2.0-rc.1 hang as soon as they show the update window, also on the
  background check before each game launch.
- The config files an earlier Mach9 wrote link to the `compatdb-core`
  docs on `docs.mach9.app`, which are dead; the same pages are on
  `mach9.app`.
- The download link points at GitHub directly,
  `https://github.com/athei/mach9/releases/latest/download/Mach9.dmg`.
  In the appcasts `appcast.sh` builds, from 0.2.0 on, every DMG and delta
  URL points at a release download on GitHub. From the release after
  0.2.0 on, every DMG URL points at the `Mach9.dmg` of its release.
  0.1.0 and 0.2.0 also carry the same DMG as `Mach9-X.Y.Z.dmg`, which the
  `v0.2.0` appcast points at. The `v0.1.0` appcast's DMG URL is on
  `updates.mach9.app`; no copy downloads it, since every copy that polls
  `mach9.app` is newer than 0.1.0.
- A copy installs an update only when it is signed with the EdDSA key
  whose public half is `SUPublicEDKey`, by the Developer ID team the copy
  was signed by, for the bundle id `app.mach9`, with a higher
  `CFBundleVersion`. `appcast.sh` checks the key and the version;
  `codesign-bundle.sh` signs with the identity of the
  [signing identity](#signing-identity) secrets.
- The EdDSA key pair was generated once. Without its private half no copy
  ever accepts another update, so keep a backup of it off the Mac it lives
  on. It lives in that Mac's login keychain, and Sparkle's tools come from
  [`fetch-sparkle.sh`](../launcher/scripts/fetch-sparkle.sh). Export it to
  a file outside the checkout, copy that to the backup, and delete it:

  ```bash
  target/launcher/sparkle/bin/generate_keys -x ~/sparkle-key.txt   # export
  target/launcher/sparkle/bin/generate_keys -f ~/sparkle-key.txt   # import
  rm ~/sparkle-key.txt
  ```

- An update window without notes means the release's appcast item has no
  `<description>`; `appcast.sh` carries each one over from the previous
  appcast.
- How the launcher runs the check is in the header of
  [`update.rs`](../launcher/app/src/update.rs); each skipped check logs
  `update check skipped: <reason>` in `gamelauncher.log`.

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
  cache current from `main`, so a release build usually downloads little.

### Other caches

| Cache | Key |
|-------|-----|
| Each staged directory | Runner image and the hash of its `KEY` ([commands](internals.md#commands)), saved under the `KEY` the build wrote |
| ccache directory | Runner image, run ID and attempt; every run restores the newest by prefix |
| DXMT's LLVM | The LLVM pin, the floor, the Xcode version and the runner image |
| Pinned downloads | The `redist.env` lines that `pins-key.sh` hashes for it in `release.yml` |
| cargo's registry | `Cargo.lock` |
| Sparkle | The `redist.env` lines that `pins-key.sh` hashes for it, and `fetch-sparkle.sh` |
| Xcode choice | Runner image, its version, and the hashes of `host.sh` and `profile.sh`; `brew-cache.yml` shares it |

Only runs on `main` save caches, the Homebrew kegs included. Every run can
read what `main` saved, so a tag build builds only what changed since the
last run on `main`. A cache saved on a tag or another branch would serve
only later runs on that ref, so runs on tags, pull requests and other
branches read `main`'s caches and write none.

- The LLVM key does not change with how `build-dxmt.sh` builds LLVM. To
  rebuild it after such a change, delete the `dxmt-llvm15-*` caches
  (`gh cache list --key dxmt-llvm15-`, then `gh cache delete <id>`) or
  change the pin.
- A new download pinned in `redist.env` that `bundle-wine.sh` or
  `bin/ci/llvm-mingw.sh` keeps in `CACHE_DIR` needs its prefix added to the
  `pins-key.sh` call for the downloads cache in `release.yml`. Sparkle is
  not in that cache; it has its own key.
- A new commit on a branch a pin names, such as the Wine branch, starts no
  run. The next scheduled run picks it up; or start a run on `main` by
  hand before tagging.
- GitHub evicts caches no run used for a week, which the scheduled runs
  prevent, and disables the scheduled workflows of a public repository
  after 60 days without activity. Re-enable `release.yml` and
  `brew-cache.yml` in the Actions tab then.

After a run on `main` saves the ccache directory, the Homebrew kegs, the
pinned downloads or the cargo registry, the `prune` job of the workflow
deletes the caches of `main` the new one replaces with
[`prune-caches.sh`](../bin/ci/prune-caches.sh): the other caches of the
same family of keys created before the run started. The families are the
prefixes the build jobs pass to it as outputs.

- Each pruned family keeps one cache, and also one a run at the same time
  saved after this run started, such as `brew-cache.yml` next to
  `release.yml`.
- A cache whose save failed prunes nothing, and a failed prune only warns.
- The `prune` job alone may write actions. The build jobs run third-party
  code (Wine, Mesa, FEX, bottles, crates) and cannot delete caches.
- The other caches are small, and pruning them would make runs on another
  runner image, unrebased pull requests and tags on older commits build
  again; GitHub's eviction removes them.

### Signing identity

`release.yml` signs the loader and the app with the identity of the
[provisioning profile](internals.md#loader-app)'s team, from the secrets of
the GitHub environment `release`, or of `pull-request` on a pull request.
`bin/ci/keychain.sh` imports it into a keychain of its own for the bundle
step and the app's signature, and deletes that keychain at the end of the
job. It has to be the
Developer ID Application certificate of the team that signed the releases
in the field ([updates in the field](#updates-in-the-field)).

- Create the environment `release` and limit its deployments to the tag
  pattern `v*` and the branch `main`. A pull request that edits the
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

### Notarization and update keys

A tag run also needs these secrets in the environment `release`, and no
other run does:

| Secret | What it holds |
|--------|---------------|
| `NOTARY_API_KEY` | An App Store Connect API key's `.p8` file, base64 |
| `NOTARY_KEY_ID` | That key's ID |
| `NOTARY_ISSUER` | The issuer ID of the account's API keys |
| `SPARKLE_ED_KEY` | Sparkle's private EdDSA key, as `generate_keys -x` exports it |

- Create the API key in App Store Connect, Users and Access,
  Integrations, App Store Connect API, Team Keys, with the Developer role.
  The page shows the issuer ID; the `.p8` file downloads once.
  `codesign-bundle.sh notarize` passes it to `notarytool`.
- Store them, with the exported key outside the checkout, and delete both
  files once the secrets are set:

  ```bash
  base64 -i ~/Downloads/AuthKey_<key id>.p8 | gh secret set NOTARY_API_KEY --env release
  gh secret set NOTARY_KEY_ID --env release
  gh secret set NOTARY_ISSUER --env release
  target/launcher/sparkle/bin/generate_keys -x ~/sparkle-key.txt
  gh secret set SPARKLE_ED_KEY --env release < ~/sparkle-key.txt
  rm ~/Downloads/AuthKey_<key id>.p8 ~/sparkle-key.txt
  ```

- `SPARKLE_ED_KEY` is the key pair the releases in the field were signed
  with, never a new one ([updates in the field](#updates-in-the-field)).
  The tag run refuses a key whose public half is not `SUPublicEDKey`.

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
- `wine.inf` aliases Segoe MDL2 Assets and Segoe Fluent Icons to the
  [icon font](internals.md#icon-font); the bundle step checks both.
- The WMA decoder DMO's stream info and input status calls return `S_OK`;
  `smoke/wmadec.c` checks them.
- The wineserver runs CrossOver's msync and prints the lines the smoke
  test looks for ([runtime](internals.md#runtime)).
- The loader checks for its entitlement and re-execs itself with 4K pages
  ([loader app](internals.md#loader-app)).
- `include/wine/test.h` has `winetest_selected()`, which reads `WT_LIST`,
  `WT_ONLY` and `WT_SKIP` and prints the `wt-list` and `wt-run` lines the
  [conformance runner](internals.md#conformance-runner) reads. The
  graphics tests' sources call it from the blocks
  `conformance wine-macros` generates; `build-wine.sh` checks them
  ([Wine](#wine)).

[gcmouse]: https://mach9.app/compatdb_core/struct.Rule.html#method.gcmouse
[force_nx]: https://mach9.app/compatdb_core/struct.Rule.html#method.force_nx
[opengl]: https://mach9.app/compatdb_core/struct.Rule.html#method.opengl

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

### Wine

Our series on
[`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) moves
from one upstream `wine-X.Y` tag to the next. The two test selection
commits stay at the top of the series. On a rebase:

1. Rebase onto the new tag without "tests: Make the test functions of the
   graphics tests selectable.": drop it, so no generated file conflicts.
   "include: Add winetest_selected() to run single test functions." is
   rebased like the other commits.
2. In this repository, run
   `cargo run -p conformance -- wine-macros --write <wine-src>`. It
   writes nothing, and the `--check` of `build-wine.sh` stops, while:
   - a `START_TEST` body has a call it cannot place;
   - a `START_TEST` of a `SOURCES` file has no unit in the config (a new
     upstream unit);
   - the config names a unit, setup call or helper the sources no longer
     have.
3. Add, fix or remove that unit's entry in
   [`wine-tests.toml`](../conformance/wine-tests.toml) (its header says
   how) and run `--write` again.
4. Commit the result in Wine with the dropped commit's message, and the
   config change in this repository.
5. Run `build-wine.sh` and a full bundle step: the runner needs the full
   bundle's `lib/wine/tests`.
6. Run all suites with the
   [conformance runner](internals.md#conformance-runner), explain every
   regression, and record all suites.
7. Push Wine, then this repository's config change and baselines:
   `release.yml` checks the tip of the Wine branch with this repository's
   crate, so a run between the two pushes fails.

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
- After the bundle step, run the suites that reach Mesa with the
  [conformance runner](internals.md#conformance-runner): those whose
  `expect` in [`wine-tests.toml`](../conformance/wine-tests.toml) sets
  `opengl` or `wined3d`, `vulkan-1`, `d3d12`, and `dxgi`, whose `d3d12:`
  keys run vkd3d-proton on KosmicKrisp. Explain every regression before
  the bundle takes the new driver, and record the suites again once the
  differences are understood.
- Run the [deqp-vk suite](internals.md#deqp-vk-suite) on the new driver,
  `cargo run -p conformance -- deqp-vk run`, with `--vulkan` naming its
  `MESA_DIST` before the bundle step or the bundle after it. A change to a
  KosmicKrisp path a group covers needs that group before and after; an
  issue that needs other cases adds a group to
  [`deqp-vk.toml`](../conformance/deqp-vk.toml). Explain every regression,
  and record the groups again (`deqp-vk record`) once the differences are
  understood.
- Push the branch to athei/mesa before building without `MESA_REPO`
  pointing at the local clone; the default builds what the remote has.
- The Vulkan loader and headers are pinned together, from one Vulkan SDK
  release, in [`redist.env`](../pins/redist.env). Bump both tags and
  commits together.
- A Wine build tree configured without Vulkan has no `SONAME_LIBVULKAN`,
  and one configured before EGL was built has no `SONAME_LIBEGL`;
  `build-wine.sh` stops and asks for `--clean`.

### VK-GL-CTS

VK-GL-CTS, which builds the `deqp-vk` of the
[deqp-vk suite](internals.md#deqp-vk-suite), is pinned by tag and commit
in [`redist.env`](../pins/redist.env).

- Follow the Vulkan CTS release Mesa's CI builds `deqp-vk` from
  (`DEQP_VK_VERSION` in Mesa's `.gitlab-ci/container/build-deqp.sh`), and
  bump tag and commit together.
- Rebuild with `build-vk-gl-cts.sh`, and record every group again
  (`deqp-vk record`) on a driver that did not change since its last
  record, so the differences are the CTS's alone.
- Check that each pattern and `exclude` of
  [`deqp-vk.toml`](../conformance/deqp-vk.toml) still selects what its
  comment says, against the new `mustpass.txt`.

### FEX

FEX is our fork at the ref in [`fex-src.ref`](../pins/fex-src.ref)
([FEX](internals.md#fex)). It moves from one upstream release tag
(`FEX-YYMM`) to the next, as our Wine series on
[`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) moves
from one upstream `wine-X.Y` tag to the next. To move FEX to a newer tag:

- Rebase our commits onto the new tag with `git rerere` enabled, as for
  Mesa, and drop the ones upstream has merged.
- Only our "DiskCache: Bump version" commit raises `FormatVersion` in
  `FEXCore/Source/Interface/Core/DiskCache.h` and `BinaryCacheVersion` in
  the InstructionCountCI JSON files; all other commits leave them at
  upstream's value. Keep that commit last in the series and set the
  version above both the new upstream value and our previous one, as its
  message describes. New commits that change code generation need no
  bump of their own.
- FEX depends on these parts of Wine's layout and interfaces. Recheck
  them when either side changes:
  - the loader's low reserve and Wine's top-down area;
  - the address requirements of `NtAllocateVirtualMemoryEx` and
    `NtMapViewOfSectionEx`;
  - ntdll's `__wine_syscall_dispatcher` and
    `__wine_unix_call_dispatcher_arm64ec`;
  - the `BTCpuProcessTerm` calls of wow64 and the `ProcessTerm` calls of
    ARM64EC ntdll.
- Push the branch to athei/FEX before building without `FEX_REPO`
  pointing at the local clone; the default builds what the remote has.
- After the bundle step, run all suites with the
  [conformance runner](internals.md#conformance-runner), since every test
  is x86 code FEX runs. Explain every regression before the bundle takes
  the new build, and record the suites again once the differences are
  understood.

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
- Run a full bundle step: its smoke test runs the programs in `smoke/`
  that use DXMT ([what the bundle step
  checks](internals.md#what-the-bundle-step-checks)).
- Then run the suites whose `expect` in
  [`wine-tests.toml`](../conformance/wine-tests.toml) names the `dxmt`
  tree with the [conformance runner](internals.md#conformance-runner).
  Explain every regression before the bundle takes the new build, and
  record the suites again once the differences are understood.
