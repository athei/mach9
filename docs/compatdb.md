# compatdb.so

- Built from the Cargo workspace in [`compatdb/`](../compatdb):
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

The library writes to Wine's stderr; every line starts with `compatdb:`.

- Once in every process at load it writes a block starting
  `compatdb: <exe> [<arch>]`, with the image name, arch and version
  fingerprint. The block is the whole configuration of the process: every
  setting gets a line with its value and its source, or, where no rule set
  it, `unset (upstream: ...)` with what upstream Wine does instead, followed
  by `reset by <source>` when the last rule for it set it to `default`.
  Those descriptions are [`Key::upstream`][upstream].
- The arch is `x86_64 (arm64ec)` for an x86_64 program, which runs in an
  ARM64EC process, and `aarch64` for a native ARM64 program. The main
  image's machine type tells them apart; a wow64 process is i386.
- A source is `built-in` for a built-in rule that matches every process,
  `built-in for <exe> (<fingerprint values>)` for a built-in rule for one
  program, with the values it pins, and `WINE_COMPATDB line <n>` for a
  rule from that variable ([format][format]).
- After the first line come:
  - a `warning: WINE_COMPATDB line <n>: ...` line for each field or line
    of `WINE_COMPATDB` it ignored ([parsing][parsing]);
  - the trees it prepended (`dxgi = <impl> (from <path>), <source>`,
    `d3d9 = ...`); a family no rule chose gets `wined3d`;
  - the wined3d renderer: `wined3d = <renderer>, <source>` when a rule set
    it, whose `WINE_D3D_CONFIG` the `env` lines show; otherwise the
    renderer a `WINE_D3D_CONFIG` the process has asks for, from a rule's
    `env` or inherited, as the line says, or unset, where wined3d's
    registry settings or its default decide;
  - `metal_hud = <value>, <source>`, whose variables the `env` lines show;
  - `dll_overrides` and `arguments`, each entry with its source, and for an
    argument whether it was appended or already present;
  - `debug_channels`, each entry with its source, which the library only
    logs;
  - one `dll_overrides <module>=default, <source>: ...` line for each
    module an entry with the order `default` names, with the sources of
    the overrides it removed, or saying there was none to remove;
  - one `arguments_override, <source>: dropped ...` line for each
    `arguments_override` that dropped arguments, naming each with its
    source;
  - `dpi_aware`, `large_address_aware`, `gcmouse`, `force_nx` and
    `opengl` (`large_address_aware` and `force_nx` noting that they have no
    effect in a 64-bit process);
  - one `env NAME=value, <source>: ...` line per entry, saying whether it
    was set, removed (naming the source of the earlier entry whose value it
    undoes, or saying the variable was not set, so there was nothing to
    remove), left alone because the process had the variable, with the
    value it kept ([`Rule::env`][env]), or replaced by a later entry for
    the same variable, whose source it names, so the last entry that
    applies is the value the process ends up with.
- A line also says when ntdll lacks the hook a setting needs, and the
  setting is ignored.
- To confirm a tree took effect, look at the running process's mapped files,
  for example `lsof -p <pid> | grep -i dxgi.dll`. A path under
  `lib/wine/dxgi/<impl>/` or `lib/wine/d3d9/<impl>/` proves it.

[spec]: https://athei.github.io/wine-build/compatdb_core/
[format]: https://athei.github.io/wine-build/compatdb_core/#format
[parsing]: https://athei.github.io/wine-build/compatdb_core/#strict-and-lenient-parsing
[recipes]: https://athei.github.io/wine-build/compatdb_core/#recipes
[upstream]: https://athei.github.io/wine-build/compatdb_core/enum.Key.html#method.upstream
[runtime]: https://athei.github.io/wine-build/compatdb_core/runtime/index.html
[dxgi]: https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.dxgi
[d3d9]: https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.d3d9
[env]: https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.env
