//! What `compatdb.so` needs to apply a [`Resolution`] inside a process.
//!
//! That is the process's arch, its image path, command line and environment
//! block as Wine hands them over in UTF-16, the reader of an image's version
//! resource, and the values the library hands to Wine.
//!
//! A launcher does not need this module. It builds rules, writes the table
//! into [`ENV_VAR`](crate::ENV_VAR) and, to show what a program would get,
//! uses [`Table::preview`](crate::Table::preview) with a [`VersionInfo`] it
//! fills in itself.
//!
//! [`Resolution`]: crate::Resolution
//! [`VersionInfo`]: crate::VersionInfo

mod process;
mod version;

pub use self::{
    process::{
        Arch, IMAGE_FILE_MACHINE_ARM64, append_cmdline, append_cmdline_each, basename_of,
        env_value, merge_env,
    },
    version::version_info,
};
use crate::{D3d9, Dxgi, OpenGl, Wined3d};

/// The DXGI-family implementation a process loads when no rule names one.
///
/// That is wined3d, as in upstream Wine, for every arch. Its tree has no
/// D3D12. Either tree serves every arch: the 64-bit modules of both are
/// ARM64X, so a native ARM64 program loads their ARM64 half and an x64
/// program, which runs as ARM64EC, their ARM64EC half; an i386 program gets
/// their i386 builds under emulation.
#[must_use]
pub const fn default_dxgi(_arch: Arch) -> Dxgi {
    Dxgi::Wined3d
}

/// The DXGI-family implementation a process loads: the requested one, or
/// [`default_dxgi`] for the arch when nothing was requested.
#[must_use]
pub const fn effective_dxgi(requested: Option<Dxgi>, arch: Arch) -> Dxgi {
    match requested {
        Some(want) => want,
        None => default_dxgi(arch),
    }
}

/// The D3D9 implementation a process loads when no rule names one.
///
/// That is wined3d, as in upstream Wine, for every arch. Either tree serves
/// every arch: the 64-bit modules of both, mtld3d's release build and Wine's
/// own `d3d9.dll`, are ARM64X, so a native ARM64 program loads their ARM64
/// half and an x64 program, which runs as ARM64EC, their ARM64EC half; an
/// i386 program gets their i386 builds under emulation.
#[must_use]
pub const fn default_d3d9(_arch: Arch) -> D3d9 {
    D3d9::Wined3d
}

/// The D3D9 implementation a process loads: the requested one, or
/// [`default_d3d9`] for the arch when nothing was requested.
#[must_use]
pub const fn effective_d3d9(requested: Option<D3d9>, arch: Arch) -> D3d9 {
    match requested {
        Some(want) => want,
        None => default_d3d9(arch),
    }
}

/// The value ntdll's `set_compat_opengl` takes for `opengl`: 1 for Mesa's
/// EGL, 0 for Apple's OpenGL.
#[must_use]
pub const fn opengl_hook_value(opengl: OpenGl) -> i32 {
    match opengl {
        OpenGl::Egl => 1,
        OpenGl::Cgl => 0,
    }
}

/// The `WINE_D3D_CONFIG` value for the renderer `wined3d` on top of
/// `config`.
///
/// `config` is the value the rules' own `env` entries leave the variable at
/// (`None` when they do not set it, or remove it). The renderer comes first
/// and the other options of `config` are kept, while a `renderer` option
/// there is dropped: the setting decides the renderer. OpenGL is set as
/// explicitly as Vulkan, so a process that asks for it does not inherit
/// Vulkan from the process that started it.
#[must_use]
pub fn wined3d_config(wined3d: Wined3d, config: Option<&str>) -> String {
    let mut options = vec![format!("renderer={}", wined3d.as_str())];
    options.extend(
        config
            .unwrap_or("")
            .split([',', ';'])
            .filter(|o| !o.is_empty())
            .filter(|o| o.split('=').next() != Some("renderer"))
            .map(String::from),
    );
    options.join(",")
}

/// The `renderer` option of a `WINE_D3D_CONFIG` value, the first one as
/// wined3d reads it, or `None` when the value has none.
#[must_use]
pub fn wined3d_renderer_in(config: &str) -> Option<&str> {
    config
        .split([',', ';'])
        .find_map(|o| o.strip_prefix("renderer="))
}

#[cfg(test)]
mod tests;
