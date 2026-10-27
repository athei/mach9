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
- At load it reads the process's image name and version resource, takes
  the built-in rules (`Table::builtin`) and the rules of `WINE_COMPATDB`
  (`Table::parse_lenient`), resolves those that match and applies them
  through ntdll's hooks
  ([Wine-side contract](maintenance.md#wine-side-contract)).

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
  program, and `WINE_COMPATDB line <n>` for a rule from that variable
  ([format][format]).

| Line | What it shows |
|------|---------------|
| `<exe> [<arch>]` | The image name, arch and version fingerprint. The arch is `x86_64 (arm64ec)` for an x86_64 program, `aarch64` for a native ARM64 one and `i386` for a wow64 one |
| `warning: WINE_COMPATDB line <n>: ...` | A field or line of `WINE_COMPATDB` it ignored ([parsing][parsing]) |
| `dxgi = <impl> (from <path>), <source>`, `d3d9 = ...` | The trees it prepended; a family no rule chose gets `wined3d` |
| `wined3d = ...` | The wined3d renderer a rule set, or the one a `WINE_D3D_CONFIG` from a rule's `env` or inherited asks for; unset, wined3d's registry settings or its default decide |
| `metal_hud = <value>, <source>` | The HUD setting; its variables are in the `env` lines |
| `dll_overrides`, `arguments` | Each entry with its source; for an argument, whether it was appended or already present |
| `debug_channels` | Each entry with its source; only logged |
| `dll_overrides <module>=default, <source>: ...` | The overrides an entry with the order `default` removed, or that there was none |
| `arguments_override, <source>: dropped ...` | The arguments it dropped, each with its source |
| `dpi_aware`, `large_address_aware`, `gcmouse`, `force_nx`, `opengl` | The value; `large_address_aware` and `force_nx` note that they do nothing in a 64-bit process |
| `env NAME=value, <source>: ...` | Whether the entry was set, removed, left alone because the process had the variable ([`Rule::env`][env]), or replaced by a later entry, with the sources involved |
| `<setting> = <value>, <source>: ntdll lacks <hook>, ignored` | ntdll lacks the hook; the setting is ignored |

To confirm a tree took effect, look at the running process's mapped files,
for example `lsof -p <pid> | grep -i dxgi.dll`. A path under
`lib/wine/dxgi/<impl>/` or `lib/wine/d3d9/<impl>/` proves it.

[spec]: https://docs.mach9.app
[format]: https://docs.mach9.app/compatdb_core/#format
[parsing]: https://docs.mach9.app/compatdb_core/#strict-and-lenient-parsing
[recipes]: https://docs.mach9.app/compatdb_core/#recipes
[upstream]: https://docs.mach9.app/compatdb_core/enum.Key.html#method.upstream
[runtime]: https://docs.mach9.app/compatdb_core/runtime/index.html
[dxgi]: https://docs.mach9.app/compatdb_core/struct.Rule.html#method.dxgi
[d3d9]: https://docs.mach9.app/compatdb_core/struct.Rule.html#method.d3d9
[env]: https://docs.mach9.app/compatdb_core/struct.Rule.html#method.env
