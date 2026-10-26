# wine-build

A relocatable, signed Wine for macOS on Apple Silicon, made to be embedded
in a launcher, and [`compatdb-core`](compatdb/core), the Rust crate that
writes its per-program configuration. What it is built from:
[the arm64 build](docs/arm64.md).

## Using it from a launcher

- Get a bundle from the `arm64-*`
  [releases](https://github.com/athei/wine-build/releases)
  ([releases](docs/maintenance.md#releases)), or from a local build
  ([building](docs/building.md)). The `cx-*` releases are the frozen
  x86_64 Wine this repository built before, run under Rosetta.
- Copy the `wine/` tree into the app
  ([bundle layout](docs/building.md#bundle-layout)).
- Signing an app that embeds the bundle:
  [loader app](docs/arm64.md#loader-app).
- The minimum macOS: [macOS floors](docs/maintenance.md#macos-floors).
- Prefixes after an upgrade:
  [upgrade checklist](docs/maintenance.md#upgrade-checklist).

## Per-program configuration

A launcher passes per-program rules to the bundle's `compatdb.so` in
`WINE_COMPATDB`, written with the `compatdb-core` crate. The crate and
the bundle are used at the same release:

```toml
[dependencies]
compatdb-core = { git = "https://github.com/athei/wine-build", tag = "<the bundle's arm64-* tag>" }
```

The [crate documentation][spec] is the spec. A complete example is in
the [Quick start][quick-start]. What a launcher must do is in
[launcher responsibilities][launcher]; each process logs its
configuration ([logging](docs/compatdb.md#logging-and-verification)).

The crate has only a Rust API.

## Environment variables

Variables the bundle reads besides `WINE_COMPATDB`:

| Variable | Explained in |
|----------|--------------|
| `WINEMSYNC`, `WINENCPU` | [runtime](docs/arm64.md#runtime) |
| `FEX_HARDWARE_TSO`, `FEX_HOSTFEATURES` | [FEX](docs/arm64.md#fex) |
| `VK_DRIVER_FILES`, `VK_ICD_FILENAMES`, `VK_ADD_DRIVER_FILES`, `MESA_KK_EXPERIMENTAL` | [Vulkan](docs/arm64.md#vulkan) |
| `GST_PLUGIN_SYSTEM_PATH_1_0`, `GST_REGISTRY_1_0`, each also without `_1_0` | [GStreamer](docs/arm64.md#gstreamer) |
| `VKD3D_CONFIG`, `VKD3D_DEBUG` and vkd3d-proton's other variables | [upstream](https://github.com/HansKristian-Work/vkd3d-proton#readme) |
| `WINE_D3D_CONFIG` | [`Rule::wined3d`][wined3d] |

## Defaults that differ from Wine

[`builtin::defaults`][defaults] lists the built-in rules for every
process. The defaults that are not compatdb rules:

- msync: [runtime](docs/arm64.md#runtime)
- the hardware x86 memory ordering, and no AFP in x86_64 programs:
  [FEX](docs/arm64.md#fex)
- KosmicKrisp's features, and the bundled Vulkan driver:
  [Vulkan](docs/arm64.md#vulkan)
- D3D12 frame latency handles: [vkd3d-proton](docs/arm64.md#vkd3d-proton)

## Building

- [docs/building.md](docs/building.md): building the bundle locally.
- [docs/arm64.md](docs/arm64.md): how the build works.
- [docs/compatdb.md](docs/compatdb.md): how `compatdb.so` applies the
  rules.
- [docs/maintenance.md](docs/maintenance.md): maintaining the build.

[spec]: https://athei.github.io/wine-build/compatdb_core/
[quick-start]: https://athei.github.io/wine-build/compatdb_core/#quick-start
[launcher]: https://athei.github.io/wine-build/compatdb_core/#launcher-responsibilities
[defaults]: https://athei.github.io/wine-build/compatdb_core/builtin/fn.defaults.html
[wined3d]: https://athei.github.io/wine-build/compatdb_core/struct.Rule.html#method.wined3d
