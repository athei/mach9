# wine-build

A relocatable, signed Wine for Apple Silicon Macs, made to be embedded in a
launcher. The launcher configures it through one environment variable,
`WINE_COMPATDB`, with rules that match processes, so each game gets the
Direct3D implementation
([backends](docs/compatdb.md#direct3d-backends)) and the settings it
needs. The API that writes the rules is [published][spec], so a launcher
generates them in code.

For example, a launcher that wants one game's D3D9 on wined3d sets:

```bash
export WINE_COMPATDB='exe=game.exe;d3d9=wined3d'
```

`game.exe` then gets the wined3d D3D9 tree while every other process keeps
mtld3d; the [format][format] and the [recipes][recipes] show the rest.

- Wine is built for arm64 and runs without Rosetta.
  [FEX](docs/internals.md#fex) runs the x86 code of Windows programs.
- It is aimed at old games. D3D9 runs on
  [mtld3d](docs/internals.md#mtld3d), a D3D9 on Metal, and x87 code runs
  fast because FEX's reduced x87 precision is on by default
  ([`builtin::fex_reduced_x87_precision`][x87]).
- Modern games run too: D3D10 and D3D11 on
  [DXMT](docs/internals.md#dxmt), D3D12 on
  [vkd3d-proton](docs/internals.md#vkd3d-proton). x64 programs run as
  ARM64EC: Wine and the graphics stack are ARM64X and run natively, and
  only the game's own code is emulated. Unlike upstream FEX, our FEX runs
  the x86_64 code of these programs without AFP: AFP needs a
  floating-point mode switch at every call between x86 and native code,
  and measurements showed that switch costs more than AFP saves
  ([FEX](docs/internals.md#fex)).

Other defaults that differ from Wine, besides the built-in rules
([`builtin::defaults`][defaults]):

- msync ([runtime](docs/internals.md#runtime))
- the hardware x86 memory ordering ([FEX](docs/internals.md#fex))
- the bundled Vulkan driver with its features always on
  ([Vulkan](docs/internals.md#vulkan))
- D3D12 frame latency handles released when the GPU is done
  ([vkd3d-proton](docs/internals.md#vkd3d-proton))

## Using it from a launcher

- Get a bundle from the `arm64-*`
  [releases](https://github.com/athei/mach9/releases)
  ([releases](docs/maintenance.md#releases)) or a local build
  ([building](docs/building.md)).
- Copy the `wine/` tree into the app
  ([bundle layout](docs/building.md#bundle-layout)).
- Sign the app without touching `wine.app`
  ([loader app](docs/internals.md#loader-app)).
- Minimum macOS: [macOS floors](docs/maintenance.md#macos-floors).
- Prefixes after an upgrade:
  [upgrade checklist](docs/maintenance.md#upgrade-checklist).

## Per-program configuration

The rules in `WINE_COMPATDB` are written with the `compatdb-core` crate,
used at the same release as the bundle:

```toml
[dependencies]
compatdb-core = { git = "https://github.com/athei/mach9", tag = "<the bundle's arm64-* tag>" }
```

- The [crate documentation][spec] is the spec; it has only a Rust API.
- A complete example: [Quick start][quick-start].
- What a launcher must do: [launcher responsibilities][launcher].
- Each process logs its configuration
  ([logging](docs/compatdb.md#logging-and-verification)).

## Environment variables

Variables the bundle reads besides `WINE_COMPATDB`:

| Variable | Explained in |
|----------|--------------|
| `WINEMSYNC`, `WINENCPU` | [runtime](docs/internals.md#runtime) |
| `FEX_HARDWARE_TSO`, `FEX_HOSTFEATURES` | [FEX](docs/internals.md#fex) |
| `VK_DRIVER_FILES`, `VK_ICD_FILENAMES`, `VK_ADD_DRIVER_FILES`, `MESA_KK_EXPERIMENTAL` | [Vulkan](docs/internals.md#vulkan) |
| `GST_PLUGIN_SYSTEM_PATH_1_0`, `GST_REGISTRY_1_0`, each also without `_1_0` | [GStreamer](docs/internals.md#gstreamer) |
| `VKD3D_CONFIG`, `VKD3D_DEBUG` and vkd3d-proton's other variables | [upstream](https://github.com/HansKristian-Work/vkd3d-proton#readme) |
| `WINE_D3D_CONFIG` | [`Rule::wined3d`][wined3d] |

## Building

- [docs/building.md](docs/building.md): building the bundle locally.
- [docs/internals.md](docs/internals.md): how the build works.
- [docs/compatdb.md](docs/compatdb.md): how `compatdb.so` applies the
  rules.
- [docs/maintenance.md](docs/maintenance.md): maintaining the build.

[spec]: https://docs.mach9.app/compatdb_core/
[format]: https://docs.mach9.app/compatdb_core/#format
[recipes]: https://docs.mach9.app/compatdb_core/#recipes
[quick-start]: https://docs.mach9.app/compatdb_core/#quick-start
[launcher]: https://docs.mach9.app/compatdb_core/#launcher-responsibilities
[defaults]: https://docs.mach9.app/compatdb_core/builtin/fn.defaults.html
[x87]: https://docs.mach9.app/compatdb_core/builtin/fn.fex_reduced_x87_precision.html
[wined3d]: https://docs.mach9.app/compatdb_core/struct.Rule.html#method.wined3d
