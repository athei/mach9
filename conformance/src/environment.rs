//! The environment every Wine process of a run gets: the caller's, without
//! anything that changes Wine, its graphics stack or the test framework,
//! plus the settings of the runner's prefix.

use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

/// The name prefixes of the caller's variables that are removed.
///
/// `WINE` covers every `WINETEST_*`, so `WINETEST_DEBUG`,
/// `WINETEST_REPORT_FLAKY`, `WINETEST_PLATFORM` and `WINETEST_COLOR`, which
/// change what [`crate::output`] reads, never reach a test; `WT_` covers
/// the selection. `GALLIUM_`, `LIBGL_`, `EGL_`, `NIR_` and `MVK_` change
/// Mesa and `MoltenVK`; `DYLD_` changes which libraries load, and cargo
/// sets `DYLD_FALLBACK_LIBRARY_PATH` for the processes it runs.
pub const SCRUBBED: [&str; 17] = [
    "WINE", "FEX_", "VK_", "VKD3D_", "MESA_", "MTL_", "METAL_", "DXMT_", "GST_", "ZINK_", "WT_",
    "GALLIUM_", "LIBGL_", "EGL_", "MVK_", "NIR_", "DYLD_",
];

/// The caller's variables without the [`SCRUBBED`] ones, then:
///
/// - `WINEPREFIX` the runner's `prefix`;
/// - `WINEDLLOVERRIDES=mscoree,mshtml=`, so no Mono or Gecko installer
///   opens a dialog;
/// - `WINEDEBUG=-all`;
/// - `WINETEST_NO_MT_D3D=1`, which runs the queued tests of the d3d units
///   one after another instead of on several threads at once;
/// - `WINETEST_TIME=1`, which puts the elapsed time on every check line;
/// - `VKD3D_SHADER_CACHE_PATH` the runner's `work` directory, since
///   vkd3d-proton writes `vkd3d-proton.cache` into the working directory
///   otherwise.
///
/// The selection variables (`WT_ONLY`, `WT_LIST`) are the caller's to add
/// per process.
#[must_use]
pub fn wine_env(
    caller: impl IntoIterator<Item = (OsString, OsString)>,
    prefix: &Path,
    work: &Path,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = caller
        .into_iter()
        .filter(|(name, _)| !is_scrubbed(name))
        .collect();
    for (name, value) in [
        ("WINEPREFIX", prefix.as_os_str()),
        ("WINEDLLOVERRIDES", OsStr::new("mscoree,mshtml=")),
        ("WINEDEBUG", OsStr::new("-all")),
        ("WINETEST_NO_MT_D3D", OsStr::new("1")),
        ("WINETEST_TIME", OsStr::new("1")),
        ("VKD3D_SHADER_CACHE_PATH", work.as_os_str()),
    ] {
        env.push((OsString::from(name), value.to_owned()));
    }
    env
}

/// The environment of every `deqp-vk` process of the deqp-vk suite: the
/// caller's variables without the [`SCRUBBED`] ones, then:
///
/// - `VK_DRIVER_FILES` the manifest directory `icd_dir`, so the loader
///   loads that driver and no other;
/// - `VK_LOADER_LAYERS_DISABLE=~all~`, so no layer found on the system
///   loads;
/// - `MESA_SHADER_CACHE_DIR` a directory in the runner's `work` directory,
///   so the cases' shaders do not fill the cache the user's games share;
/// - `MESA_KK_DEBUG` the `kk_debug` of a group that sets one
///   ([`crate::deqp::groups::Group::kk_debug`]).
#[must_use]
pub fn deqp_env(
    caller: impl IntoIterator<Item = (OsString, OsString)>,
    icd_dir: &Path,
    work: &Path,
    kk_debug: Option<&str>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = caller
        .into_iter()
        .filter(|(name, _)| !is_scrubbed(name))
        .collect();
    let cache = work.join("mesa-shader-cache");
    for (name, value) in [
        ("VK_DRIVER_FILES", icd_dir.as_os_str()),
        ("VK_LOADER_LAYERS_DISABLE", OsStr::new("~all~")),
        ("MESA_SHADER_CACHE_DIR", cache.as_os_str()),
    ] {
        env.push((OsString::from(name), value.to_owned()));
    }
    if let Some(flags) = kk_debug {
        env.push((OsString::from("MESA_KK_DEBUG"), OsString::from(flags)));
    }
    env
}

/// Whether the variable `name` is one [`wine_env`] removes.
fn is_scrubbed(name: &OsStr) -> bool {
    let name = name.as_encoded_bytes();
    SCRUBBED
        .iter()
        .any(|prefix| name.starts_with(prefix.as_bytes()))
}

#[cfg(test)]
mod tests;
