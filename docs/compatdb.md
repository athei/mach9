# compatdb.so

- Built from the crates in [`compatdb/`](../compatdb):
  `compatdb-table` ([`compatdb/table`](../compatdb/table)) holds the pure
  rule logic, `compatdb` the cdylib around it.
- Installed as `lib/wine/<host>-unix/compatdb.so`, next to ntdll's unix
  half (`x86_64-unix`, or `aarch64-unix` in the
  [arm64 Wine](#arm64-wine)). Wine's ntdll dlopens
  `<ntdll_dir>/compatdb.so` in every process before any PE code runs, the
  slot CrossOver's own compat database uses.
- At load it reads the process's image name and version resource, resolves
  the matching rules (the built-in ones plus
  [`WINE_COMPATDB`](#wine_compatdb)) and applies them. The x86_64 Wine's
  ntdll also asks for `x87_sidecar` and `env` before it starts a process
  (see [Matching](#matching)).
- The same source builds the library for the x86_64 Wine and for an arm64
  one; [arm64 Wine](#arm64-wine) lists what differs.

## Settings

A setting a rule leaves unset is no opinion: a less specific rule decides,
and with none, Wine does.

| Key | Values | Effect | Applied |
|-----|--------|--------|---------|
| `dxgi` | `gptk`, `dxmt`, `wined3d` | The D3D10 to D3D12 tree to prepend. Unset: the arch default ([x86_64 Wine](../README.md#direct3d-backends), [arm64 Wine](#arm64-wine)). `gptk` becomes `dxmt` where it cannot load ([gptk availability](#gptk-availability)) | At load |
| `d3d9` | `mtld3d`, `wined3d` | The D3D9 tree to prepend. Unset: `mtld3d`, except for a native ARM64 program ([arm64 Wine](#arm64-wine)) | At load |
| `dll_overrides` | `WINEDLLOVERRIDES`-style `names=order`, repeatable | Added through ntdll's load-order hook | At load |
| `dpi_aware` | `true`, `false` | `true`: DPI-aware (system-aware). `false`: unaware. win32u applies it before the `AppCompatFlags\Layers` registry value and the manifest, and the first setting wins, so the rule beats both. `true` also caps a program that asks for per-monitor awareness at system-aware | At load, read by win32u |
| `large_address_aware` | `true`, `false` | `true`: a 32-bit process gets 4 GB even without the large-address-aware flag. `false` only stops forcing it: the executable's flag, `WINE_LARGE_ADDRESS_AWARE` and the `AppDefaults` registry value decide. No effect on 64-bit | At load; ntdll applies it after the library returns |
| `x87_sidecar` | `true`, `false` | `true` attaches the x87sidecar, `false` keeps it away. Only matters for an i386 process under Rosetta; the arm64 library accepts the key and ignores it | Before exec, `compatdb_query_x87` |
| `arguments` | text, repeatable | Appended to the command line unless already present (case-sensitive), which leaves a Chromium child that inherited its parent's switches alone | At load |
| `env` | `NAME=value`, repeatable; an empty value removes the variable | In the x86_64 Wine, set in the unix environment right before exec, so what runs ahead of the program sees it: its x87sidecar (the `X87_*` variables) and Rosetta. Written into the environment block and the unix environment at load, before any PE code runs, which is the only time the [arm64 Wine](#arm64-wine) sets it. Processes the program starts inherit it | Before exec, `compatdb_query_env` (x86_64 Wine); at load |

The boolean keys take exactly `true` or `false`, case-sensitive. `dxgi` and
`d3d9` values are case-insensitive.

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
- In the x86_64 Wine, `x87_sidecar` and `env` are also answered before
  exec, in the process that starts the new one; the arm64 Wine's ntdll does
  not ask. Those queries match the executable's file on disk (its basename
  and the version resource read from the file).
- For the first process of a launch, ntdll maps a DOS path such as `C:\...`
  through the prefix's `dosdevices` to a unix path. When that fails, only the
  basename is known, and only rules without a fingerprint can match.

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

## WINE_COMPATDB

Whatever starts the process tree can add or change rules through this
environment variable.

```
v=<N>
name=my-game;exe=Game.exe;company=Some Vendor;d3d9=wined3d;dpi_aware=false;env=MTL_HUD_ENABLED=1
name=old-game;exe=OldGame.exe;large_address_aware=false;x87_sidecar=false
name=dpi-aware;enabled=false
```

### Format

- The first line is the format header: the value of `HEADER` in
  [`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs). `v=<N>` above
  is a placeholder for it.
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

### Errors

Errors never abort a process.

- An unknown key, an unknown `dxgi` or `d3d9` value, a boolean other than
  `true` or `false` (so `TRUE` or `1` too), a missing `name` or a field
  without `=` drops that line with a diagnostic. The other lines still apply.
- A header other than `HEADER` makes the library ignore the whole value with
  one diagnostic. That keeps a format change safe for long-lived processes,
  and it means a launcher and `compatdb.so` must be updated together when
  `HEADER` changes.

## Recipes

Each line goes after the header line. `Game.exe` stands for the game's
executable.

| Goal | Rule |
|------|------|
| Let one game load its own Vulkan loader | `name=my-game;exe=Game.exe;dll_overrides=vulkan-1=n` |
| Stop disabling Vulkan for every process | `name=no-vulkan;enabled=false` |
| Re-enable Wine's Mono for every process | `name=mono;exe=*;dll_overrides=mscoree=b` |
| Prefer a native .NET | `name=mono;exe=*;dll_overrides=mscoree=n,b` |
| Hand Mono and Gecko back to the registry `DllOverrides` | `name=no-mono-gecko;enabled=false` |
| Make one game DPI-unaware | `name=my-game;exe=Game.exe;dpi_aware=false` |
| Make every process DPI-unaware | `name=global;exe=*;dpi_aware=false` |
| Let the registry and the manifest decide DPI awareness | `name=dpi-aware;enabled=false` |
| Stop forcing 4 GB for one game | `name=my-game;exe=Game.exe;large_address_aware=false` |
| Stop forcing 4 GB for every process | `name=large-address-aware;enabled=false` |
| Keep the x87sidecar away from one game | `name=my-game;exe=Game.exe;x87_sidecar=false` |
| Keep the x87sidecar away from every process | `name=no-x87;exe=*;x87_sidecar=false`, or set `ROSETTA_X87_PATH` to an empty value |
| Force Direct3D trees for one game | `name=my-game;exe=Game.exe;dxgi=wined3d;d3d9=wined3d` |
| Let one game turn no-execute off (x86_64 Wine) | `name=my-game;exe=Game.exe;env=WINE_FORCE_NX=0` |
| Keep no-execute on for one game (arm64 Wine) | `name=my-game;exe=Game.exe;env=WINE_FORCE_NX=1` |
| Give one game FEX's full x87 precision (arm64 Wine) | `name=my-game;exe=Game.exe;env=FEX_X87REDUCEDPRECISION=` |
| Give every process FEX's full x87 precision (arm64 Wine) | `name=fex-x87-reduced-precision;enabled=false` |

- `WINEDLLOVERRIDES=vulkan-1=n` or `mscoree=b` alone does not work: ntdll
  parses that variable first, and an override compatdb adds replaces the
  entry for the same module. ntdll also consults compatdb's overrides before
  the registry `DllOverrides` keys.
- A rule works because it folds after the built-in rules and ntdll keeps
  the last entry for a module. A rule for one executable folds later by
  specificity; a new `*` rule folds later by table order, since rules from
  `WINE_COMPATDB` come after the built-in ones.
- Dropping `x87-sidecar` with `enabled=false` is not enough to keep the
  sidecar away: the library then has no opinion, and ntdll attaches it
  anyway.
- The processes a game starts inherit the variables of its `env` rule, so
  `WINE_FORCE_NX` keeps no-execute on in them too. An empty value in a rule
  removes the variable instead of setting it empty, so in the arm64 Wine
  `env=WINE_FORCE_NX=` does not keep no-execute on.

## gptk availability

- Apple ships no i386 D3DMetal and none for an arm64 Wine, and the x86_64
  one needs at least `GPTK_MIN` in
  [`compatdb/src/macos.rs`](../compatdb/src/macos.rs).
- The arm64 library never reads the macOS version: `gptk` cannot load
  there on any release.
- Where `gptk` cannot load, a process that would get it, from the x86_64
  default or from a rule, gets `dxmt` instead, and the log says why. So
  `dxgi=gptk` in a rule means "D3DMetal where it loads".
- The library reads the macOS version from `kern.osproductversion`.
- macOS puts a compatibility value (major version 10 or 16) there for a
  process that runs with `SYSTEM_VERSION_COMPAT=1` or whose main binary was
  built against an old SDK.
- The Darwin release in `kern.osrelease` decides instead (against
  `GPTK_MIN_DARWIN`), and the log names it, when:
  - `kern.system_version_compat` is nonzero,
  - the major version is 10 or 16, or
  - the value cannot be read or parsed.
- When neither is usable, the library assumes `gptk` loads.

## arm64 Wine

The library built for an arm64 Wine (`aarch64-apple-darwin`) applies the
same rules, with these differences. They are chosen by `Host` in
[`compatdb/table/src/lib.rs`](../compatdb/table/src/lib.rs) and
`table_for` in [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs).

- There is no Rosetta, so no x87sidecar: the built-in `x87-sidecar` rule
  and the `compatdb_query_x87` export are absent. `x87_sidecar` in
  `WINE_COMPATDB` still parses, so a launcher can send the same rules to
  both Wines; `name=x87-sidecar;enabled=false` is logged as
  `override names no rule`.
- Instead, the built-in `fex-x87-reduced-precision` rule sets an `env` entry
  for every process that has FEX emulate x87 code with reduced precision,
  which is much faster and slightly less accurate. FEX reads it from the
  process's environment block when it starts, which compatdb.so has
  already rewritten. The entry and the reasoning are in
  [`compatdb/src/builtin.rs`](../compatdb/src/builtin.rs). Neither a FEX
  config file nor the variable exported before launch overrides it: FEX's
  environment layer outranks its config files, and the rule replaces a
  value already in the environment. To turn it off for one game or for
  every process, use the two FEX recipes in [recipes](#recipes).
- `gptk` is never available, so an x86 process defaults to `dxmt`, and a
  `dxgi=gptk` rule falls back with a log line that names the arm64 Wine, not
  the macOS release.
- A native ARM64 program defaults to `wined3d` in both families: the `dxmt`
  and `mtld3d` trees of the arm64 bundle hold x86 code only, while the
  `wined3d` trees hold Wine's own ARM64X modules. A rule asking such a
  program for `dxmt`, `gptk` or `mtld3d` falls back to `wined3d` with a log
  line that names the arch. `default_for` and `available_for` of `Dxgi` and
  `D3d9` hold the table. (The bundle's placeholders for these DLL names are
  still x64 ones, which keeps a native ARM64 program from loading them at
  all; see [arm64.md](arm64.md#limitations).)
- A 64-bit process loads the Direct3D trees from `aarch64-windows`, as Wine
  on an arm64 host loads its builtins from there. An i386 process still
  uses `i386-windows`.
- The log labels an x86_64 program, which runs in an ARM64EC process, as
  `x86_64 (arm64ec)` and a native ARM64 program as `aarch64`. The main
  image's machine type tells them apart; a wow64 process is i386.

## Logging and verification

The library writes to Wine's stderr; every line starts with `compatdb:`.

| Line | Written by | Shows |
|------|------------|-------|
| A block starting `compatdb: <exe> [<arch>]` | compatdb.so, once in every process at load | Image name, arch and version fingerprint; `rules = ...` or `no rule matched`; the trees it prepended (`dxgi = <impl> (from <path>)`, `d3d9 = ...`) and any fallback from the implementation a rule asked for; the overrides, DPI and large address awareness it set; `x87_sidecar (rule)` for an i386 process (x86_64 only); arguments and env. Parse diagnostics and `override names no rule` lines come just before the block |
| `compatdb: x87 query <exe> ...` | compatdb.so (x86_64 only), per `compatdb_query_x87` call | The executable, its fingerprint and the `x87_sidecar (rule)` answer |
| `compatdb: env query <exe> ...` | compatdb.so, per `compatdb_query_env` call that returns entries | The executable, its fingerprint and the `NAME=value` entries in the order ntdll sets them; nothing when there are none |
| `... attaching rosettax87 --cooperative ...` | ntdll, on the `err:module` channel | That the sidecar was actually started |

- Both queries run in the process that starts the new one, so their lines
  appear before that process's own block.
- They show only what the rules say. ntdll still skips the sidecar when
  `ROSETTA_X87_PATH` is empty, when there is no executable sidecar, for a
  64-bit process, or when the ntdll lacks the query.
- To confirm a tree took effect, look at the running process's mapped files,
  for example `lsof -p <pid> | grep -i dxgi.dll`. A path under
  `lib/wine/dxgi/<impl>/` or `lib/wine/d3d9/<impl>/` proves it.
