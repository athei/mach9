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

A setting a rule leaves unset is no opinion: a less specific rule decides,
and with none, Wine does.

| Key | Values | Effect | Applied |
|-----|--------|--------|---------|
| `dxgi` | `dxmt`, `wined3d` | The D3D10 to D3D12 tree to prepend. Unset: the arch default ([Direct3D backends](../README.md#direct3d-backends), [native ARM64 programs](#native-arm64-programs)) | At load |
| `d3d9` | `mtld3d`, `wined3d` | The D3D9 tree to prepend. Unset: `mtld3d`, except for a native ARM64 program ([native ARM64 programs](#native-arm64-programs)) | At load |
| `wined3d` | `gl`, `vulkan` | The renderer of wined3d, for the whole process: every wined3d user in it, the `wined3d` trees of both families and D3D8 and DirectDraw, shares one wined3d. Set, it names the renderer in `WINE_D3D_CONFIG` through the `env` path; unset, it sets nothing, so a `WINE_D3D_CONFIG` from elsewhere, or wined3d's default, OpenGL, applies ([wined3d on Vulkan](#wined3d-on-vulkan)) | At load, as `env` |
| `dll_overrides` | `WINEDLLOVERRIDES`-style `names=order`, repeatable | Added through ntdll's load-order hook | At load |
| `dpi_aware` | `true`, `false` | `true`: DPI-aware (system-aware). `false`: unaware. win32u applies it before the `AppCompatFlags\Layers` registry value and the manifest, and the first setting wins, so the rule beats both. `true` also caps a program that asks for per-monitor awareness at system-aware | At load, read by win32u |
| `large_address_aware` | `true`, `false` | `true`: a 32-bit process gets 4 GB even without the large-address-aware flag. `false` only stops forcing it: the executable's flag, `WINE_LARGE_ADDRESS_AWARE` and the `AppDefaults` registry value decide. No effect on 64-bit | At load; ntdll applies it after the library returns |
| `arguments` | text, repeatable | Appended to the command line unless already present (case-sensitive), which leaves a Chromium child that inherited its parent's switches alone | At load |
| `env` | `NAME=value`, repeatable; an empty value removes the variable | Written into the environment block and the unix environment, before any PE code runs. Processes the program starts inherit it. An ntdll that asks also gets the entries before it execs the program, through `compatdb_query_env`; the arm64 Wine's ntdll does not ask | At load |

The boolean keys take exactly `true` or `false`, case-sensitive. `dxgi`,
`d3d9` and `wined3d` values are case-insensitive. Some values from the
retired x86_64 Wine still parse
([values from the x86_64 Wine](#values-from-the-x86_64-wine)).

## Matching

- `exe` matches the image basename, case-insensitively, or `*` for every
  process.
- `company`, `product` and `original_filename` are optional
  case-insensitive substrings of the version resource's `CompanyName`,
  `ProductName` and `OriginalFilename`. A field the resource lacks reads as
  empty, so a rule that pins it cannot match.
- Every matching rule applies. Rules are folded least specific first: `*`,
  then `*` with a fingerprint, then a basename, then a basename with a
  fingerprint. Within a tier they fold in table order: built-in rules first,
  then `WINE_COMPATDB` rules in the order given.
- A scalar ends up with the last folded value, so the most specific rule
  wins. Lists (`dll_overrides`, `arguments`, `env`) accumulate in fold order.
- So a `*` rule from `WINE_COMPATDB` beats a built-in `*` default, and a
  rule for one executable beats both. Several `*` rules are not a conflict.
- `compatdb_query_env` answers in the process that starts the new one and
  matches the executable's file on disk: its basename and the version
  resource read from the file. For a DOS path that ntdll cannot map to a
  unix path, only the basename is known, and only rules without a
  fingerprint can match.

## Built-in rules

- The built-in rules are compiled in from
  [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs), each with a
  comment giving the reason it exists.
- The rules that match every process (`exe` is `*`) set the defaults listed
  in [the README](../README.md#defaults-that-differ-from-wine).
- Every other rule names an executable and pins its version resource, so it
  cannot catch an unrelated program of the same name; a test enforces this.
- The CEF helper rules add `--in-process-gpu` because a CEF GPU process
  cannot present into another process's window
  ([upgrade checklist](maintenance.md#upgrade-checklist)).
- To change or drop a built-in rule, see
  [merging with the built-in rules](#merging-with-the-built-in-rules).

### FEX and KosmicKrisp

- The built-in `fex-x87-reduced-precision` rule sets an `env` entry for
  every process that has FEX emulate x87 code with reduced precision, which
  is much faster and slightly less accurate. FEX reads it from the
  process's environment block when it starts, which compatdb.so has already
  rewritten. Neither a FEX config file nor the variable exported before
  launch overrides it: FEX's environment layer outranks its config files,
  and the rule replaces a value already in the environment.
- The built-in `kosmickrisp-experimental` rule sets `MESA_KK_EXPERIMENTAL`
  for every process, so that KosmicKrisp ([arm64.md](arm64.md#vulkan))
  advertises the experimental features our Mesa topic commits add.
  compatdb.so sets it when it loads, before any PE code runs and so before
  win32u loads the Vulkan loader, where the driver reads it. Like the FEX
  entry, it replaces a value already in the environment.
- The entries and the reasoning for both are in
  [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs). The FEX and
  KosmicKrisp [recipes](#recipes) turn them off or change them.

## WINE_COMPATDB

Whatever starts the process tree can add or change rules through this
environment variable.

```
v=<N>
name=my-game;exe=Game.exe;company=Some Vendor;d3d9=wined3d;dpi_aware=false;env=MTL_HUD_ENABLED=1
name=old-game;exe=OldGame.exe;large_address_aware=false
name=dpi-aware;enabled=false
```

### Format

- The first line is the format header: the value of `HEADER` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs), or of
  `PREVIOUS_HEADER` there, which the library still accepts. `v=<N>` above is
  a placeholder for it.
- Then one rule per line, as `key=value` fields joined by `;`. Blank lines
  are ignored.
- Keys: `name` (required), the matching keys (`exe`, `company`, `product`,
  `original_filename`), the [settings](#settings), and `enabled`. The parser,
  `record_to_rule` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs), is the
  authoritative list.
- Inside a value, `%`, `;`, CR, LF and other control characters are
  percent-encoded (`%3B` for `;`, `%25` for `%`). Everything else, including
  `=` and `,`, passes through.

### Merging with the built-in rules

- A rule whose `name` matches a built-in rule (case-insensitively) is merged
  into it: a set scalar replaces the built-in value, lists append. It needs
  only the fields it changes.
- `enabled=false` drops the rule of that name.
- A rule with a new name needs an `exe`. One without is dropped and logged
  as `override names no rule`.

### Values from the x86_64 Wine

A launcher written for the retired x86_64 Wine may still send what that
Wine understood. The library keeps such a record instead of dropping it:

- `dxgi=gptk` reads as `dxmt`, which is what this Wine always loaded in its
  place; a native ARM64 program still gets `wined3d`.
- `x87_sidecar` is accepted, with any value, and ignored. There is no
  x87sidecar; FEX emulates x87 code
  ([FEX and KosmicKrisp](#fex-and-kosmickrisp)).
- A rule that names a built-in rule only that Wine had, such as
  `name=no-vulkan;enabled=false` or `name=x87-sidecar;enabled=false`, names
  no rule here and is logged as `override names no rule`.

### Errors

Errors never abort a process.

- An unknown key, an unknown `dxgi`, `d3d9` or `wined3d` value, a boolean
  other than `true` or `false` (so `TRUE` or `1` too), a missing `name` or a
  field without `=` drops that line with a diagnostic. The other lines still
  apply.
- A header other than `HEADER` or `PREVIOUS_HEADER` makes the library
  ignore the whole value with one diagnostic. A library older than a format
  change therefore drops a table written with the new header rather than
  half understand it, so a launcher that writes a new `HEADER` needs a
  `compatdb.so` that knows it. The library reads the previous format too,
  so a launcher can move to a new `HEADER` after the library does.

## Recipes

Each line goes after the header line. `Game.exe` stands for the game's
executable.

| Goal | Rule |
|------|------|
| Re-enable Wine's Mono for every process | `name=mono;exe=*;dll_overrides=mscoree=b` |
| Prefer a native .NET | `name=mono;exe=*;dll_overrides=mscoree=n,b` |
| Hand Mono and Gecko back to the registry `DllOverrides` | `name=no-mono-gecko;enabled=false` |
| Make one game DPI-unaware | `name=my-game;exe=Game.exe;dpi_aware=false` |
| Make every process DPI-unaware | `name=global;exe=*;dpi_aware=false` |
| Let the registry and the manifest decide DPI awareness | `name=dpi-aware;enabled=false` |
| Stop forcing 4 GB for one game | `name=my-game;exe=Game.exe;large_address_aware=false` |
| Stop forcing 4 GB for every process | `name=large-address-aware;enabled=false` |
| Force Direct3D trees for one game | `name=my-game;exe=Game.exe;dxgi=wined3d;d3d9=wined3d` |
| Keep no-execute on for one game | `name=my-game;exe=Game.exe;env=WINE_FORCE_NX=1` |
| Give one game FEX's full x87 precision | `name=my-game;exe=Game.exe;env=FEX_X87REDUCEDPRECISION=` |
| Give every process FEX's full x87 precision | `name=fex-x87-reduced-precision;enabled=false` |
| Turn KosmicKrisp's experimental features off for one game | `name=my-game;exe=Game.exe;env=MESA_KK_EXPERIMENTAL=` |
| Advertise only some of them to one game | `name=my-game;exe=Game.exe;env=MESA_KK_EXPERIMENTAL=<flags>`, with the flags comma-separated |
| Turn KosmicKrisp's experimental features off for every process | `name=kosmickrisp-experimental;enabled=false` |
| Put wined3d on Vulkan for one game | `name=my-game;exe=Game.exe;wined3d=vulkan` |

- The KosmicKrisp flags are described in Mesa's
  [`docs/drivers/kosmickrisp.rst`](https://gitlab.freedesktop.org/mesa/mesa/-/blob/main/docs/drivers/kosmickrisp.rst),
  except those our Mesa topic commits add; the ones the built-in rule sets
  are in [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs). A game
  rule's `MESA_KK_EXPERIMENTAL` folds after the built-in one and replaces it
  for that game.
- `WINEDLLOVERRIDES=mscoree=b` alone does not work: ntdll parses that
  variable first, and an override compatdb adds replaces the entry for the
  same module. ntdll also consults compatdb's overrides before the registry
  `DllOverrides` keys.
- A rule works because it folds after the built-in rules and ntdll keeps
  the last entry for a module. A rule for one executable folds later by
  specificity; a new `*` rule folds later by table order, since rules from
  `WINE_COMPATDB` come after the built-in ones.
- The processes a game starts inherit the variables of its `env` rule, so
  `WINE_FORCE_NX` keeps no-execute on in them too. An empty value in a rule
  removes the variable instead of setting it empty, so
  `env=WINE_FORCE_NX=` does not keep no-execute on.

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

## Native ARM64 programs

- A native ARM64 program defaults to `wined3d` in both families: the `dxmt`
  and `mtld3d` trees hold x86 code only, while the `wined3d` trees hold
  Wine's own ARM64X modules. A rule asking such a program for `dxmt` or
  `mtld3d` falls back to `wined3d` with a log line that names the arch.
  `default_for` and `available_for` of `Dxgi` and `D3d9` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs) hold the
  table. (The bundle's placeholders for these DLL names are still x64 ones,
  which keeps a native ARM64 program from loading them at all; see
  [arm64.md](arm64.md#limitations).)
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
  fingerprint, then:
  - `rules = ...` or `no rule matched`;
  - the trees it prepended (`dxgi = <impl> (from <path>)`, `d3d9 = ...`)
    and any fallback from the implementation a rule asked for;
  - the wined3d renderer (`wined3d = <renderer> (...)`): set by a rule,
    asked for by a `WINE_D3D_CONFIG` the process has, or `default (not
    set)`, where wined3d's registry settings or its default decide;
  - the overrides, DPI and large address awareness it set, the arguments
    and the env.
- Parse diagnostics and `override names no rule` lines come just before the
  block.
- A `compatdb: env query <exe> ...` line, with the executable, its
  fingerprint and the `NAME=value` entries in the order ntdll sets them,
  comes from each `compatdb_query_env` call that returns entries. It is
  written in the process that starts the new one, so it appears before
  that process's own block.
- To confirm a tree took effect, look at the running process's mapped files,
  for example `lsof -p <pid> | grep -i dxgi.dll`. A path under
  `lib/wine/dxgi/<impl>/` or `lib/wine/d3d9/<impl>/` proves it.
