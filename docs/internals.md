# How the build works

`build-wine.sh` and `bundle-wine.sh` build and bundle a native arm64 Wine
that runs x86 programs through [FEX](https://github.com/FEX-Emu/FEX). The
settings they share are in [`profile.sh`](../bin/lib/profile.sh), and each
script's header says what it builds and stages.

- Wine is upstream Wine with our series on top, the branch
  [`arm64-patched`](https://github.com/athei/wine/tree/arm64-patched) of
  athei/wine, some of it from Hadron. It is configured against Homebrew
  with the allowlist in [`build-wine.sh`](../bin/build-wine.sh), plus the
  floor overrides ([macOS floors](maintenance.md#macos-floors)).
- `WINE_ARCHS` pairs aarch64 with arm64ec, so x64 code runs as ARM64EC,
  and adds i386 for WoW64.
- FEX is our fork ([FEX](#fex)). Built-in compatdb rules keep no-execute
  on in 32-bit processes
  ([`builtin::force_nx`](https://mach9.app/compatdb_core/builtin/fn.force_nx.html))
  and turn on FEX's reduced x87 precision
  ([`builtin::fex_reduced_x87_precision`](https://mach9.app/compatdb_core/builtin/fn.fex_reduced_x87_precision.html)).
- Direct3D comes from [DXMT](#dxmt), [vkd3d-proton](#vkd3d-proton) and
  [mtld3d](#mtld3d), in the trees `DXGI_IMPLS` and `D3D9_IMPLS` name.
  Their 64-bit DLLs are ARM64X: an x64 game loads the ARM64EC half, so
  they run natively and only the game's own code goes through FEX, and a
  native ARM64 program loads the ARM64 half.
- Vulkan runs on KosmicKrisp ([Vulkan](#vulkan)), OpenGL on Zink
  ([OpenGL](#opengl)).
- Media decoding runs on a GStreamer built here ([GStreamer](#gstreamer)).
- A [Wine SDK](#wine-sdk) is staged for the PE code built outside Wine.
- The loader runs from a signed app bundle with 4K pages
  ([Loader app](#loader-app)).

## Requirements

- An Apple silicon Mac and a macOS SDK at least as new as `MACOS_FLOOR`.
- Homebrew at `DEPS_PREFIX` with the formulae in
  [`brew-formulae`](../pins/brew-formulae), which a CI build installs.
  Only a host on the floor's major macOS version gets bottles that meet
  the floor
  ([Homebrew libraries and the floor](#homebrew-libraries-and-the-floor)).
- Xcode's Metal toolchain (`xcrun metal`), for DXMT's shaders.
- [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) at `MINGW_DIR`,
  with its arm64ec toolchain. The scripts append its `bin/` to `PATH`;
  never prepend it, since its unprefixed `clang` breaks configure's host
  compiler detection.
- Rust with the `RUST_TARGET` target, for `compatdb.so` and for the check
  of the Wine source's test selection that `build-wine.sh` runs before it
  builds.
- A code signing identity in the keychain whose certificate
  [`signing/loader.provisionprofile`](../signing/loader.provisionprofile)
  lists.
- Network access:
  - The bundle step resolves each fork's ref (`*_REF` or its `*-src.ref`,
    [paths](#paths)) and gets the loader's secure timestamp.
  - `build-fex.sh`, `build-mesa.sh`, `build-dxmt.sh` and
    `build-vkd3d-proton.sh` resolve their ref against `*_REPO` on every
    run, which needs the network unless `*_REPO` is a local clone.
  - When they build: `build-vkd3d-proton.sh` for the submodules,
    `build-mesa.sh` for a new Vulkan loader or Python environment, and
    `build-gstreamer.sh` unless its clone and wraps are already there.
- For the [deqp-vk suite](#deqp-vk-suite) only, which no CI build runs:
  `build-vk-gl-cts.sh` needs git, CMake, Ninja and Python 3, all of them
  build tools of `brew-formulae` or macOS, and the network for its clone
  and the sources VK-GL-CTS's `external/fetch_sources.py` fetches.
- For the bundle step on a Mac with a Metal 4 GPU: Khronos' validation
  layers, which `build-vulkan-validation-layers.sh` builds with the same
  tools, and the network for its clone and the sources the layers'
  `scripts/update_deps.py` fetches
  ([what the bundle step checks](#what-the-bundle-step-checks)).

## Commands

| Script | What it does |
|--------|--------------|
| [`build-wine.sh`](../bin/build-wine.sh) | Runs `build-mesa.sh` and `build-gstreamer.sh`, builds Wine and the [Wine SDK](#wine-sdk), then runs `build-fex.sh`, `build-dxmt.sh` and `build-vkd3d-proton.sh`; passes `--clean` on to all five |
| [`build-mesa.sh`](../bin/build-mesa.sh) | The Vulkan loader, KosmicKrisp, and EGL with Zink ([Vulkan](#vulkan), [OpenGL](#opengl)) |
| [`build-gstreamer.sh`](../bin/build-gstreamer.sh) | GStreamer with glib and FFmpeg ([GStreamer](#gstreamer)) |
| [`build-fex.sh`](../bin/build-fex.sh) | FEX's emulator DLLs and their unix halves ([FEX](#fex)) |
| [`build-dxmt.sh`](../bin/build-dxmt.sh) | DXMT, after a Wine build ([DXMT](#dxmt)) |
| [`build-vkd3d-proton.sh`](../bin/build-vkd3d-proton.sh) | vkd3d-proton's D3D12 DLLs, after a Wine build ([vkd3d-proton](#vkd3d-proton)) |
| [`bundle-wine.sh`](../bin/bundle-wine.sh) | Assembles the bundle, signs the loader and checks the result ([what the bundle step checks](#what-the-bundle-step-checks)) |
| [`build-vk-gl-cts.sh`](../bin/build-vk-gl-cts.sh) | `deqp-vk` for the [deqp-vk suite](#deqp-vk-suite); not run by `build-wine.sh` |
| [`build-vulkan-validation-layers.sh`](../bin/build-vulkan-validation-layers.sh) | Khronos' Vulkan validation layers, for the bundle step's checks that run under them ([what the bundle step checks](#what-the-bundle-step-checks)); not run by `build-wine.sh`, and not in the bundle |

The flags are in [building.md](building.md#commands). The five component
scripts also run alone.

- FEX, Mesa, DXMT and vkd3d-proton are fetched from their `*_REPO` at the
  ref their `*-src.ref` names, or their `*_REF` when it is set
  ([paths](#paths)). A commit that is only in a local clone is built only
  with `*_REPO` set to that clone.
- Each staged directory (`*_DIST`) holds a `KEY` line naming what it was
  built from: the source commit, the hashes of the scripts, the tool
  versions (`dist_key` in `profile.sh`, and each script's key function).
- A script builds nothing when its directory is complete and has the
  `KEY` it would write now (`skip_current_dist`); `--clean` deletes the
  directory. DXMT also builds whenever the Wine source has uncommitted
  changes.
- `--print-key` prints that `KEY` and exits; the release workflow keys
  its caches on it ([other caches](maintenance.md#other-caches)).
- `WINE_BUILD_CCACHE`, the absolute path of a ccache, runs the compilers
  of Wine, Mesa and FEX through it (`profile.sh`). It changes neither
  configure results nor what is built, and needs no `--clean`. DXMT and
  vkd3d-proton do not use it.

## Paths

Every tree lives under `build/` and the bundle in `dist/wine`, in the
[layout](building.md#layout). Each variable below overrides the default of
the same name, set in [`profile.sh`](../bin/lib/profile.sh) or, for
`MINGW_DIR`, `CACHE_DIR` and the `VK_GL_CTS_*` and
`VULKAN_VALIDATION_LAYERS_*` ones, at the top of the scripts that use
them. A `MACOSX_DEPLOYMENT_TARGET` may raise the floor but not lower it.

| Variable | What it is |
|----------|------------|
| `WINE_SRC` | The Wine source, a checkout of athei/wine |
| `BUILD_DIR` | The Wine build tree; also holds cargo's target directories for `compatdb.so` and for the test selection check, one per checkout of this repository (`COMPATDB_TARGET` in [`bundle-wine.sh`](../bin/bundle-wine.sh), `CONFORMANCE_TARGET` in [`build-wine.sh`](../bin/build-wine.sh)) |
| `X86LIBS_BUILD_DIR` | The x86 tree for the SDK's x86_64 archives and [graphics tests](#graphics-tests) |
| `WINE_SDK_DIR` | The [Wine SDK](#wine-sdk) |
| `FEX_REPO`, `MESA_REPO`, `DXMT_REPO`, `VKD3D_PROTON_REPO` | The repositories the forks are fetched from |
| `FEX_REF`, `MESA_REF`, `DXMT_REF`, `VKD3D_PROTON_REF` | The branch, tag or commit of that repository to build instead of the one its `*-src.ref` names, such as a candidate branch in a worktree of the fork's checkout (point `FEX_SRC` and `MESA_SRC` at other clones then: by default they are those checkouts, which the build would move to the candidate); the bundle step checks the staged commit against the same ref, so it needs the same variables |
| `FEX_SRC`, `MESA_SRC` | The local clones `build-fex.sh` and `build-mesa.sh` check out |
| `FEX_BUILD_DIR`, `MESA_BUILD_DIR`, `DXMT_BUILD_DIR`, `VKD3D_PROTON_BUILD_DIR`, `GSTREAMER_BUILD_DIR` | Each component's trees, and the clones its script keeps there |
| `DXMT_LLVM` | The LLVM static libraries `winemetal.so` links |
| `FEX_DIST`, `MESA_DIST`, `DXMT_DIST`, `VKD3D_PROTON_DIST`, `GSTREAMER_DIST` | What the bundle installs from each component, with the commits it was built from; by default inside the component's build directory |
| `LOADER_PROFILE` | The provisioning profile the loader is signed for |
| `VK_GL_CTS_BUILD_DIR`, `VK_GL_CTS_DIST` | The tree of [`build-vk-gl-cts.sh`](../bin/build-vk-gl-cts.sh), with its VK-GL-CTS clone, and the `deqp-vk` it stages |
| `VULKAN_VALIDATION_LAYERS_BUILD_DIR`, `VULKAN_VALIDATION_LAYERS_DIST` | The tree of [`build-vulkan-validation-layers.sh`](../bin/build-vulkan-validation-layers.sh), with its clone and the sources it builds on, and the layers it stages, which the bundle step reads |
| `MINGW_DIR` | llvm-mingw |
| `CACHE_DIR` | Downloaded redistributables |

## Runtime

The variables are listed in the
[README](../README.md#environment-variables).

msync is CrossOver's implementation of Windows synchronization objects on
Mach semaphores and shared memory, which saves wineserver round trips.

- Only the wineserver reads `WINEMSYNC`. Unset or a non-zero number, msync
  is on. Any other value (`0`, empty, not a number) turns it off, and Wine
  synchronizes through wineserver requests as upstream Wine does.
- Set `WINEMSYNC=0` where the wineserver session starts, normally the
  launcher's first Wine process. It takes effect only for a new session,
  and every process of the session follows the wineserver.
- A per-game compatdb `env` rule cannot turn msync off reliably: the game
  only starts the wineserver when no session is running yet.
- With msync on, the wineserver prints `msync: up and running.` on its
  standard error and registers a launchd service named
  `wine-<dev>-<inode>-msync` (`launchctl print gui/<uid>` lists it for a
  wineserver of the GUI login session).
- When it cannot register that service, it prints an `msync: error:` line
  and runs the session without msync. The messages are in
  [`server/msync.c`](https://github.com/athei/wine/blob/arm64-patched/server/msync.c).

`WINENCPU` limits the number of processors Wine reports, for programs that
misbehave with many.

- Set it where the session starts, like `WINEMSYNC`. Each process limits
  the count it reports itself, but the `CentralProcessor` registry keys
  and `NUMBER_OF_PROCESSORS` are written once per session, and FEX's
  emulated CPUID reads those keys, so a value set later for one game
  leaves them at the session's count.
- A value of 0, one that is not a number, or one above the real count is
  ignored.

## FEX

FEX runs the x86 code of a process; Wine loads it as the emulator DLLs of
`aarch64-windows` and `i386-windows`. FEX is
[athei/FEX](https://github.com/athei/FEX) at the ref in
[`fex-src.ref`](../pins/fex-src.ref): upstream FEX plus our commits. Two
of its modes differ from upstream. They are emulator settings, changed
with FEX's own variables, not compatdb settings.

- Memory ordering. With `FEX_HARDWARE_TSO` unset or `1`, FEX runs x86 code
  with the hardware x86 memory ordering (TSO) instead of emulating it with
  ordered loads and stores. It switches a thread to TSO when the thread
  enters x86 code and back at its syscalls and unix calls, since native
  code runs slower in it. `0` makes FEX emulate the ordering; any other
  value logs an error and does the same. The [loader app](#loader-app)'s
  entitlement gates the mode.
- AFP. x86_64 programs, which run as ARM64EC, run without AFP (the Arm
  floating-point behaviour FEX uses to match x86): the floating-point mode
  would have to change at every call between x86 and native code, which
  costs more than AFP saves. `enableafp` in `FEX_HOSTFEATURES` turns it
  back on. i386 programs keep AFP.
- FEX reads its options from the Windows environment block, which
  compatdb.so has already rewritten, so a compatdb `env` entry sets one
  for one program. A FEX config file does not override such an entry.

## Wine SDK

PE code built outside Wine links against Wine's `libwinecrt0.a` and
`libntdll.a` and runs `winebuild` to mark itself a builtin.
`build-wine.sh` stages all of it in one directory, in the layout of an
installed Wine, with a `WINE_COMMIT` file naming the Wine source commit.
What goes in, and why, is the Wine SDK step of
[`build-wine.sh`](../bin/build-wine.sh).

- A full bundle carries the same files, so the bundle is an installed Wine
  with its SDK. [mtld3d](https://github.com/athei/mtld3d) builds against
  an installed Wine; its
  [`docs/BUILDING.md`](https://github.com/athei/mtld3d/blob/main/docs/BUILDING.md)
  lists what that build needs.
- `build-dxmt.sh` and the bundle step refuse an SDK whose `WINE_COMMIT` is
  not the Wine source's current commit.

### Graphics tests

The SDK and a full bundle carry Wine's graphics conformance tests as
`lib/wine/tests/<pe-dir>/<name>_test.exe`, to test the bundle's own stack
(mtld3d, DXMT, wined3d, the Vulkan loader) with nothing but the bundle.
mtld3d's conformance suite runs the `d3d9_test.exe` of both directories.

- The tests are `GRAPHICS_TESTS` in [`profile.sh`](../bin/lib/profile.sh),
  built for `TEST_PE_DIRS`, the x86 architectures games use. An x86_64
  test runs as ARM64EC, as an x64 game does.
- They are plain executables, outside the directories Wine searches for
  modules.
- `lib/wine/tests/VERSION` holds the `wine --version` string of the Wine
  they were built from, so a copy of `lib/wine/tests` taken alone still
  names its build. mtld3d's conformance runner reads its first line.
- A full bundle's `lib/wine/tests/COMPONENTS` has a `<name>=<commit>`
  line for each component besides Wine that these scripts build from
  source and the tests run on (`TEST_COMPONENTS` in
  [`bundle-wine.sh`](../bin/bundle-wine.sh)), so results from two bundles
  show what each ran on. The SDK has no such file.
- `d3d12_test.exe` needs the `dxmt` tree ([vkd3d-proton](#vkd3d-proton)).
  On the `wined3d` tree the loader stops it with `c0000135` (a missing
  DLL) and it exits with 53.

To run one in a prefix of its own, give its path to the bundle's `wine`,
with the names of the parts to run (all without a name; the names are the
`START_TEST` entries in the test's sources):

```bash
WINEPREFIX=/tmp/gfx-prefix dist/wine/bin/wine \
    dist/wine/lib/wine/tests/x86_64-windows/d3d9_test.exe device
```

The test picks its Direct3D tree through compatdb as a game does, so a
`WINE_COMPATDB` rule for `d3d9_test.exe` puts it on another tree
([recipes](https://mach9.app/compatdb_core/#recipes)).
It prints a summary line per part and exits with the number of failed
checks, at most 255.

To run single test functions of a part, set `WT_ONLY` to their keys, a
comma-separated list; `WT_SKIP` runs all but the listed ones, and
`WT_LIST=1` prints a `wt-list:<key>` line per test function and runs none.
The parts `wine-tests.toml` marks `whole`, and every part of
`d3d9_test.exe`, ignore them (`winetest_selected()` in Wine's
`include/wine/test.h`, [conformance runner](#conformance-runner)).

#### Conformance runner

The crate in [`conformance/`](../conformance) runs these tests on a bundle
and compares the results with baselines committed here, so a fork sync or
a component upgrade shows what changed, test by test. Each test function
runs in a process of its own, so a crash or stall loses only that function.

```bash
cargo run -p conformance -- run d3d11 dxgi   # compare with the baselines
cargo run -p conformance -- record d3d11     # write the baselines again
```

- A suite is a test executable of
  [`wine-tests.toml`](../conformance/wine-tests.toml), named without
  `_test.exe`, or `all` for every one. Direct3D 9 is not a suite:
  mtld3d's conformance suite covers it.
- A key is one test function of a unit (a `START_TEST` part), named after
  the function, in one leg (`i386-windows` or `x86_64-windows`). A unit
  that `wine-tests.toml` marks `whole` runs as one process with the key
  `*`.
- `run --help` and `record --help` list the options.
- Each invocation boots a fresh prefix with the launcher's emulated
  display modes and the retina and DPI values its automatic setting gives
  on a Retina display (`SETTINGS` in
  [`session.rs`](../conformance/src/session.rs)). Record baselines on a
  Retina display. The runner refuses to start while a wineserver of the
  same bundle runs.
- Each test process has to become the active application: one that stays
  in the background gets no activation or focus events, which the ddraw
  `test_coop_level_mode_set` tests depend on. macOS does not activate a
  process while another application is active, so `run` and `record` make
  the runner a regular application that takes the activation when it
  starts, hands it to each Wine process of its prefix that asks for it,
  and takes it back from any other application in between
  ([`activation.rs`](../conformance/src/activation.rs)). The line before
  the result counts both. Leave the keyboard and mouse alone until
  the run ends.
- `run` and `record` refuse to start on a locked screen, where no process
  becomes active (`IOConsoleLocked` of `ioreg -n Root -d1`). They check
  the selection, the bundle, the baselines, the triage file, the state
  directory and the running wineservers before they take the activation.
- Every process gets the caller's environment without the variables that
  change Wine, the libraries it loads, its graphics stack or the test
  framework (`SCRUBBED` in
  [`environment.rs`](../conformance/src/environment.rs)).
- Before it trusts a result, the runner checks the test process's
  `compatdb:` lines ([logging](compatdb.md#logging-and-verification))
  against the `expect` of its executable in `wine-tests.toml`, and stops on
  any other implementation.
- `record` writes `conformance/baselines/<suite>.txt`, which nobody edits
  by hand; `run` needs one. Its header has the bundle's `VERSION` and
  `COMPONENTS`, whose differences a run reports without failing. The
  format is in [`baseline.rs`](../conformance/src/baseline.rs).
- People keep [`triage.txt`](../conformance/baselines/triage.txt) in the
  same directory: per test an issue and whether it is flaky.
- The state directory, `build/conformance` in the
  [layout](building.md#layout) by default, serves one invocation at a
  time:
  - `runs/<run id>/` holds the raw logs of each invocation. The runner
    never deletes them; delete old runs by hand.
  - `baseline-raw/<suite>/` holds the logs of the last record of each
    suite, on the machine that recorded it only.
  - `record` writes the baselines and `baseline-raw/` once every suite of
    the invocation is done.
- Interrupts:
  - `SIGINT`, `SIGTERM` or `SIGHUP` ends the prefix's Wine processes and
    exits with 130; a second signal ends the runner at once, and the next
    invocation ends what is left.
  - The terminal the run was started from is not the active application
    during the run, so a Ctrl-C typed there does not arrive. Choose Quit
    in the Dock menu of the runner's icon, or send `kill -INT <pid>` from
    a shell that does not need the keyboard of this Mac, such as an SSH
    session; the first output line names the pid. The runner also takes a
    Ctrl-C or Command-Q key event that reaches it as a stop request, but a
    key press during a run does not reach it reliably.
  - A Quit, Ctrl-C or Command-Q ends the run as the first signal does,
    but leaves the signal handling as it was: another Quit changes
    nothing, and the next signal still counts as the first, so only the
    signal after that ends the runner at once.
  - The same command continues where it stopped, unless the test
    executable, `VERSION` or `COMPONENTS` changed
    ([`resume.rs`](../conformance/src/resume.rs)).
- A process a test moved out of its process group runs on into later keys
  until a stall or a key that was not reached restarts the Wine session
  ([`exec.rs`](../conformance/src/exec.rs)).
- The suites need a Mac with a Metal 4 GPU, open windows and take the
  focus, so they run locally only;
  [`conformance.yml`](../.github/workflows/conformance.yml) runs only the
  crate's own tests.

| Result | Meaning | Exit code |
|--------|---------|-----------|
| Match | `run`: no change fails the run. `record`: a baseline with no unstable key | 0 |
| Regression | `run`: a key got worse, a new key crashes or stalls, or the run no longer lists a key | 1 |
| Stale baseline | `run`: a key got better or is new: record the suite | 1 |
| Unstable | `run`: a changed key gave its baseline's result the second time. `record`: a key gave two results and is written with the higher status and the larger of each count | 1 |
| Error | Bad arguments, no baseline, or a bundle, state directory or implementation the runner cannot use | 2 |
| Interrupted | Stopped by a signal, or for `run` and `record` by a quit request or a stop key | 130 |

- The last line of the output names the result. What counts as worse or
  better, and what is only listed as `flaky` or `info`, is in
  [`diff.rs`](../conformance/src/diff.rs).
- `run` runs again each baselined key whose change fails the run; `record`
  each key whose result differs from the old baseline, new keys included.
  Mark an unstable key `flaky` in the triage file.
- `record` refuses to record one leg or some keys of a suite on another
  build than its baseline's: record the whole suite.

After a regression:

- The second line of the output names the invocation's log directory. A
  test's log in it is `<leg>/<suite>/<unit>/<key>.log` (`whole.log` for a
  whole unit), with every run of the key and how each ended; a stall also
  leaves a sample beside it. A resumed test's log is in the directory of
  the invocation that ran it.
- On the machine that recorded the baseline, `baseline-raw/<suite>/` has
  the same test's log from that record.
- Rerun one key alone, which also rules out a process an earlier key left
  running; `--only` takes a comma-separated list:

  ```bash
  cargo run -p conformance -- run d3d11 --leg x86_64 --only d3d11/test_create_device
  ```

- Trace a key with `--winedebug <channels>`, which `run` takes and
  `record` refuses. Every Wine process of the invocation gets the channels
  as its `WINEDEBUG` instead of `-all`, and the traces go into the raw
  logs: a test's into its log, the session's into `session.log`. A trace
  slows a test and counts as output for the stall limit, so its result is
  that of the traced test, and its state is never continued without the
  option:

  ```bash
  cargo run -p conformance -- run ddraw --leg x86_64 --only ddraw7/test_coop_level_mode_set --winedebug +win,+event
  ```

- Record the suite again only once every regression is fixed or
  explained.

#### deqp-vk suite

The same crate runs cases of Khronos' Vulkan CTS on KosmicKrisp with
`deqp-vk`, natively and without Wine, so a change to our Mesa shows what
it changes in the CTS groups it touches. Which cases run is the groups of
[`deqp-vk.toml`](../conformance/deqp-vk.toml), patterns over the
vk-default mustpass; a group is added when an issue needs it, and the full
mustpass is not a goal.

```bash
bin/build-vk-gl-cts.sh                                     # build deqp-vk once
cargo run -p conformance -- deqp-vk run                    # compare with the expected results
cargo run -p conformance -- deqp-vk run --group draw       # one group
cargo run -p conformance -- deqp-vk record --group draw    # write its expected results again
```

- [`build-vk-gl-cts.sh`](../bin/build-vk-gl-cts.sh) builds `deqp-vk` from
  the VK-GL-CTS release [`redist.env`](../pins/redist.env) pins and stages
  it in `build/vk-gl-cts/dist`; its header says what it stages. It is not
  part of `build-wine.sh`, and the bundle does not carry it.
- `--vulkan` names the Vulkan stack: a full bundle, by default
  `dist/wine`, or a `MESA_DIST` of `build-mesa.sh`, which compares two
  Mesa builds without a bundle step. `deqp-vk` loads that stack's loader
  (`--deqp-vk-library-path`) with `VK_DRIVER_FILES` naming its manifest
  directory and every layer disabled (`deqp_env` in
  [`environment.rs`](../conformance/src/environment.rs)).
- Before any case counts, a probe runs `dEQP-VK.info.device_properties`
  with the loader's driver messages on. It must pass, the loader must have
  found its manifests in that directory alone and set up every device with
  the driver of that stack, and the runner prints the device, the driver
  library and its `driverVersion`
  ([`probe.rs`](../conformance/src/deqp/probe.rs)).
- A group's `kk_debug` sets `MESA_KK_DEBUG` for its `deqp-vk` processes,
  to run its cases on a KosmicKrisp path the cases do not reach otherwise;
  every other variable of Mesa stays removed. The groups of each setting run
  after those without one, in processes of their own, with a probe of their
  own whose line names the setting, and their logs in `kk_debug-<names>/` of
  the run's directory; a case of two groups with different settings runs
  once for each. A KosmicKrisp that does not know a name ignores it.
- The cases run in batches, one `deqp-vk` process each, several at a time
  (`--jobs`, `--batch`). A process that dies or stalls in a case gives
  that case `crash` or `stall` and the batch goes on in a new process.
  `deqp-vk run --help` lists the options.
- A case that loses the device on KosmicKrisp can make the cases other
  processes run at the same time fail or lose their device too. So the
  cases the expected results give `crash` or `stall` run after the
  others, one process at a time, and so does every other case that
  crashed or stalled among the others. When one did, the cases that
  failed among the others run again without it. After each lost device
  in a process of its own, `dEQP-VK.api.smoke.triangle` has to pass
  before the next case runs (`run_cases` and `recover` in
  [`run.rs`](../conformance/src/deqp/run.rs)).
- When a case that crashed or stalled among the others does so alone
  too, it took the others with it, and the results alone and run again
  count. When none does, nothing explains the first crash: each such case,
  and each case whose result changed when it ran again, is unstable.
  `run` then ends with the unstable result, and `record` writes the worse
  of its two results and counts it as unstable.
- `record` writes `conformance/baselines/deqp-vk/<group>.txt`, which
  nobody edits by hand: every case of the group that does not pass, with
  its status (one `<node>.*` line where all cases of the group under a
  node share one), and a header with the commits of Mesa, the loader and
  VK-GL-CTS, which a run reports without failing, and the number of cases,
  which a run of the whole group that selects another number reports as
  stale. The format and what counts as worse or better are in
  [`expected.rs`](../conformance/src/deqp/expected.rs).
- People keep [`triage.txt`](../conformance/baselines/deqp-vk/triage.txt)
  in the same directory: per case pattern an issue and whether it is
  flaky. A flaky case fails a run only on a new crash or stall, and
  `record` runs it three more times alone and writes its worst result,
  so a case that loses the device in some runs is expected to crash.
  Mark a case flaky only with two runs of one build that differ.
- `--cases <pattern>`, which may be given several times, runs and
  compares only the cases of the groups a pattern matches, to rerun a
  regression alone.
- The results and exit codes are those of the table above. `run` runs
  each case whose change fails the run a second time, and `record` each
  case whose result differs from the old file.
- The state directory, `build/deqp-vk` by default, serves one invocation
  at a time. `runs/<run id>/` holds the probe's output and test log and
  each batch's output (`first/batch-<n>.log`, `second/` for the cases run
  again, `first-again/` and `second-again/` for the failed cases run
  again without one that lost the device, and `first-alone/` and
  `second-alone/` for those run alone).
- Interrupts:
  - Ctrl-C, `SIGTERM` or `SIGHUP` ends the `deqp-vk` processes and exits
    with 130; an interrupted invocation keeps no results, and the next
    one starts over.
  - A second signal ends the runner at once and can leave `deqp-vk`
    processes running without a watchdog. The runner refuses to start
    while a process of the dist's `deqp-vk` runs, and names it: it cannot
    tell its own from one started by hand, so it ends none, as the Wine
    suites refuse to start while a wineserver of the bundle runs.
- Like the Wine suites, it needs a Mac with a Metal 4 GPU and runs
  locally only. It opens no windows: the window-system cases the groups
  match end NotSupported.

## DXMT

DXMT is [athei/dxmt](https://github.com/athei/dxmt) at the ref in
[`dxmt-src.ref`](../pins/dxmt-src.ref): upstream DXMT plus our commits,
among them the `dxgi.dll` support for vkd3d-proton's swapchains. How it is
built (ARM64X and i386 DLLs, `winemetal.so` from the same meson tree, the
LLVM its shader converter links) is the header of
[`build-dxmt.sh`](../bin/build-dxmt.sh).

The DLLs and `winemetal.so` share a private call interface, so they come
from one checkout; the bundle step checks the recorded commit, version and
Wine commit (`check_dxmt_build`).

## vkd3d-proton

Direct3D 12 is [athei/vkd3d-proton](https://github.com/athei/vkd3d-proton)
at the ref in [`vkd3d-proton-src.ref`](../pins/vkd3d-proton-src.ref):
upstream vkd3d-proton plus our commits. How it is built is the header of
[`build-vkd3d-proton.sh`](../bin/build-vkd3d-proton.sh).

- It translates D3D12 to Vulkan on KosmicKrisp, whose features it needs
  are always on in our Mesa ([Vulkan](#vulkan)).
- Our commits release a swapchain's frame latency handle once the GPU has
  finished a present, not once the present is on screen, which on
  KosmicKrisp is one or more refresh cycles later. Otherwise a game that
  waits on the handle at the default latency of 1 starts a frame only every
  few refresh cycles, with the GPU idle and at a low clock in between.
  vkd3d-proton's own frame latency limit still waits for the display.
  `smoke/d3d12.c` runs a waitable swapchain, and `check_d3d12_runs` checks
  that vkd3d-proton reports this for it.
- `d3d12.dll` and `d3d12core.dll` go into the `dxmt` tree. vkd3d-proton
  has no DXGI of its own: DXMT's `dxgi.dll` presents its Vulkan swapchain
  into the window, so the two are always loaded together.
- No other tree has D3D12 (Step 3b of
  [`bundle-wine.sh`](../bin/bundle-wine.sh)): under `dxgi=wined3d`,
  `d3d12.dll` does not load (error 126).

## mtld3d

mtld3d comes from the release tarball pinned in
[`redist.env`](../pins/redist.env), not built here. Step 4 of
[`bundle-wine.sh`](../bin/bundle-wine.sh) says what it takes from the
tarball. It refuses one whose 64-bit DLLs are not ARM64X, since
compatdb.so gives native ARM64 programs the mtld3d tree too
([native ARM64 programs](compatdb.md#native-arm64-programs)).

## Vulkan

Wine's Vulkan (`vulkan-1.dll`, `winevulkan`) runs on KosmicKrisp, Mesa's
Vulkan driver on Metal, through the Khronos Vulkan loader. mtld3d and DXMT
go to Metal directly; D3D12 uses Vulkan through vkd3d-proton, and wined3d
can, per game.

- Mesa is [athei/mesa](https://github.com/athei/mesa) at the ref in
  [`mesa-src.ref`](../pins/mesa-src.ref): upstream Mesa plus our topic
  commits, originally from
  [Hadron's Mesa patches](https://github.com/Broyojo/hadron/tree/main/patches/mesa).
  Commits of upstream merge requests not merged yet are carried under
  their original authors and dropped once upstream has them
  ([rebasing](maintenance.md#mesa-and-the-vulkan-loader)).
- KosmicKrisp is built in two stages so the driver links system libraries
  only, and the loader is built from source for the floor; both are in the
  header of [`build-mesa.sh`](../bin/build-mesa.sh).
- `win32u.so` loads the loader the first time something in the process
  needs Vulkan. That includes the display device refresh `explorer.exe`
  runs when a session starts, which takes the GPU's name, memory size and
  UUID for the registry from KosmicKrisp.
- The loader finds the driver through the manifest in
  `share/vulkan/icd.d/`, whose `library_path` is relative, so the bundle
  stays relocatable. ntdll sets `VK_DRIVER_FILES` to that directory unless
  the caller set `VK_DRIVER_FILES`, `VK_ICD_FILENAMES` or
  `VK_ADD_DRIVER_FILES`. A caller that sets one chooses the drivers
  itself, and has to name the bundled one to use it. A portability driver
  loads through `VK_DRIVER_FILES` with its stock manifest.
- The features our commits add (device-generated commands, transform
  feedback, sparse resources), which D3D12 and Zink need, are always
  advertised. `MESA_KK_EXPERIMENTAL` keeps only Mesa's own experimental
  flags
  ([`kosmickrisp.rst`](https://gitlab.freedesktop.org/mesa/mesa/-/blob/main/docs/drivers/kosmickrisp.rst));
  the bundle sets none.
- wined3d (the `wined3d` trees, D3D8 and DirectDraw) keeps its OpenGL
  renderer, which reaches Direct3D 11 on Zink ([OpenGL](#opengl)) and a
  lower feature level on Apple's OpenGL. Its Vulkan renderer is not the
  default because it still crashes in many of Wine's conformance tests and
  does not fall back to OpenGL. A compatdb rule turns it on for one game
  ([`Rule::wined3d`](https://mach9.app/compatdb_core/struct.Rule.html#method.wined3d)).
- DXVK is not covered. A program that wants Vulkan itself gets it.

[`bench/kk.sh`](../bench/kk.sh) builds a native benchmark of KosmicKrisp's
GPU time and runs it on a full bundle or a `MESA_DIST`, so a Mesa change
gets a number before and after; the bundle step does not run it. The cases
and the output are in the header of [`bench/kk.c`](../bench/kk.c).

```bash
bench/kk.sh                                             # on dist/wine
bench/kk.sh ../build/mesa/dist --case dispatch-loads    # one case, on a Mesa build
```

Each timed run starts with a fixed load in the run's command buffer, before
the first timestamp, so the GPU is in its highest performance state whatever
ran before; without it a light arm slows the arms after it. Compare the
difference between the arms of one invocation, not absolute medians across
invocations: the absolute times of one build still move between invocations
while the differences stay put. For a before and after, alternate
invocations of the two stacks with the same `--case` selection.

## OpenGL

OpenGL runs on Zink on KosmicKrisp through Mesa's EGL wherever
KosmicKrisp has a GPU, instead of on Apple's OpenGL, which stops at 4.1
core and 2.1 compatibility.

- `build-mesa.sh` builds `libEGL.1.dylib` and the gallium library with
  Zink, without GLES, GLX and GBM; the bundle ships both in
  `lib/external/`, where Zink finds the bundled Vulkan loader
  (`check_egl` in [`bundle-wine.sh`](../bin/bundle-wine.sh)).
- `build-wine.sh` passes `--with-opengl` and configure finds this EGL
  through the `egl.pc` in `MESA_DIST`, so a build without it fails.
  `SONAME_LIBEGL` is patched to `lib/external/` like the Vulkan loader.
- Zink reports OpenGL above 2.1 only with transform feedback, which our
  KosmicKrisp always advertises.
- The Mac driver takes the choice from compatdb's `opengl` setting
  ([`Rule::opengl`](https://mach9.app/compatdb_core/struct.Rule.html#method.opengl)).
- With `egl` it does not fall back: where EGL does not start, as on a Mac
  without a GPU of the Metal 4 family, it logs a `winediag` message and
  the process gets no OpenGL.

## GStreamer

`winegstreamer` decodes audio and video for DirectShow, Media Foundation
and the DMOs games use (the WMA decoder of `wmadmod.dll` among them) on a
GStreamer the bundle carries, so no GStreamer needs to be installed.

- [`build-gstreamer.sh`](../bin/build-gstreamer.sh) builds it from source
  for the floor, LGPL only, with just the plugins and FFmpeg decoders
  games need; its header has the lists and the checks it runs on what it
  stages. Homebrew's GStreamer and FFmpeg are GPL and are not used.
- `build-wine.sh` passes `--with-gstreamer` and configure finds it through
  the `.pc` files in `GSTREAMER_DIST`, so a build without it fails.
  `winegstreamer.so` links its libraries directly, by their paths under
  `lib/external/` ([relocation](maintenance.md#relocation)).
- ntdll sets `GST_PLUGIN_SYSTEM_PATH_1_0` to the bundle's
  `lib/external/gstreamer-1.0/` and unsets
  `GST_PLUGIN_PATH` and `GST_PLUGIN_PATH_1_0`, unless the caller set
  `GST_PLUGIN_SYSTEM_PATH` or `GST_PLUGIN_SYSTEM_PATH_1_0`. A plugin of
  another GStreamer would load a second GStreamer into the process.
- In that case it also points `GST_REGISTRY_1_0` at a file of its own
  under `XDG_CACHE_HOME` (or `~/.cache`), unless the caller set
  `GST_REGISTRY` or `GST_REGISTRY_1_0`, so another GStreamer does not
  write the same registry.
- There is no `gst-plugin-scanner`: the first process of a new bundle that
  uses GStreamer reads the plugins itself.

## Icon font

Programs name Segoe MDL2 Assets and Segoe Fluent Icons for window controls
and toolbars. Those fonts are Windows', not Wine's, so without a replacement
the icons draw as boxes.

- The bundle carries Uno Fluent Icons, pinned in
  [`redist.env`](../pins/redist.env), in Wine's data directory. win32u
  loads every font there and lists it in the Fonts key dwrite reads, so
  each prefix has it.
- Its family name is `Symbols`. wine.inf aliases both Windows names to it
  under `HKCU\Software\Wine\Fonts\Replacements`, the key win32u and dwrite
  read; dwrite does not read `FontSubstitutes`.
- The alias reaches an existing prefix when `wine.inf` is reinstalled
  ([upgrade checklist](maintenance.md#upgrade-checklist)).
- An icon the font does not have draws as it does without the font.
- There is no switch: a program that names neither font sees more font
  families.
- Its licences are in [`licenses/`](../licenses), installed into
  `share/licenses/`.

## Loader app

The loader needs `com.apple.developer.cross-architecture-support`, and Wine
does not run without it. macOS honours it only through a provisioning
profile embedded in an app bundle and a signature by a certificate of the
profile's team. The entitlement:

- leaves most of the 4 GB `__PAGEZERO` as a mapping Wine can replace,
  which Wine claims as its low reserved area for i386 code under FEX;
- covers the space between 4 GB and the randomly placed loader the same
  way, which the loader reserves so that an x64 executable built for
  0x100000000 without relocations can load there;
- lets a process run with 4K pages instead of 16K;
- gates the hardware x86 memory ordering of [FEX](#fex).

How the loader uses it:

- It execs itself with 4K pages before Wine sets up its address space
  (`exec_with_4k_pages` in
  [`loader/main.c`](https://github.com/athei/wine/blob/arm64-patched/loader/main.c)),
  and ntdll starts new Wine processes with 4K pages directly
  (`preloader_exec` in
  [`dlls/ntdll/unix/loader.c`](https://github.com/athei/wine/blob/arm64-patched/dlls/ntdll/unix/loader.c)).
- Everything in a Wine process, FEX included, then runs on 4K pages, so
  page protections, guard pages, decommit and FEX's tracking of
  self-modifying code work as on Windows. The wineserver and other plain
  Unix programs keep the default page size.
- When the kernel placed the loader too close to 4 GB for such an
  executable, the loader execs itself again for a new placement, a bounded
  number of times, and warns and keeps its placement if that fails (the
  threshold and limit are in `loader/main.c`).
- A loader without the entitlement would be killed in that exec without a
  message, so it checks first and exits with an error saying it must run
  from the signed `wine.app`. There is no fallback to 16K pages.

The app:

- The bundle step wraps the loader into `lib/wine/aarch64-unix/wine.app`
  and links the entry points to it (`package_loader` in
  [`bundle-wine.sh`](../bin/bundle-wine.sh)).
- It is named Mach9, the launcher this Wine runs under, and carries the
  games category and Game Mode keys, so a game the launcher starts gets
  Game Mode. The bundle identifier stays the one the profile grants.
- The signature claims only what the loader needs; `package_loader` and
  `LOADER_RUNTIME_ENTITLEMENTS` list each entitlement with its reason.
- It is signed once as a whole with the hardened runtime and a secure
  timestamp, as notarization and Mach9's install verification require.
  `LOADER_TIMESTAMP=0` signs without the timestamp for an offline local
  build, which cannot be notarized.
- An app that embeds the bundle leaves `wine.app` as it is: re-signing it
  or editing its `Info.plist` loses the profile or breaks the signature.
  Entitlements take effect only on a main executable, and every Wine
  process runs as this one, so the other Mach-O files of the bundle need
  none; the app signs them for notarization with the hardened runtime.

## What the bundle step checks

The preflight runs before anything is deleted, Step 5 after assembly, and
the smoke test last, in [`bundle-wine.sh`](../bin/bundle-wine.sh), with
`check_pe64_native` in [`profile.sh`](../bin/lib/profile.sh). Each
function's comment says what it checks and why; each program in
[`smoke/`](../smoke) says what it does in its header. `--skip-smoke`
leaves out the smoke test and the preflight checks only it needs: the
validation layers and the llvm-mingw compilers.

| Checks | Where |
|--------|-------|
| macOS SDK, formulae, Wine SDK, each staged component current against its pin or `*_REF` (which needs the network), on a Mac with a Metal 4 GPU the validation layers at their pin, llvm-mingw tools, provisioning profile and identity | Preflight: `check_*_build`, `check_wine_sdk`, `loader_preflight` |
| Inherited `com.apple.quarantine` removed, nothing else | Step 4f |
| No reference to `LEAK_PREFIXES`, no `@rpath` dependency outside the bundle, every dylib loads ([`smoke/dlopen.c`](../smoke/dlopen.c)), GStreamer loads only from the bundle | `check_no_leaks`, `check_rpath_deps`, `check_dylibs_load`, `check_gstreamer` |
| Copied-in components in place, the [icon font](#icon-font)'s aliases in `wine.inf`, mtld3d's DLLs the release's, FEX as `xtajit*.dll`, marker machines, native 64-bit PE ([Direct3D trees](maintenance.md#direct3d-trees)) | Step 5, `check_trees_arm64x`, `check_pe64_native` (`profile.sh`) |
| No PE debug sections or symbol tables (stripped by Step 4e; `--keep-symbols` skips the stripping in Step 4e, and Step 5 reports the count of files with symbols instead of failing), no dangling symlinks | Step 5 |
| Vulkan driver and manifest, EGL with Zink, ntdll's compatdb hooks | `check_vulkan`, `check_egl`, `check_compat_hooks` |
| `wine.app` signature, entitlements and `Info.plist` | `check_loader_app` |
| `minos` of every Mach-O at most the floor, no import of `FLOOR_NEWER_FUNCS` ([macOS floors](maintenance.md#macos-floors)) | `check_minos`, `check_floor_newer_imports` |
| A fresh prefix boots with msync and compatdb, and the programs in `smoke/` run on every arch and tree: Direct3D trees, WGL, Vulkan, D3D12, DXMT's emulated stream output under Metal shader validation, DXMT's bounds on shader bytecode, its context state and deferred context lifetimes and its depth bias on D24 targets, wined3d, WMA decoding, FEX on code that faults or changes, a 64-bit debugger attaching to and detaching from an i386 process, once continuing its break-in and once passing it on, and outside Wine, GStreamer, and EGL with Zink's depth bias on 24-bit depth buffers, its pixel buffer object transfers of float and signed normalized texels and its statistics queries that are active together, the latter also under Khronos' validation layers, which may report no error | `smoke_test` and the `check_*` functions it calls |

On a Mac without a Metal 4 GPU, such as a hosted CI runner, the checks
that need a KosmicKrisp device or Zink change or are left out, and so are
the stream output and `dxmt.exe` runs, which are for DXMT on Apple GPUs;
so are those that need Apple's accelerated OpenGL on a Mac without it
([`smoke/metal4.m`](../smoke/metal4.m),
[`smoke/opengl.m`](../smoke/opengl.m)). The step says which.

## Homebrew libraries and the floor

The libraries in `lib/external/` other than Mesa's and GStreamer's are
Homebrew's: `DEPS_FORMULAE` and their runtime closure. Homebrew builds each
bottle for one macOS major version, with the deployment target at its
`.0` release, and pours the bottle of the host's version, so the host
decides whether those libraries meet the floor.

- On a host of the floor's major version, every bottle meets the floor. A
  CI or release build runs on such a host and passes `--strict-floor`
  ([releases](maintenance.md#releases)).
- On a newer macOS, the bundle step warns about each newer bottle. That is
  expected for a local build, which is not distributed.
- A formula with no bottle for the host is compiled there and carries the
  host's full version as `minos`, which can exceed the floor even on its
  major version. `--strict-floor` stops such a bundle.
- Before moving the build to another macOS version, check that version's
  bottles: `formulae.brew.sh/api/formula/<name>.json` lists every tag's
  bottle (`brew fetch --bottle-tag` may know only the host's). Unpack each
  and read its dylibs' `minos` with `vtool -show-build`.

Static linking:

- Nothing from Homebrew is linked statically into a file the bundle
  ships.
- Wine's unix modules load Homebrew's libraries by soname.
- The libraries that are linked (GStreamer's, DXMT's LLVM, DXMT's and
  Mesa's own archives) are built here for the floor, with Homebrew off
  `PATH` for Mesa and GStreamer.
- `minos` does not show a static object that needs a newer macOS; only
  the linker's warning that an object `was built for newer` macOS does.
  After adding a static dependency, search a clean build's log for it.

## Limitations

- A native ARM64 program cannot load a Direct3D name whose marker in
  `aarch64-windows` is AMD64: the loader fails with error 193
  (`ERROR_BAD_EXE_FORMAT`). The bundle step therefore stamps hybrid
  markers and requires every 64-bit module of every tree to be ARM64X
  ([Direct3D trees](maintenance.md#direct3d-trees)).
- A tree changed after the bundle step, such as by a development install
  of mtld3d, has to keep to that, or a native ARM64 program cannot load
  its module. Which tree such a program gets is compatdb's choice
  ([native ARM64 programs](compatdb.md#native-arm64-programs)).

## Pinned inputs

A `*-src.ref` file names a branch, tag or commit of the component's
`*_REPO`; its `*_REF` overrides it ([paths](#paths)).

| Input | Pinned in |
|-------|-----------|
| FEX | [`fex-src.ref`](../pins/fex-src.ref) |
| Mesa (KosmicKrisp) | [`mesa-src.ref`](../pins/mesa-src.ref) |
| DXMT | [`dxmt-src.ref`](../pins/dxmt-src.ref) |
| vkd3d-proton | [`vkd3d-proton-src.ref`](../pins/vkd3d-proton-src.ref); its submodules at the commits that ref records |
| mtld3d | [`redist.env`](../pins/redist.env): URL and SHA-256 of the release tarball |
| Icon font | [`redist.env`](../pins/redist.env): URL at a commit of its repository and SHA-256 |
| LLVM for `winemetal.so` | [`redist.env`](../pins/redist.env): tag and commit |
| Vulkan loader and headers | [`redist.env`](../pins/redist.env): tag and commit of each, from one SDK release |
| Python modules of Mesa's build scripts | [`redist.env`](../pins/redist.env): pip requirements with exact versions |
| GStreamer, glib and FFmpeg | [`redist.env`](../pins/redist.env): the monorepo's tag and commit, and the commit of the FFmpeg port branch its wrap names; the other wraps pin their tarballs by checksum |
| Wine | Not pinned: whatever `WINE_SRC` has checked out. A CI build checks out the branch [`wine-src.ref`](../pins/wine-src.ref) names. The SDK and DXMT record the commit, and the bundle step refuses either when it is not the current one. All uncommitted states count as one (`-dirty`), so the bundle step cannot tell them apart: rebuild after changing uncommitted changes |
| Homebrew libraries | Not pinned ([Homebrew libraries and the floor](#homebrew-libraries-and-the-floor)); a CI build installs [`brew-formulae`](../pins/brew-formulae) at Homebrew's newest versions ([Homebrew formulae](maintenance.md#homebrew-formulae)) |
| llvm-mingw | A CI build: [`redist.env`](../pins/redist.env), version and SHA-256. A local build: `MINGW_DIR` |
| Provisioning profile | [`signing/loader.provisionprofile`](../signing/loader.provisionprofile); replace it when it expires or the App ID changes |
| Sparkle, Mach9.app's updater | [`redist.env`](../pins/redist.env): version and SHA-256 of the release tarball |
| VK-GL-CTS, for the [deqp-vk suite](#deqp-vk-suite) | [`redist.env`](../pins/redist.env): tag and commit; the sources it builds on at the commits and checksums its `external/fetch_sources.py` pins |
| Vulkan validation layers, for the [bundle step](#what-the-bundle-step-checks) | [`redist.env`](../pins/redist.env): tag and commit; the sources they build on at what the tag's `scripts/known_good.json` names |
