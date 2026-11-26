# compatdb.so

- Built from two crates of the Cargo workspace at the repository root:
  `compatdb-core` ([`compatdb/core`](../compatdb/core)) holds the rules,
  their format, the built-in rules and the API a launcher uses;
  `compatdb` ([`compatdb/src`](../compatdb/src)) is the cdylib around it.
- The [rustdoc of `compatdb-core`][spec] is the spec of the rules and of
  every setting. This page covers what the library does in a process.
- Installed as `lib/wine/aarch64-unix/compatdb.so`, next to ntdll's unix
  half. Wine's ntdll dlopens `<ntdll_dir>/compatdb.so` in every process
  before any PE code runs, the slot CrossOver's own compat database uses.
- At load it reads the process's image name, version resource and Steam
  app id ([matching][matching]), takes the built-in rules
  (`Table::builtin`) and the caller's rules (`Table::parse_lenient`,
  `Table::parse_file_lenient`), resolves those that match and applies them
  through ntdll's hooks
  ([Wine-side contract](maintenance.md#wine-side-contract)).
- The caller's rules are the value of `WINE_COMPATDB`, then the rules of
  the file `WINE_COMPATDB_FILE` names, after the built-in rules
  ([format][format], [folding][folding],
  [launcher responsibilities][launcher],
  [`rules_file.rs`](../compatdb/src/rules_file.rs)). A launcher may send its
  own rules in the variable and point a stable file at the rules that
  change.

## Direct3D backends

| Family | APIs | Rule value | Tree |
|--------|------|------------|------|
| `dxgi` | D3D10, D3D11, D3D12 | `dxmt` ([DXMT](internals.md#dxmt), D3D12 from [vkd3d-proton](internals.md#vkd3d-proton)) | `lib/wine/dxgi/dxmt/` |
| `dxgi` | D3D10, D3D10.1, D3D11 | `wined3d` | `lib/wine/dxgi/wined3d/` |
| `d3d9` | D3D9 | `mtld3d` ([mtld3d](internals.md#mtld3d)) | `lib/wine/d3d9/mtld3d/` |
| `d3d9` | D3D9 | `wined3d` | `lib/wine/d3d9/wined3d/` |

- Which tree a process gets is [`Rule::dxgi`][dxgi] and
  [`Rule::d3d9`][d3d9]. compatdb.so prepends one tree per family to the
  process's builtin search path, so a process sees one coherent set of
  modules.
- The default directories hold only fake-module markers for these DLLs
  ([Direct3D trees](maintenance.md#direct3d-trees)), so a process gets
  Direct3D only through the tree compatdb.so prepends, and a process
  without compatdb.so has none.
- D3D10, D3D11 and D3D12 form one family because they create their device
  or swapchain through one `dxgi.dll` and only work with their own
  implementation's copy.
- A game that can use both D3D12 and D3D11 and prefers D3D12 gets
  vkd3d-proton instead of DXMT's D3D11, unless a [recipe][recipes] hides
  D3D12 from it. Under that rule `d3d12.dll` fails to load with error
  126; the smoke test checks that it does and that `dxgi.dll` still
  loads.
- D3D8 and DirectDraw stay on wined3d.

### Native ARM64 programs

- A native ARM64 program gets the same trees as every program and loads
  the ARM64 half of their ARM64X modules. Whether it can load a Direct3D
  name at all depends on the name's marker
  ([limitations](internals.md#limitations)).
- `default_dxgi`, `default_d3d9`, `effective_dxgi` and `effective_d3d9` in
  the crate's [`runtime`][runtime] module decide the tree when no rule
  names one.
- A 64-bit process loads the Direct3D trees from `aarch64-windows`, as Wine
  on an arm64 host loads its builtins from there. An i386 process uses
  `i386-windows`.

## Profiler check-in

The hardened `wine.app` drops `DYLD_INSERT_LIBRARIES`, so a profiler gets a
process through the library instead. It registers a bootstrap service,
starts Wine with the service's name in `WINE_PROF_SERVICE`, and each
process sends that service one message with its task control port and image
name ([`prof.rs`](../compatdb/src/prof.rs) has the message).

- Without the variable, or with an empty one, the library makes no
  bootstrap call.
- Every process that inherits the variable checks in, Wine's own such as
  `services.exe` too, so a profiler receives until it has the one it wants.
  A message that waits in the queue of the service can outlive its process.
- The send has a timeout. A process that cannot check in logs why and runs
  unprofiled.
- The variable also turns on FEX's extended debug info, an `env` entry with
  the source `WINE_PROF_SERVICE`
  ([`Resolution::fold_profiler_env`][fold-profiler]). It goes into the
  environment block, where FEX reads its options. A value in the block and
  a rule's entry win, and a FEX without the option ignores the variable.
- A child usually inherits both variables with the block. A child started
  with a block of its own still has them in the unix environment, so it
  checks in, and the library puts the unix value of the FEX option into its
  block.
- The control port gives full control of the process, so a launch must not
  take the variable from where the launcher was started. A launcher clears
  it when it inherited it ([launcher responsibilities][launcher]). Setting
  it in Mach9.app's `[env]`, or a launcher's own equivalent, is the
  deliberate route, and it reaches Wine.

## Linger at exit

A launch from Finder or the Dock execs Wine in the process Launch Services
started, so the first Windows program runs in the app's main process. When
that program starts a game and exits, as a game's own launcher does, macOS
ends the app and terminates its other processes, wineserver and the game
included.

- The launcher sets `WINE_COMPATDB_APP_PID` to its own pid for such a launch
  ([`APP_PID_VAR`][app-pid]). The library turns the linger on in the
  process with that pid ([`runtime::linger_at_exit`][linger]) through
  ntdll's `set_compat_linger_at_exit`
  ([Wine-side contract](maintenance.md#wine-side-contract)). Every other
  process inherits the variable and has another pid.
- When that process exits while another user process runs, its Windows
  process ends as usual: its handle is signalled and its exit code set, so
  a program that waits for it goes on. Its unix process stays until no user
  process runs other than Wine's system processes and the desktop's
  `explorer.exe`, then exits with the same status.
- It closes the fds of its sockets and files before it waits, so a
  connection or a listening port of the program ends with the Windows
  process.
- A launch from a terminal sets no variable, and the process exits as in
  upstream Wine.
- Only a process that ends itself lingers. One that another process
  terminates exits at once, as in upstream Wine.
- While it stays, it keeps its memory, its Metal objects and its windows: a
  window the program left open stays on screen, and the app quits only when
  the game exits.
- The other user processes are those of the wineserver session, not of the
  app: a second Mach9 instance started in the same session stays until the
  first instance's processes have exited too.

## Logging and verification

The library writes to Wine's stderr, once in every process at load, a
block that is the whole configuration of the process. Every line starts
with `compatdb:`.

- Each setting gets a line with its value and its source. Where no rule
  set it, the line says `unset (upstream: ...)` with what upstream Wine
  does instead ([`Key::upstream`][upstream]), followed by
  `reset by <source>` when the last rule for it set it to `default`.
- A source is `built-in` for a built-in rule for every process,
  `built-in for <exe> (<fingerprint values>)` for a built-in rule for one
  program, `WINE_COMPATDB line <n>` for a rule from that variable,
  `WINE_COMPATDB_FILE line <n>` for a rule from the file that variable
  names and `WINE_PROF_SERVICE` for what a
  [profiler](#profiler-check-in) turns on. Each counts its own lines
  ([format][format]).

| Line | What it shows |
|------|---------------|
| `<exe> [<arch>]` | The image name, arch and version fingerprint. The arch is `x86_64 (arm64ec)` for an x86_64 program, `aarch64` for a native ARM64 one and `i386` for a wow64 one |
| `SteamAppId = <id>` | The Steam app id rules can match ([matching][matching]); only when the process has one |
| `WINE_COMPATDB_FILE = <path>` | The file that adds rules; only when the variable is set |
| `warning: WINE_COMPATDB_FILE not read: <reason>; ...` | The file cannot be used and adds no rules; `WINE_COMPATDB` and the built-in rules still apply |
| `warning: WINE_COMPATDB line <n>: ...`, `warning: WINE_COMPATDB_FILE line <n>: ...` | A field or line of the rules it ignored, in the variable or the file; a matcher field it cannot read drops the whole line ([parsing][parsing]) |
| `WINE_PROF_SERVICE = <name>: ...` | Whether the process [checked in](#profiler-check-in), or why it did not; only when the variable is set |
| `dxgi = <impl> (from <path>), <source>`, `d3d9 = ...` | The trees it prepended; a family no rule chose gets `wined3d` |
| `wined3d = ...` | The wined3d renderer a rule set, or the one a `WINE_D3D_CONFIG` from a rule's `env` or inherited asks for; unset, wined3d's registry settings or its default decide |
| `metal_hud = <value>, <source>` | The HUD setting; its variables are in the `env` lines |
| `vulkan_capture = true, <source>: Ctrl+Shift+P records into <folder>` | The capture layer is loaded and the chord armed; its variables are in the `env` lines ([Vulkan capture](internals.md#vulkan-capture)) |
| `vulkan_capture = true, <source>: error: <reason>; no capture` | The folder cannot be made or written to, the layer is missing, or ntdll lacks a hook; the process runs without the layer |
| `vulkan_capture = unset ...; GFXRECON_CAPTURE_PROCESS_NAME=<name> inherited: ...` | The process inherited the capture variables from the program that started it: the layer loads and records nothing |
| `vulkan_capture, <source>: ignored, ...` | A rule for `*` or a Steam app id set it, which only a rule that names one program may |
| `metal_capture = ...` | The same lines for the Metal capture ([Metal capture](internals.md#metal-capture)); its inherited note names `MTL_CAPTURE_ENABLED` and `MESA_KK_GPU_CAPTURE` |
| `dll_overrides`, `arguments` | Each entry with its source; for an argument, whether it was appended or already present |
| `debug_channels` | Each entry with its source; only logged. Wine gives an entry of a rule for `game.exe` to a process whose command line starts with a path that ends in the component `game.exe`, in any case, whatever its image name, so a program started as `wine game` misses it ([`Table::winedebug`][winedebug]) |
| `dll_overrides <module>=default, <source>: ...` | The overrides an entry with the order `default` removed, or that there was none |
| `arguments_override, <source>: dropped ...` | The arguments it dropped, each with its source |
| `arguments_override, <source>: ignored, ...` | The rule is for a Steam app id, where the override does not apply ([`Rule::arguments_override`][arguments-override]) |
| `dpi_aware`, `large_address_aware`, `gcmouse`, `force_nx`, `opengl` | The value; `large_address_aware` and `force_nx` note that they do nothing in a 64-bit process |
| `linger_at_exit = on, ...`, `linger_at_exit = off, ...` | Whether the process [lingers at exit](#linger-at-exit): `WINE_COMPATDB_APP_PID` unset, or the pid it names and whether that is this process |
| `env NAME=value, <source>: ...` | Whether the entry was set, removed, left alone because the process had the variable ([`Rule::env`][env]), and then whether the value of its unix environment went into its environment block, or replaced by a later entry, with the sources involved |
| `<setting> = <value>, <source>: ntdll lacks <hook>, ignored` | ntdll lacks the hook; the setting is ignored |

To confirm a tree took effect, look at the running process's mapped files,
for example `lsof -p <pid> | grep -i dxgi.dll`. A path under
`lib/wine/dxgi/<impl>/` or `lib/wine/d3d9/<impl>/` proves it.

[spec]: https://mach9.app/compatdb_core/
[format]: https://mach9.app/compatdb_core/#format
[parsing]: https://mach9.app/compatdb_core/#strict-and-lenient-parsing
[matching]: https://mach9.app/compatdb_core/#matching
[folding]: https://mach9.app/compatdb_core/#folding
[launcher]: https://mach9.app/compatdb_core/#launcher-responsibilities
[recipes]: https://mach9.app/compatdb_core/#recipes
[upstream]: https://mach9.app/compatdb_core/enum.Key.html#method.upstream
[runtime]: https://mach9.app/compatdb_core/runtime/index.html
[dxgi]: https://mach9.app/compatdb_core/struct.Rule.html#method.dxgi
[d3d9]: https://mach9.app/compatdb_core/struct.Rule.html#method.d3d9
[env]: https://mach9.app/compatdb_core/struct.Rule.html#method.env
[arguments-override]: https://mach9.app/compatdb_core/struct.Rule.html#method.arguments_override
[fold-profiler]: https://mach9.app/compatdb_core/struct.Resolution.html#method.fold_profiler_env
[app-pid]: https://mach9.app/compatdb_core/constant.APP_PID_VAR.html
[linger]: https://mach9.app/compatdb_core/runtime/fn.linger_at_exit.html
[winedebug]: https://mach9.app/compatdb_core/struct.Table.html#method.winedebug
