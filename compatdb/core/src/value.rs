//! The values of the settings that choose between implementations: the
//! Direct3D trees, the wined3d renderer and the Mac driver's OpenGL.

use std::{fmt, str::FromStr};

use crate::{Arch, FieldError, Key};

/// Which implementation of the DXGI family a process loads.
///
/// D3D10, D3D10.1, D3D11 and D3D12 travel together because they all create
/// their device through one `dxgi.dll`, and each implementation's modules only
/// work with its own.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Dxgi {
    /// DXMT, which a built-in rule gives every program.
    Dxmt,
    /// Wine's own wined3d, what upstream Wine loads, and so what a process
    /// gets when no rule names an implementation.
    Wined3d,
}

impl Dxgi {
    /// Every implementation.
    pub const ALL: &'static [Self] = &[Self::Dxmt, Self::Wined3d];

    /// The wire spelling, which is also the `lib/wine/dxgi/<name>` tree.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dxmt => "dxmt",
            Self::Wined3d => "wined3d",
        }
    }

    /// What a process gets when no rule names an implementation: wined3d,
    /// as in upstream Wine, for every arch. Its tree has no D3D12. Either
    /// tree serves every arch: the 64-bit modules of both are ARM64X, so a
    /// native ARM64 program loads their ARM64 half and an x64 program, which
    /// runs as ARM64EC, their ARM64EC half; an i386 program gets their i386
    /// builds under emulation.
    #[must_use]
    pub const fn default_for(_arch: Arch) -> Self {
        Self::Wined3d
    }

    /// The implementation a process loads: the requested one, or the default
    /// for the arch when nothing was requested.
    #[must_use]
    pub const fn effective(requested: Option<Self>, arch: Arch) -> Self {
        match requested {
            Some(want) => want,
            None => Self::default_for(arch),
        }
    }
}

/// Which D3D9 implementation a process loads. Independent of [`Dxgi`]: neither
/// implementation's `d3d9.dll` touches `dxgi.dll`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum D3d9 {
    /// mtld3d, which a built-in rule gives every program.
    Mtld3d,
    /// Wine's own wined3d, what upstream Wine loads, and so what a process
    /// gets when no rule names an implementation.
    Wined3d,
}

impl D3d9 {
    /// Every implementation.
    pub const ALL: &'static [Self] = &[Self::Mtld3d, Self::Wined3d];

    /// The wire spelling, which is also the `lib/wine/d3d9/<name>` tree.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mtld3d => "mtld3d",
            Self::Wined3d => "wined3d",
        }
    }

    /// What a process gets when no rule names an implementation: wined3d,
    /// as in upstream Wine, for every arch. Either tree serves every arch:
    /// the 64-bit modules of both, mtld3d's release build and Wine's own
    /// `d3d9.dll`, are ARM64X, so a native ARM64 program loads their ARM64
    /// half and an x64 program, which runs as ARM64EC, their ARM64EC half;
    /// an i386 program gets their i386 builds under emulation.
    #[must_use]
    pub const fn default_for(_arch: Arch) -> Self {
        Self::Wined3d
    }

    /// The implementation a process loads: the requested one, or the default
    /// for the arch when nothing was requested.
    #[must_use]
    pub const fn effective(requested: Option<Self>, arch: Arch) -> Self {
        match requested {
            Some(want) => want,
            None => Self::default_for(arch),
        }
    }
}

/// Which renderer wined3d, Wine's own Direct3D, draws with in a process.
///
/// One `wined3d.dll` serves every wined3d user in a process (the wined3d trees
/// of [`Dxgi`] and [`D3d9`], D3D8 and `DirectDraw`), and it reads its settings
/// once, so this is a setting of the process, not of a family. It chooses no
/// tree: [`Dxgi`] and [`D3d9`] still decide whether wined3d is used at all.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Wined3d {
    /// OpenGL, wined3d's own default.
    Gl,
    /// Vulkan, on the bundle's `KosmicKrisp`.
    Vulkan,
}

impl Wined3d {
    /// The environment variable wined3d reads its settings from, a list of
    /// `name=value` options.
    pub const CONFIG_VAR: &'static str = "WINE_D3D_CONFIG";

    /// Every renderer.
    pub const ALL: &'static [Self] = &[Self::Gl, Self::Vulkan];

    /// The wire spelling, which is also the value of wined3d's `renderer`
    /// option.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gl => "gl",
            Self::Vulkan => "vulkan",
        }
    }

    /// The `WINE_D3D_CONFIG` value for this renderer on top of `config`, the
    /// value the rules' own `env` entries leave the variable at (`None` when
    /// they do not set it, or remove it). The renderer comes first and the
    /// other options of `config` are kept, while a `renderer` option there is
    /// dropped: this setting decides the renderer. OpenGL is set as
    /// explicitly as Vulkan, so a process that asks for it does not inherit
    /// Vulkan from the process that started it.
    #[must_use]
    pub fn config(self, config: Option<&str>) -> String {
        let mut options = vec![format!("renderer={}", self.as_str())];
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
    pub fn renderer_in(config: &str) -> Option<&str> {
        config
            .split([',', ';'])
            .find_map(|o| o.strip_prefix("renderer="))
    }
}

/// Which OpenGL the Mac driver gives a process.
///
/// The value is the process's own: ntdll stores it for winemac.drv, which
/// reads it when OpenGL is first initialized, so a program does not take the
/// choice of the program that started it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OpenGl {
    /// Mesa's EGL, with Zink on `KosmicKrisp`. Without a GPU `KosmicKrisp`
    /// can use, the process gets no OpenGL.
    Egl,
    /// Apple's OpenGL, upstream Wine's.
    Cgl,
}

impl OpenGl {
    /// Every OpenGL.
    pub const ALL: &'static [Self] = &[Self::Egl, Self::Cgl];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Egl => "egl",
            Self::Cgl => "cgl",
        }
    }

    /// The value ntdll's `set_compat_opengl` takes: 1 for Mesa's EGL, 0 for
    /// Apple's OpenGL.
    #[must_use]
    pub const fn hook_value(self) -> i32 {
        match self {
            Self::Egl => 1,
            Self::Cgl => 0,
        }
    }
}

/// The value of `all` that `name` spells `s`, ASCII-case-insensitively, or
/// the error for the setting `key`, whose spellings without `default` are
/// the ones it lists.
fn parse_word<T: Copy>(
    key: Key,
    all: &[T],
    name: fn(T) -> &'static str,
    s: &str,
) -> Result<T, FieldError> {
    all.iter()
        .copied()
        .find(|value| name(*value).eq_ignore_ascii_case(s))
        .ok_or_else(|| FieldError::BadValue {
            key,
            value: s.to_owned(),
            expected: key
                .values()
                .split_last()
                .map_or(&[], |(_default, words)| words),
        })
}

impl FromStr for Dxgi {
    type Err = FieldError;

    /// The implementation spelled `s`, ASCII-case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_word(Key::Dxgi, Self::ALL, Self::as_str, s)
    }
}

impl FromStr for D3d9 {
    type Err = FieldError;

    /// The implementation spelled `s`, ASCII-case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_word(Key::D3d9, Self::ALL, Self::as_str, s)
    }
}

impl FromStr for Wined3d {
    type Err = FieldError;

    /// The renderer spelled `s`, ASCII-case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_word(Key::Wined3d, Self::ALL, Self::as_str, s)
    }
}

impl FromStr for OpenGl {
    type Err = FieldError;

    /// The OpenGL spelled `s`, ASCII-case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_word(Key::OpenGl, Self::ALL, Self::as_str, s)
    }
}

impl fmt::Display for Dxgi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for D3d9 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Wined3d {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for OpenGl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests;
