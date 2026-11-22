//! `vulkan_capture` and `metal_capture`: the folder `captures` next to the
//! executable, the variables that load the capture layers, and the hook that
//! has win32u start a capture on the capture chord.

use std::{
    ffi::{CString, OsStr},
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
};

use compatdb_core::{
    EnvEntry, Origin, Resolution, Scalar,
    runtime::{
        CAPTURES_DIR, VULKAN_LAYER_DIR, capture_hook_value, env_value, metal_capture_env,
        vulkan_capture_env, vulkan_capture_file,
    },
};

use crate::{Process, apply, log, ntdll::Ntdll};

/// The variable `vulkan_capture` gives the name of the program to record,
/// which a process started by such a program inherits.
const PROCESS_NAME_VAR: &str = "GFXRECON_CAPTURE_PROCESS_NAME";

/// The variables of `metal_capture` that load Metal's capture layer and arm
/// KosmicKrisp in a process started by such a program, which inherits them.
const METAL_INHERITED_VARS: [&str; 2] = ["MTL_CAPTURE_ENABLED", "MESA_KK_GPU_CAPTURE"];

/// Apply the capture settings to the process and log them: for each set to
/// `true`, make the folder, fold its variables into `resolution`'s env
/// entries, which the caller applies with the rest, and set ntdll's capture
/// hook. On an error that setting captures nothing, and its line says why.
pub fn apply(nt: &Ntdll, process: &Process, resolution: &mut Resolution) {
    for (scalar, by) in &resolution.needs_program_ignored {
        log::line(&ignored_line(*scalar, by));
    }
    let inherited_var = |name: &str| inherited(process, name);
    let wanted =
        |scalar| resolution.scalar(scalar).value == Some(compatdb_core::ScalarValue::Bool(true));
    let (vulkan, metal) = (wanted(Scalar::VulkanCapture), wanted(Scalar::MetalCapture));
    let dir = if vulkan || metal {
        captures_folder(nt, process)
    } else {
        Err(String::new())
    };
    let mut armed = (false, false);
    for (scalar, on) in [
        (Scalar::VulkanCapture, vulkan),
        (Scalar::MetalCapture, metal),
    ] {
        let source = crate::scalar_source(resolution, scalar);
        if !on {
            let value = resolution
                .scalar(scalar)
                .value
                .as_ref()
                .map(|v| format!("{v}, "))
                .unwrap_or_default();
            let note = if scalar == Scalar::VulkanCapture {
                inherited_note(&[PROCESS_NAME_VAR], inherited_var, "the capture layer")
            } else {
                inherited_note(
                    &METAL_INHERITED_VARS,
                    inherited_var,
                    "Metal's capture layer",
                )
            };
            log::line(&format!("  {scalar} = {value}{source}{note}"));
            continue;
        }
        let entries = dir
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|dir| match scalar {
                Scalar::VulkanCapture => vulkan_entries(dir, process, resolution),
                _ => metal_capture_env(&path_str(dir)?, &process.exe, std::process::id())
                    .map_err(|e| e.to_string()),
            });
        let entries = entries.and_then(|e| dir.as_ref().map(|d| (e, d)).map_err(Clone::clone));
        match entries {
            Ok((entries, dir)) => {
                resolution.fold_capture_env(scalar, entries);
                if scalar == Scalar::VulkanCapture {
                    armed.0 = true;
                } else {
                    armed.1 = true;
                }
                log::line(&format!(
                    "  {scalar} = true, {source}: Ctrl+Shift+P records into {}",
                    dir.display()
                ));
            }
            Err(why) => {
                log::line(&format!(
                    "  {scalar} = true, {source}: error: {why}; no capture"
                ));
            }
        }
    }
    if armed.0 || armed.1 {
        nt.set_compat_capture(capture_hook_value(armed.0, armed.1));
    }
}

/// The folder `captures` next to the executable, made and writable, or why
/// there is none.
fn captures_folder(nt: &Ntdll, process: &Process) -> Result<PathBuf, String> {
    if !nt.has_set_compat_capture() {
        return Err(String::from("ntdll lacks set_compat_capture"));
    }
    let exe = nt.unix_file_name(&process.image_path)?;
    let dir = captures_dir(Path::new(OsStr::from_bytes(&exe)))?;
    make_writable_dir(&dir)?;
    Ok(dir)
}

/// The variables of `vulkan_capture` for captures into `dir`, or why there
/// are none, such as a bundle without the layer. The layer lists take in
/// what `resolution`'s env entries set, else what the process inherited.
fn vulkan_entries(
    dir: &Path,
    process: &Process,
    resolution: &Resolution,
) -> Result<Vec<EnvEntry>, String> {
    let layer_dir = apply::lib_wine_dir()
        .ok_or("cannot locate lib/wine")?
        .join(VULKAN_LAYER_DIR);
    let layer_dir = layer_dir
        .canonicalize()
        .map_err(|e| format!("no capture layer in {}: {e}", layer_dir.display()))?;
    let file = vulkan_capture_file(&path_str(dir)?, &process.exe, std::process::id());
    let current = |name: &str| {
        resolution
            .env_value(name)
            .map_or_else(|| inherited(process, name), |value| value.map(String::from))
    };
    vulkan_capture_env(
        &path_str(&layer_dir)?,
        &file,
        &program_name().unwrap_or_else(|| process.exe.clone()),
        current,
    )
    .map_err(|e| e.to_string())
}

/// The value of `name` the process was started with: from its environment
/// block, else from the unix environment.
fn inherited(process: &Process, name: &str) -> Option<String> {
    env_value(&process.env, name).or_else(|| std::env::var(name).ok())
}

/// The name of the process for `getprogname()`, which Wine sets to the
/// executable's file name as the path it was started by spells it, and
/// which the capture layer
/// compares `GFXRECON_CAPTURE_PROCESS_NAME` with.
fn program_name() -> Option<String> {
    // SAFETY: getprogname takes no arguments and returns libc's program
    // name, a NUL-terminated string or NULL.
    let name = unsafe { libc::getprogname() };
    if name.is_null() {
        return None;
    }
    // SAFETY: a non-NULL result is a NUL-terminated string libc keeps.
    let name = unsafe { std::ffi::CStr::from_ptr(name) };
    Some(name.to_string_lossy().into_owned())
}

/// The folder `captures` next to the executable at `exe`.
fn captures_dir(exe: &Path) -> Result<PathBuf, String> {
    exe.parent()
        .map(|parent| parent.join(CAPTURES_DIR))
        .ok_or_else(|| format!("{} has no folder", exe.display()))
}

/// Make `dir` unless it exists, and check that this process can write files
/// into it.
fn make_writable_dir(dir: &Path) -> Result<(), String> {
    match std::fs::create_dir(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && dir.is_dir() => {}
        Err(e) => return Err(format!("cannot make {}: {e}", dir.display())),
    }
    let c_dir = CString::new(dir.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: `c_dir` is a valid NUL-terminated path for the call.
    if unsafe { libc::access(c_dir.as_ptr(), libc::W_OK | libc::X_OK) } != 0 {
        return Err(format!(
            "cannot write into {}: {}",
            dir.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// `path` as text, which the variables need.
fn path_str(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(String::from)
        .ok_or_else(|| format!("{} is not UTF-8", path.display()))
}

/// What the line of a process without the setting says about its capture
/// variables `vars` that it inherited (`inherited`) from the process that
/// started it: `layer` loads but records nothing.
fn inherited_note(
    vars: &[&str],
    inherited: impl Fn(&str) -> Option<String>,
    layer: &str,
) -> String {
    let found: Vec<String> = vars
        .iter()
        .filter_map(|var| inherited(var).map(|value| format!("{var}={value}")))
        .collect();
    if found.is_empty() {
        String::new()
    } else {
        format!(
            "; {} inherited: {layer} loads and records nothing here",
            found.join(", ")
        )
    }
}

/// The log line of a setting that only a rule naming one program may hold,
/// which the rule `by` for `*` or a Steam app id set.
fn ignored_line(scalar: Scalar, by: &Origin) -> String {
    format!(
        "  {scalar}, {}: ignored, only a rule that names one program may set it",
        by.label()
    )
}

#[cfg(test)]
mod tests;
