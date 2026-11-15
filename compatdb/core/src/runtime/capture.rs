//! Where a capture of [`Rule::vulkan_capture`] and [`Rule::metal_capture`]
//! goes and what `compatdb.so` sets for it.
//!
//! [`Rule::vulkan_capture`]: crate::Rule::vulkan_capture
//! [`Rule::metal_capture`]: crate::Rule::metal_capture

use crate::{EnvEntry, EnvName, FieldError};

/// The folder next to the executable that the captures of a process go
/// into. `compatdb.so` makes it when a rule asks for a capture.
pub const CAPTURES_DIR: &str = "captures";

/// `GFXReconstruct`'s capture layer, as its manifest names it.
pub const VULKAN_CAPTURE_LAYER: &str = "VK_LAYER_LUNARG_gfxreconstruct";

/// The directory of the layer's manifest, relative to the bundle's `lib/wine`,
/// the directory above `compatdb.so`'s.
pub const VULKAN_LAYER_DIR: &str = "../../share/vulkan/explicit_layer.d";

/// What [`vulkan_capture_env`] gives `GFXRECON_CAPTURE_TRIGGER`.
///
/// It is the name of no key, which the layer accepts and never reads as
/// pressed. Any trigger puts the layer into its trimming mode, where it
/// records nothing until win32u starts a capture through
/// `GFXRSetTrimmingState`.
pub const VULKAN_CAPTURE_TRIGGER: &str = "win32u";

/// The capture file `compatdb.so` names for the process `pid` running `exe`
/// in the folder `dir`.
///
/// It is `<dir>/<exe without .exe>-<pid>.gfxr`. The layer
/// inserts `_trim_trigger_<time>` before the extension for each capture,
/// the time its local `yyyymmddThhmmss`, so the files of one process
/// differ by the second the capture started in.
#[must_use]
pub fn vulkan_capture_file(dir: &str, exe: &str, pid: u32) -> String {
    format!("{dir}/{}-{pid}.gfxr", exe_stem(exe))
}

/// `exe` without a `.exe` in any case, unless nothing is left.
fn exe_stem(exe: &str) -> &str {
    exe.len()
        .checked_sub(4)
        .filter(|&at| exe.is_char_boundary(at))
        .and_then(|at| {
            let (stem, ext) = exe.split_at(at);
            ext.eq_ignore_ascii_case(".exe").then_some(stem)
        })
        .filter(|stem| !stem.is_empty())
        .unwrap_or(exe)
}

/// The environment entries that load the capture layer into the process
/// and make it write to `file`.
///
/// `file` is what [`vulkan_capture_file`] names, `layer_dir` holds the
/// layer's manifest, `process_name` is the name the process has for
/// `getprogname()`, and `current` gives the value a variable has without
/// the capture: the one a rule's env entry gives it, else the inherited
/// one, `None` for none.
///
/// - `VK_ADD_LAYER_PATH` puts `layer_dir` ahead of the directories
///   `current` names, so the bundle's layer, the one win32u starts and
///   stops, wins over a layer of the same name elsewhere.
/// - `VK_INSTANCE_LAYERS` puts the layer behind the layers `current` names.
///   The loader orders an instance's layers as listed, the first next to
///   the program, so the capture layer records the program's calls as the
///   other layers pass them on to the driver, which is what a replay
///   without those layers needs.
/// - `GFXRECON_CAPTURE_FILE` and `GFXRECON_CAPTURE_FILE_TIMESTAMP` give the
///   files their names.
/// - `GFXRECON_CAPTURE_TRIGGER` is [`VULKAN_CAPTURE_TRIGGER`]: the layer
///   records nothing until win32u starts a capture on the capture chord,
///   and win32u stops it after the presents it counts.
/// - `GFXRECON_CAPTURE_PROCESS_NAME` is `process_name`, which the layer
///   compares with `getprogname()`, so a program the process starts
///   inherits the variables and loads the layer but records nothing.
/// - `GFXRECON_MEMORY_TRACKING_MODE` is `unassisted`: while it records, the
///   layer writes the whole content of every mapped memory range into the
///   capture at each `vkQueueSubmit` and `vkUnmapMemory`. Its default,
///   `page_guard`, which tracks the writes through protected copies of the
///   mapped memory, loses the program's writes to mapped memory on macOS
///   arm64, in Wine and outside it, so the program reads zeros back.
///
/// A list that names the directory or the layer already gets it moved to
/// that side, not a second time.
///
/// # Errors
///
/// [`FieldError::Nul`] when `layer_dir`, `file`, `process_name` or a value
/// of `current` holds a NUL.
pub fn vulkan_capture_env(
    layer_dir: &str,
    file: &str,
    process_name: &str,
    current: impl Fn(&str) -> Option<String>,
) -> Result<Vec<EnvEntry>, FieldError> {
    let layer_path = list_with(current("VK_ADD_LAYER_PATH").as_deref(), layer_dir, true);
    let layers = list_with(
        current("VK_INSTANCE_LAYERS").as_deref(),
        VULKAN_CAPTURE_LAYER,
        false,
    );
    [
        ("VK_ADD_LAYER_PATH", layer_path.as_str()),
        ("VK_INSTANCE_LAYERS", layers.as_str()),
        ("GFXRECON_CAPTURE_FILE", file),
        ("GFXRECON_CAPTURE_FILE_TIMESTAMP", "true"),
        ("GFXRECON_CAPTURE_TRIGGER", VULKAN_CAPTURE_TRIGGER),
        ("GFXRECON_CAPTURE_PROCESS_NAME", process_name),
        ("GFXRECON_MEMORY_TRACKING_MODE", "unassisted"),
    ]
    .into_iter()
    .map(|(name, value)| EnvEntry::new(EnvName::new(name)?, value))
    .collect()
}

/// The `:`-separated list `list` with `item` at its front, or at its back,
/// and nowhere else. Empty elements are dropped.
fn list_with(list: Option<&str>, item: &str, front: bool) -> String {
    let rest = list
        .unwrap_or_default()
        .split(':')
        .filter(|element| !element.is_empty() && *element != item);
    let items: Vec<&str> = if front {
        std::iter::once(item).chain(rest).collect()
    } else {
        rest.chain(std::iter::once(item)).collect()
    };
    items.join(":")
}

/// The environment entries of [`Rule::metal_capture`](crate::Rule::metal_capture)
/// for captures of the process `pid` running `exe` into the folder `dir`:
///
/// - `MTL_CAPTURE_ENABLED` loads Metal's capture layer, without which no
///   backend can capture.
/// - `MESA_KK_GPU_CAPTURE` has KosmicKrisp start and stop a capture on the
///   queue labels win32u inserts on the capture chord, and
///   `MESA_KK_GPU_CAPTURE_DIRECTORY` names the folder its files go into.
///   KosmicKrisp names them itself,
///   `<dir>/<process name without .exe>-<pid>-<n>.gputrace`, after
///   `getprogname()`, which Wine sets to the executable's file name as the
///   path it was started by spells it: `wine game` gives `game.exe`.
/// - `DXMT_CAPTURE_EXECUTABLE` is `exe` without `.exe`, which turns DXMT's
///   capture on in that program alone, and `DXMT_CAPTURE_FILE` the files'
///   path without the number DXMT adds, `<dir>/<exe without .exe>-<pid>-dxmt`,
///   so DXMT writes `<dir>/<exe without .exe>-<pid>-dxmt-<n>.gputrace` and
///   no name of KosmicKrisp's.
///
/// # Errors
///
/// [`FieldError::Nul`] when `dir` or `exe` holds a NUL.
pub fn metal_capture_env(dir: &str, exe: &str, pid: u32) -> Result<Vec<EnvEntry>, FieldError> {
    let stem = exe_stem(exe);
    let file = format!("{dir}/{stem}-{pid}-dxmt");
    [
        ("MTL_CAPTURE_ENABLED", "1"),
        ("MESA_KK_GPU_CAPTURE", "1"),
        ("MESA_KK_GPU_CAPTURE_DIRECTORY", dir),
        ("DXMT_CAPTURE_EXECUTABLE", stem),
        ("DXMT_CAPTURE_FILE", &file),
    ]
    .into_iter()
    .map(|(name, value)| EnvEntry::new(EnvName::new(name)?, value))
    .collect()
}

/// The value ntdll's `set_compat_capture` takes: a mask of what win32u
/// records on the capture chord, `1` for the Vulkan capture and `2` for
/// KosmicKrisp's Metal capture.
#[must_use]
pub const fn capture_hook_value(vulkan: bool, metal: bool) -> i32 {
    (if vulkan { 1 } else { 0 }) | (if metal { 2 } else { 0 })
}

#[cfg(test)]
mod tests;
