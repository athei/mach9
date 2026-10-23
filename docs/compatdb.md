# compatdb.so

- Built from the crates in [`compatdb/`](../compatdb):
  `compatdb-table` ([`compatdb/table`](../compatdb/table)) holds the pure
  rule logic, `compatdb` the cdylib around it.
- Installed as `lib/wine/aarch64-unix/compatdb.so`, next to ntdll's unix
  half. Wine's ntdll dlopens `<ntdll_dir>/compatdb.so` in every process
  before any PE code runs, the slot CrossOver's own compat database uses.
- At load it reads the process's image name and version resource, resolves
  the matching rules (the built-in ones plus
  [`WINE_COMPATDB`](#wine_compatdb)) and applies them.

## Settings

A setting a rule leaves unset is no opinion: a rule folded earlier decides
([merging](#merging)), and with none, Wine does.

| Key | Values | Effect | Applied |
|-----|--------|--------|---------|
| `dxgi` | `dxmt`, `wined3d` | The D3D10 to D3D12 tree to prepend. A built-in rule sets `dxmt` for every process; unset, `wined3d`, as in upstream Wine. Only `dxmt` has D3D12 (vkd3d-proton): under `wined3d`, `d3d12.dll` does not load, so a D3D12 game does not start ([Direct3D backends](../README.md#direct3d-backends), [native ARM64 programs](#native-arm64-programs)) | At load |
| `d3d9` | `mtld3d`, `wined3d` | The D3D9 tree to prepend. A built-in rule sets `mtld3d` for every process; unset, `wined3d`, as in upstream Wine ([Direct3D backends](../README.md#direct3d-backends), [native ARM64 programs](#native-arm64-programs)) | At load |
| `wined3d` | `gl`, `vulkan` | The renderer of wined3d, for the whole process: every wined3d user in it, the `wined3d` trees of both families and D3D8 and DirectDraw, shares one wined3d. Set, it names the renderer in `WINE_D3D_CONFIG` through the `env` path; unset, it sets nothing, so a `WINE_D3D_CONFIG` from elsewhere, or wined3d's default, OpenGL, applies ([wined3d on Vulkan](#wined3d-on-vulkan)) | At load, as `env` |
| `opengl` | `egl`, `cgl` | Which OpenGL the Mac driver gives the process. `egl`: Mesa's EGL with Zink on KosmicKrisp, and no OpenGL where EGL does not start, as on a Mac without a Metal 4 GPU. `cgl`: Apple's OpenGL. Unset: Apple's OpenGL, as in upstream Wine. The value is the process's own; the programs it starts get their own rules' value ([OpenGL](#opengl)) | At load, read by winemac.drv |
| `dll_overrides` | `WINEDLLOVERRIDES`-style `names=order`, repeatable; the order `default` removes | Added through ntdll's load-order hook. An entry `<names>=default` adds nothing: it takes its modules out of the entries of the rules folded before it, so the registry's `DllOverrides` and `WINEDLLOVERRIDES` decide for them again ([merging](#merging)) | At load |
| `dpi_aware` | `true`, `false` | `true`: DPI-aware (system-aware). `false`: unaware. win32u applies it before the `AppCompatFlags\Layers` registry value and the manifest, and the first setting wins, so the rule beats both. `true` also caps a program that asks for per-monitor awareness at system-aware | At load, read by win32u |
| `large_address_aware` | `true`, `false` | `true`: a 32-bit process gets 4 GB even without the large-address-aware flag. `false` is the same as unset: the executable's large-address-aware flag decides, as in upstream Wine. No built-in `*` rule sets it. No effect on 64-bit | At load; ntdll applies it after the library returns |
| `gcmouse` | `true`, `false` | `true`: the Mac driver takes the mouse movement of raw input (`WM_INPUT`) and DirectInput from GameController's `GCMouse`, in the device's own counts, so mouselook is linear. `false`: that movement is the cursor's, which macOS accelerates and scales for retina. Unset: the same as `false`, as in upstream Wine ([GameController mice](#gamecontroller-mice)) | At load, read by winemac.drv |
| `force_nx` | `true`, `false` | `true`: no-execute stays permanently on in a 32-bit process, even when its executable or a DLL lacks the `NX_COMPAT` flag, for which Wine turns it off, and the process cannot turn it off. `false` is the same as unset: Wine's behaviour. No effect on 64-bit, which always has no-execute on. The value is the process's own; the programs it starts get their own rules' value ([no-execute](#no-execute)) | At load, read by ntdll |
| `arguments` | text, repeatable | Appended to the command line unless already present (case-sensitive), which leaves a Chromium child that inherited its parent's switches alone | At load |
| `env` | `NAME=value`, repeatable; an empty value removes the variable | Written into the environment block and the unix environment, before any PE code runs. Processes the program starts inherit it. An entry of a built-in `*` rule is a default and leaves a variable the process already has alone; every other entry replaces it ([environment defaults](#environment-defaults)) | At load |

The boolean keys take exactly `true` or `false`, case-sensitive. `dxgi`,
`d3d9`, `wined3d` and `opengl` values are case-insensitive.

Each key above that holds one value, `dxgi`, `d3d9`, `wined3d`, `opengl`,
`dpi_aware`, `large_address_aware`, `gcmouse` and `force_nx`, also takes
`default`, in lower case: the setting is unset again from that rule on, so
upstream Wine decides, unless a rule folded later sets it. `env` and
`arguments` take no `default`: an empty `env` value removes the variable.

## Matching

- A rule has no name: it is its matchers and its settings.
- `exe` matches the image basename, case-insensitively, or `*` for every
  process. Every rule needs one.
- `company`, `product` and `original_filename` are optional
  case-insensitive substrings of the version resource's `CompanyName`,
  `ProductName` and `OriginalFilename`. A field the resource lacks reads as
  empty, so a rule that pins it cannot match.

## Merging

- Every matching rule applies; no rule removes or replaces another rule.
- The built-in rules fold first and the `WINE_COMPATDB` rules after them.
  Within each of the two layers the rules fold least specific first: `*`,
  then `*` with a fingerprint, then a basename, then a basename with a
  fingerprint. Within a tier they fold in table order, which for
  `WINE_COMPATDB` is the order of its lines.
- A scalar ends up with the last folded value. So any value a
  `WINE_COMPATDB` rule sets beats every built-in value, and within a layer
  the more specific rule wins. Lists (`dll_overrides`, `arguments`, `env`)
  accumulate in fold order.
- So a built-in value is changed by a `WINE_COMPATDB` rule that sets the
  same key: `exe=*` for every process, or a rule for one executable. That
  includes the value of a built-in rule for one program, which pins a
  fingerprint: a rule on the basename alone changes it. The log names the
  matchers of the built-in rule behind each value
  ([logging](#logging-and-verification)).
- Within `WINE_COMPATDB`, a rule for one executable beats a `*` rule
  whatever their line order, and several `*` rules are not a conflict.
- `default` resets a scalar: from that rule on the setting is unset, as if
  no rule had set it, and a rule folded later can set it again. For
  `dxgi`, `d3d9`, `opengl`, `gcmouse`, `force_nx` and `large_address_aware`
  a value does the same as unset ([settings](#settings)), but `dpi_aware`
  has none: `false` makes a program unaware, while `default` leaves it to
  the registry and the manifest.
- A `dll_overrides` entry with the order `default`, such as
  `mscoree=default` or `mscoree,mshtml=default`, takes its modules out of
  the entries the rules folded before it gave them, one entry naming
  several modules included, and adds no entry of its own. The registry's
  `DllOverrides` and `WINEDLLOVERRIDES` then decide for those modules,
  which they cannot while compatdb has an entry for them. A module name
  matches case-insensitively, with or without `.dll`. This `default` is
  case-insensitive too, unlike a scalar's: an order such as `Default` would
  otherwise reach ntdll, which reads an order without `n` or `b` as
  disabled.

## Built-in rules

- The built-in rules are compiled in from
  [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs), each with a
  comment giving the reason it exists.
- The rules that match every process (`exe` is `*`) set the defaults listed
  in [the README](../README.md#defaults-that-differ-from-wine).
- A behaviour of this Wine that differs from upstream Wine and that a rule
  can switch is off in the code by default, and a built-in rule turns it
  on, for every process or for the programs that need it. So the compatdb
  log of a process shows its complete resolved configuration, with the
  source of every value, and a rule that sets the key changes it
  ([merging](#merging)).
- Every other rule names an executable and pins its version resource, so it
  cannot catch an unrelated program of the same name; a test enforces this.
- The CEF helper rules add `--in-process-gpu` because a CEF GPU process
  cannot present into another process's window
  ([upgrade checklist](maintenance.md#upgrade-checklist)).

### Built-in environment rules

- A built-in rule sets an `env` entry for every process that has FEX
  emulate x87 code with reduced precision, which is much faster and
  slightly less accurate. FEX reads it from the process's environment block
  when it starts, which compatdb.so has already rewritten. A FEX config
  file does not override it, since FEX's environment layer outranks its
  config files, but the variable exported where the process starts does
  ([environment defaults](#environment-defaults)).
- The entry and the reasoning for this rule are in
  [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs). Its
  [recipes](#recipes) turn it off or change it.
- KosmicKrisp's features need no rule: our Mesa fork advertises the ones
  Direct3D 12 and OpenGL through Zink need without `MESA_KK_EXPERIMENTAL`
  ([arm64.md](arm64.md#vulkan)).
- D3D12's frame latency handles need no rule either: our vkd3d-proton
  signals them once the GPU has finished a frame
  ([arm64.md](arm64.md#vkd3d-proton)).

### Environment defaults

- An `env` entry of a built-in `*` rule is a default: it sets its variable
  only where the process has none, in its environment block or its unix
  environment, so a value whoever starts the process exported wins. That
  keeps the variables these rules set usable the way their own
  documentation describes, for example to try a value for one launch.
- A variable exported with an empty value counts as set, so such a default
  leaves it alone too.
- Every other entry replaces the variable: one of a built-in rule for one
  program, and one of any rule from `WINE_COMPATDB`, a `*` rule included.
  Whoever wrote such a rule chose the value for that process.
- A program inherits the environment of the process that started it, the
  values compatdb.so set there included, so a default left alone in a child
  usually holds the value the parent's rules gave it. A value a rule for one
  program sets for a variable that has a `*` default therefore carries into
  the programs it starts: their default finds the variable set and leaves
  it alone ([recipes](#recipes)).
- The log names each entry with its source, and an entry left alone with
  the value the process kept ([logging](#logging-and-verification)).

## WINE_COMPATDB

Whatever starts the process tree can add rules through this environment
variable.

```
exe=Game.exe;company=Some Vendor;d3d9=wined3d;dpi_aware=false;env=MTL_HUD_ENABLED=1
exe=OldGame.exe;large_address_aware=true
exe=*;gcmouse=false
```

### Format

- One rule per line, as `key=value` fields joined by `;`. Blank lines are
  ignored. `Table::to_env_value` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs) writes the
  value this way, for a launcher that uses the crate.
- The log names a rule by its line, counted from 1, blank lines included.
- There is no version header. A first line of `v=` and digits, the header
  launchers wrote for an older format, is ignored, so their tables still
  apply; it still counts as a line.
- Keys: the matching keys (`exe`, required, and `company`, `product`,
  `original_filename`) and the [settings](#settings). The parser,
  `record_to_rule` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs), is the
  authoritative list.
- Inside a value, `%`, `;`, CR, LF and other control characters are
  percent-encoded (`%3B` for `;`, `%25` for `%`). Everything else, including
  `=` and `,`, passes through.

### Errors

Errors never abort a process.

- A field with an unknown key, an unknown `dxgi`, `d3d9`, `wined3d` or
  `opengl` value, a boolean other than `true` or `false` (so `TRUE` or `1`
  too), or no `=` is ignored with a warning naming the line and the field.
  The rest of its line still applies.
- `name` and `enabled`, which launchers wrote for an older format where a
  rule named a built-in rule to change or drop it, are unknown keys like
  any other. A line with them and an `exe` is a rule of its own;
  `enabled=false` no longer drops anything.
- A `compatdb.so` from before rules lost their names drops every line
  without `name`, so a table in this format does nothing on an older
  bundle: a launcher on a crate with this format needs a bundle with this
  library. From this library on, a key that a newer launcher sends and the
  library does not know costs only that field.
- The matcher keys (`exe`, `company`, `product`, `original_filename`) are a
  closed set; a key added later is only ever a setting. A misspelt
  matcher is ignored with a warning like any unknown key, and the rule then
  matches more processes than it names.
- A line without `exe` is ignored with a warning, since it matches no
  process. That includes a line that changed a built-in rule by name in the
  older format. The other lines still apply.
- The warnings appear in the process's log block
  ([logging](#logging-and-verification)).

## Recipes

Each line is one line of `WINE_COMPATDB`. `Game.exe` stands for the game's
executable.

| Goal | Rule |
|------|------|
| Re-enable Wine's Mono for every process | `exe=*;dll_overrides=mscoree=b` |
| Prefer a native .NET | `exe=*;dll_overrides=mscoree=n,b` |
| Hand Mono and Gecko back to the registry's `DllOverrides` and `WINEDLLOVERRIDES` | `exe=*;dll_overrides=mscoree,mshtml=default` |
| Make one game DPI-unaware | `exe=Game.exe;dpi_aware=false` |
| Make every process DPI-unaware | `exe=*;dpi_aware=false` |
| Let the registry and the manifest decide DPI awareness | `exe=*;dpi_aware=default` |
| Give one 32-bit game 4 GB without the large-address-aware flag | `exe=Game.exe;large_address_aware=true` |
| Give every 32-bit process 4 GB | `exe=*;large_address_aware=true` |
| Leave one game to its executable's flag under such a `*` rule, or under a built-in rule for it | `exe=Game.exe;large_address_aware=false` |
| Give one game the cursor's mouse movement instead of GameController's | `exe=Game.exe;gcmouse=false` |
| Give every process the cursor's mouse movement | `exe=*;gcmouse=false` |
| Give every process wined3d for D3D10 and D3D11, and no D3D12 | `exe=*;dxgi=wined3d` |
| Give every process wined3d for D3D9 | `exe=*;d3d9=wined3d` |
| Force Direct3D trees for one game | `exe=Game.exe;dxgi=wined3d;d3d9=wined3d` (not for a D3D12 game: the `wined3d` tree has no D3D12) |
| Hide D3D12 from one game, keeping DXMT for D3D10 and D3D11 | `exe=Game.exe;dll_overrides=d3d12,d3d12core=` |
| Change a vkd3d-proton setting for one game | `exe=Game.exe;env=VKD3D_CONFIG=<options>` |
| Give one game Wine's no-execute behaviour | `exe=Game.exe;force_nx=false` |
| Give every process Wine's no-execute behaviour | `exe=*;force_nx=false` |
| Give one game FEX's full x87 precision | `exe=Game.exe;env=FEX_X87REDUCEDPRECISION=` |
| Give every process FEX's full x87 precision | `exe=*;env=FEX_X87REDUCEDPRECISION=` |
| Give one game Apple's OpenGL | `exe=Game.exe;opengl=cgl` |
| Give every process Apple's OpenGL | `exe=*;opengl=cgl` |
| Put wined3d on Vulkan for one game | `exe=Game.exe;wined3d=vulkan` |
| Leave one game to what upstream Wine does for a setting | `exe=Game.exe;<key>=default` |
| Keep a program that a built-in rule pins on its tree under a `*` rule | `exe=<its exe>;dxgi=<its tree>` |

- A `*` rule from `WINE_COMPATDB` folds after every built-in rule, so it
  also changes the programs that the built-in rules for one program set up
  ([`builtin.rs`](../compatdb/src/builtin.rs)), in both directions:
  `exe=*;dxgi=dxmt` moves the Rockstar Games Launcher onto DXMT, which
  lacks the D3D10.1 device it needs, and `exe=*;dxgi=wined3d` moves the
  Steam and Social Club web helpers (CEF) off DXMT. A rule for such a
  program that sets its tree again keeps it as the built-in rule had it;
  the log shows each value with its source.
- A game that can use D3D12 and D3D11 usually takes D3D12 when
  `d3d12.dll` loads, which in the `dxmt` tree means vkd3d-proton. The rule
  that hides D3D12 disables both of vkd3d-proton's modules through
  compatdb's load-order overrides, so `d3d12.dll` fails to load (error 126)
  and such a game falls back to D3D11 on DXMT; the rest of the `dxmt` tree
  stays. The smoke test checks that this rule disables `d3d12.dll` and
  leaves `dxgi.dll` loading. A game that only has D3D12 does not start
  with it, nor with `dxgi=wined3d`.
- `WINEDLLOVERRIDES=mscoree=b` alone does not work while compatdb has an
  entry for the module: ntdll parses that variable first, and an override
  compatdb adds replaces the entry for the same module. ntdll also consults
  compatdb's overrides before the registry `DllOverrides` keys. With
  `mscoree=default` compatdb has no entry for it, and both apply again.
- A rule works because it folds after the built-in rules and ntdll keeps
  the last entry for a module.
- The processes a game starts inherit the variables of its `env` rule. For
  a variable a built-in `*` rule sets by default, such as
  `FEX_X87REDUCEDPRECISION`, a value a rule for one game sets carries into
  the programs that game starts, since their default leaves a variable
  they inherit alone. An empty value in a rule removes the variable
  instead of setting it empty, and a value removed for one game does not
  carry over: the variable is unset in its children, and the default sets
  it again ([environment defaults](#environment-defaults)).
- A `*` rule that removes such a variable removes it in every process,
  also where whoever started the process exported it, since only the
  built-in rules' entries are defaults.
- The settings ntdll stores for the process, `dpi_aware`,
  `large_address_aware`, `gcmouse`, `force_nx` and `opengl`, are its own:
  the programs a game starts get the value of their own rules.

## wined3d on Vulkan

- `wined3d` chooses the renderer of wined3d, Wine's own Direct3D, for the
  whole process. A process has one wined3d, which reads its settings once,
  and it serves every wined3d user there: the `wined3d` trees that `dxgi`
  and `d3d9` choose, and D3D8 and DirectDraw, which always use wined3d.
  `dxgi` and `d3d9` still decide whether a family uses wined3d at all.
- Vulkan here is KosmicKrisp ([arm64.md](arm64.md#vulkan)).
- No built-in rule sets it, so wined3d stays on its default OpenGL unless a
  rule asks for Vulkan. On KosmicKrisp it still crashes in many of Wine's
  conformance tests, and wined3d does not fall back to OpenGL when Vulkan
  fails.
- Set to either value, it names the renderer in `WINE_D3D_CONFIG`, the list
  of wined3d's `Direct3D` registry settings as comma-separated `name=value`
  options, through the same path as an `env` entry, after the rules' own
  entries. When a rule's `env` also sets `WINE_D3D_CONFIG`, its options are
  kept and its `renderer`, if any, is replaced: `wined3d` decides the
  renderer, `env` the other options. A `WINE_D3D_CONFIG` the process was
  started with is replaced, as by any `env` entry.
- wined3d reads the variable from the environment block, which compatdb.so
  has already rewritten, and the variable takes precedence over wined3d's
  registry settings.
- Unset, it sets nothing, so a `WINE_D3D_CONFIG` from a rule's `env` or
  from the process's environment applies as it is. That includes the one a
  process inherits from the process that started it: a program started by a
  process with `wined3d=vulkan` runs wined3d on Vulkan too, unless a rule
  for it sets `wined3d` itself. The log line names the renderer such an
  inherited value asks for.

## GameController mice

- A built-in rule has the Mac driver read mouse movement for raw input
  (`WM_INPUT`) and DirectInput from GameController's `GCMouse`, which
  reports the device's own counts. Without it that movement is the
  cursor's, which macOS has already accelerated and, on a retina display,
  scaled, so mouselook in such a game turns further the faster the hand
  moves.
- A rule for one game with `gcmouse=false` gives it the cursor's movement
  again ([recipes](#recipes)): for a game whose sensitivity was tuned to
  that movement, a mouse that GameController reports twice, or any
  regression.
- compatdb.so hands the value to ntdll through `set_compat_gcmouse`, and
  winemac.drv reads it when it reads its settings
  ([Wine-side contract](maintenance.md#wine-side-contract)). Without a
  value from compatdb the Mac driver keeps upstream Wine's behaviour, the
  cursor's movement, which `gcmouse=false` gives too, so a `*` rule with it
  turns GameController mice off for every process.

## OpenGL

- A built-in rule sets `opengl=egl` for every process, so the Mac driver
  gives OpenGL through Mesa's EGL with Zink instead of Apple's OpenGL,
  which upstream Wine uses ([OpenGL](arm64.md#opengl)). Without a GPU of
  the Metal 4 family the process then gets no OpenGL, and a rule for it
  with `opengl=cgl` gives it Apple's ([recipes](#recipes)).
- compatdb.so hands the value to ntdll through `set_compat_opengl`, and
  winemac.drv reads it when the process first uses OpenGL
  ([Wine-side contract](maintenance.md#wine-side-contract)). Without a
  value from compatdb the Mac driver gives Apple's OpenGL, as with
  `opengl=cgl`, so a `*` rule with it gives every process Apple's OpenGL.
- The value is the process's own, not an environment variable: a program
  that a game with `opengl=cgl` starts gets the value of its own rules,
  the built-in default unless a rule for it says otherwise.

## No-execute

- A built-in rule sets `force_nx` for every process, so no-execute stays
  permanently on in a 32-bit process even when its executable or a DLL
  lacks the `NX_COMPAT` flag, for which Wine would turn it off. With
  no-execute off, FEX has to treat all writable memory as possible code
  and translates code again whenever that memory changes
  ([arm64.md](arm64.md)).
- compatdb.so hands the value to ntdll through `set_compat_force_nx`, and
  ntdll reads it the first time the process asks for or changes its
  execute flags ([Wine-side contract](maintenance.md#wine-side-contract)).
  Without a value from compatdb ntdll keeps Wine's behaviour, as with
  `force_nx=false`, so a `*` rule with it gives every process Wine's
  behaviour.
- A program that runs code from memory it did not make executable crashes
  with it, and a rule for that program with `force_nx=false` gives it
  Wine's behaviour again ([recipes](#recipes)). Programs built with the ATL
  of Visual Studio 2003, such as some game configurators and launchers,
  are a known case: their window thunks live on the heap, which Windows
  emulates and neither Wine nor FEX does in a 32-bit process here, so they
  fault on their first window message.
- The value is the process's own, not an environment variable: a program
  that a game with `force_nx=false` starts gets the value of its own
  rules, the built-in default unless a rule for it says otherwise.

## Native ARM64 programs

- A native ARM64 program gets the same trees as every program: the built-in
  rules give it `dxmt` for `dxgi` and `mtld3d` for `d3d9`, and without
  them it gets `wined3d` for both. The 64-bit modules
  of every tree of both families are ARM64X, and the program loads their
  ARM64 half. A rule may give it either tree of each family.
- Whether the program can load a Direct3D name at all depends on its
  marker, which is hybrid while every tree of its family holds an ARM64X
  build ([arm64.md](arm64.md#limitations)).
- `default_for` and `effective` of `Dxgi` and `D3d9` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs) decide the
  tree when no rule names one.
- A 64-bit process loads the Direct3D trees from `aarch64-windows`, as Wine
  on an arm64 host loads its builtins from there. An i386 process uses
  `i386-windows`.
- The log labels an x86_64 program, which runs in an ARM64EC process, as
  `x86_64 (arm64ec)` and a native ARM64 program as `aarch64`. The main
  image's machine type tells them apart; a wow64 process is i386.

## Logging and verification

The library writes to Wine's stderr; every line starts with `compatdb:`.

- Once in every process at load it writes a block starting
  `compatdb: <exe> [<arch>]`, with the image name, arch and version
  fingerprint. The block is the whole configuration of the process: every
  setting gets a line with its value and its source, or, where no rule set
  it, `unset (upstream: ...)` with what upstream Wine does instead, followed
  by `reset by <source>` when the last rule for it set it to `default`.
  Those descriptions are `UPSTREAM` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs).
- A source is `built-in` for a built-in rule that matches every process,
  `built-in for <exe> (<fingerprint values>)` for a built-in rule for one
  program, with the values it pins, and `WINE_COMPATDB line <n>` for a
  rule from that variable ([format](#format)).
- After the first line come:
  - a `warning: WINE_COMPATDB line <n>: ...` line for each field or line
    of `WINE_COMPATDB` it ignored ([errors](#errors));
  - the trees it prepended (`dxgi = <impl> (from <path>), <source>`,
    `d3d9 = ...`); a family no rule chose gets `wined3d`;
  - the wined3d renderer: `wined3d = <renderer>, <source>` when a rule set
    it, whose `WINE_D3D_CONFIG` the `env` lines show; otherwise the
    renderer a `WINE_D3D_CONFIG` the process has asks for, from a rule's
    `env` or inherited, as the line says, or unset, where wined3d's
    registry settings or its default decide;
  - `dll_overrides` and `arguments`, each entry with its source, and for an
    argument whether it was appended or already present;
  - one `dll_overrides <module>=default, <source>: ...` line for each
    module an entry with the order `default` names, with the sources of
    the overrides it removed, or saying there was none to remove;
  - `dpi_aware`, `large_address_aware`, `gcmouse`, `force_nx` and
    `opengl` (`large_address_aware` and `force_nx` noting that they have no
    effect in a 64-bit process);
  - one `env NAME=value, <source>: ...` line per entry, saying whether it
    was set, removed (naming the source of the earlier entry whose value it
    undoes, or saying the variable was not set, so there was nothing to
    remove), left alone because the process had the variable, with the
    value it kept ([environment defaults](#environment-defaults)), or
    replaced by a later entry for the same variable, whose source it names,
    so the last entry that applies is the value the process ends up with.
- A line also says when ntdll lacks the hook a setting needs, and the
  setting is ignored.
- To confirm a tree took effect, look at the running process's mapped files,
  for example `lsof -p <pid> | grep -i dxgi.dll`. A path under
  `lib/wine/dxgi/<impl>/` or `lib/wine/d3d9/<impl>/` proves it.
