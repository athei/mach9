//! The keys of a rule's fields and the values of its scalar settings.
//!
//! The case rule: every word the format itself defines (a key, a value of
//! `dxgi`, `d3d9`, `wined3d` or `opengl`, `true`, `false` and `default`) is
//! matched ASCII-case-insensitively and always written in lowercase. User
//! data (the `exe`, the fingerprint values, env names and values, arguments
//! and the text of a DLL override) is kept byte for byte.

use std::{fmt, str::FromStr};

use crate::{D3d9, DEFAULT_VALUE, Dxgi, FieldError, OpenGl, Wined3d};
#[cfg(doc)]
use crate::{Setting, Table, metal_hud_env};

/// The key of a field of a rule, which is also the name of the field on the
/// wire.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Key {
    /// `exe`: the image basename a rule matches, or `*` for every process.
    Exe,
    /// `company`: a substring of the image's version `CompanyName`.
    Company,
    /// `product`: a substring of the image's version `ProductName`.
    Product,
    /// `original_filename`: a substring of the image's version
    /// `OriginalFilename`.
    OriginalFilename,
    /// `dxgi`: the DXGI-family implementation, a [`Dxgi`].
    Dxgi,
    /// `d3d9`: the D3D9 implementation, a [`D3d9`].
    D3d9,
    /// `wined3d`: the renderer of wined3d, a [`Wined3d`].
    Wined3d,
    /// `opengl`: which OpenGL the Mac driver gives, an [`OpenGl`].
    OpenGl,
    /// `dpi_aware`: whether the process is DPI-aware.
    DpiAware,
    /// `large_address_aware`: whether a 32-bit process gets 4 GB.
    LargeAddressAware,
    /// `gcmouse`: whether mouse movement comes from GameController.
    Gcmouse,
    /// `force_nx`: whether no-execute stays on in a 32-bit process.
    ForceNx,
    /// `metal_hud`: whether the Metal performance HUD shows, through the
    /// variables of [`metal_hud_env`].
    MetalHud,
    /// `dll_overrides`: load-order overrides, repeatable.
    DllOverrides,
    /// `arguments_override`: the arguments that replace those of the rules
    /// folded before, repeatable.
    ArgumentsOverride,
    /// `arguments`: text appended to the command line, repeatable.
    Arguments,
    /// `debug_channels`: `WINEDEBUG` entries, repeatable. Wine reads
    /// `WINEDEBUG` before `compatdb.so` loads, so the library only logs
    /// them; a launcher puts them into the variable (see
    /// [`Table::winedebug`]).
    DebugChannels,
    /// `env`: an environment entry, repeatable.
    Env,
}

/// What kind of field a [`Key`] names.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyKind {
    /// A matcher: which processes the rule applies to.
    Matcher,
    /// A setting that holds one value of the given kind, or `default`.
    Scalar(ValueKind),
    /// A setting that holds a list of texts, one per field.
    List,
    /// The environment entries, `NAME=value` each.
    Env,
}

/// The values a scalar setting takes.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ValueKind {
    /// One word of a fixed set, which [`Key::values`] lists.
    Word,
    /// `true` or `false`.
    Bool,
}

impl Key {
    /// Every key, in the order a record writes its fields.
    pub const ALL: &'static [Self] = &[
        Self::Exe,
        Self::Company,
        Self::Product,
        Self::OriginalFilename,
        Self::Dxgi,
        Self::D3d9,
        Self::Wined3d,
        Self::OpenGl,
        Self::DpiAware,
        Self::LargeAddressAware,
        Self::Gcmouse,
        Self::ForceNx,
        Self::MetalHud,
        Self::DllOverrides,
        Self::ArgumentsOverride,
        Self::Arguments,
        Self::DebugChannels,
        Self::Env,
    ];

    /// The key as the wire writes it, in lowercase.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exe => "exe",
            Self::Company => "company",
            Self::Product => "product",
            Self::OriginalFilename => "original_filename",
            Self::Dxgi => "dxgi",
            Self::D3d9 => "d3d9",
            Self::Wined3d => "wined3d",
            Self::OpenGl => "opengl",
            Self::DpiAware => "dpi_aware",
            Self::LargeAddressAware => "large_address_aware",
            Self::Gcmouse => "gcmouse",
            Self::ForceNx => "force_nx",
            Self::MetalHud => "metal_hud",
            Self::DllOverrides => "dll_overrides",
            Self::ArgumentsOverride => "arguments_override",
            Self::Arguments => "arguments",
            Self::DebugChannels => "debug_channels",
            Self::Env => "env",
        }
    }

    /// The scalar setting this key names, or `None` for a matcher or a
    /// list.
    #[must_use]
    pub const fn scalar(self) -> Option<Scalar> {
        match self {
            Self::Dxgi => Some(Scalar::Dxgi),
            Self::D3d9 => Some(Scalar::D3d9),
            Self::Wined3d => Some(Scalar::Wined3d),
            Self::OpenGl => Some(Scalar::OpenGl),
            Self::DpiAware => Some(Scalar::DpiAware),
            Self::LargeAddressAware => Some(Scalar::LargeAddressAware),
            Self::Gcmouse => Some(Scalar::Gcmouse),
            Self::ForceNx => Some(Scalar::ForceNx),
            Self::MetalHud => Some(Scalar::MetalHud),
            Self::Exe
            | Self::Company
            | Self::Product
            | Self::OriginalFilename
            | Self::DllOverrides
            | Self::ArgumentsOverride
            | Self::Arguments
            | Self::DebugChannels
            | Self::Env => None,
        }
    }

    /// What kind of field this key names.
    #[must_use]
    pub const fn kind(self) -> KeyKind {
        if let Some(scalar) = self.scalar() {
            return KeyKind::Scalar(scalar.value_kind());
        }
        match self {
            Self::DllOverrides
            | Self::ArgumentsOverride
            | Self::Arguments
            | Self::DebugChannels => KeyKind::List,
            Self::Env => KeyKind::Env,
            _ => KeyKind::Matcher,
        }
    }

    /// How a launcher names the field to a person, for example
    /// `DPI-aware`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exe => "Executable",
            Self::Company => "Company",
            Self::Product => "Product",
            Self::OriginalFilename => "Original filename",
            Self::Dxgi => "DXGI",
            Self::D3d9 => "D3D9",
            Self::Wined3d => "wined3d renderer",
            Self::OpenGl => "OpenGL",
            Self::DpiAware => "DPI-aware",
            Self::LargeAddressAware => "Large-address-aware",
            Self::Gcmouse => "GameController mouse",
            Self::ForceNx => "Force no-execute",
            Self::MetalHud => "Metal HUD",
            Self::DllOverrides => "DLL overrides",
            Self::ArgumentsOverride => "Arguments override",
            Self::Arguments => "Arguments",
            Self::DebugChannels => "Debug channels",
            Self::Env => "Environment",
        }
    }

    /// What upstream Wine does for this setting when no rule sets it, as the
    /// log of `compatdb.so` prints it, or `None` for a matcher.
    /// `arguments_override` is a way of setting the arguments, so it shares
    /// their text.
    #[must_use]
    pub const fn upstream(self) -> Option<&'static str> {
        if let Some(scalar) = self.scalar() {
            return Some(scalar.upstream());
        }
        match self {
            Self::DllOverrides => Some("WINEDLLOVERRIDES and the registry's DllOverrides decide"),
            Self::ArgumentsOverride | Self::Arguments => {
                Some("the command line the process was started with")
            }
            Self::DebugChannels => Some("the WINEDEBUG the process was started with"),
            Self::Env => Some("the environment the process was started with"),
            _ => None,
        }
    }

    /// The spellings the value of a scalar setting can have, in lowercase,
    /// `default` included; empty for any other key, whose values are user
    /// data.
    #[must_use]
    pub const fn values(self) -> &'static [&'static str] {
        match self {
            Self::Dxgi => &["dxmt", "wined3d", DEFAULT_VALUE],
            Self::D3d9 => &["mtld3d", "wined3d", DEFAULT_VALUE],
            Self::Wined3d => &["gl", "vulkan", DEFAULT_VALUE],
            Self::OpenGl => &["egl", "cgl", DEFAULT_VALUE],
            Self::DpiAware
            | Self::LargeAddressAware
            | Self::Gcmouse
            | Self::ForceNx
            | Self::MetalHud => &["true", "false", DEFAULT_VALUE],
            Self::Exe
            | Self::Company
            | Self::Product
            | Self::OriginalFilename
            | Self::DllOverrides
            | Self::ArgumentsOverride
            | Self::Arguments
            | Self::DebugChannels
            | Self::Env => &[],
        }
    }
}

impl FromStr for Key {
    type Err = FieldError;

    /// The key spelled `s`, compared ASCII-case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|key| key.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| FieldError::UnknownKey(s.to_owned()))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A setting that holds one value (see [`KeyKind::Scalar`]).
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Scalar {
    /// [`Key::Dxgi`].
    Dxgi,
    /// [`Key::D3d9`].
    D3d9,
    /// [`Key::Wined3d`].
    Wined3d,
    /// [`Key::OpenGl`].
    OpenGl,
    /// [`Key::DpiAware`].
    DpiAware,
    /// [`Key::LargeAddressAware`].
    LargeAddressAware,
    /// [`Key::Gcmouse`].
    Gcmouse,
    /// [`Key::ForceNx`].
    ForceNx,
    /// [`Key::MetalHud`].
    MetalHud,
}

impl Scalar {
    /// Every scalar setting, in the order a record writes them.
    pub const ALL: &'static [Self] = &[
        Self::Dxgi,
        Self::D3d9,
        Self::Wined3d,
        Self::OpenGl,
        Self::DpiAware,
        Self::LargeAddressAware,
        Self::Gcmouse,
        Self::ForceNx,
        Self::MetalHud,
    ];

    /// The key of this setting.
    #[must_use]
    pub const fn key(self) -> Key {
        match self {
            Self::Dxgi => Key::Dxgi,
            Self::D3d9 => Key::D3d9,
            Self::Wined3d => Key::Wined3d,
            Self::OpenGl => Key::OpenGl,
            Self::DpiAware => Key::DpiAware,
            Self::LargeAddressAware => Key::LargeAddressAware,
            Self::Gcmouse => Key::Gcmouse,
            Self::ForceNx => Key::ForceNx,
            Self::MetalHud => Key::MetalHud,
        }
    }

    /// The key of this setting as the wire writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.key().as_str()
    }

    /// The kind of value this setting holds.
    #[must_use]
    pub const fn value_kind(self) -> ValueKind {
        match self {
            Self::Dxgi | Self::D3d9 | Self::Wined3d | Self::OpenGl => ValueKind::Word,
            Self::DpiAware
            | Self::LargeAddressAware
            | Self::Gcmouse
            | Self::ForceNx
            | Self::MetalHud => ValueKind::Bool,
        }
    }

    /// What upstream Wine does for this setting when no rule sets it (see
    /// [`Key::upstream`]).
    #[must_use]
    pub const fn upstream(self) -> &'static str {
        match self {
            Self::Dxgi | Self::D3d9 => "wined3d",
            Self::Wined3d => {
                "the OpenGL renderer, unless WINE_D3D_CONFIG or wined3d's registry settings name \
                 another"
            }
            Self::OpenGl => "Apple's OpenGL",
            Self::DpiAware => "the AppCompatFlags layer, then the manifest, otherwise unaware",
            Self::LargeAddressAware => "the executable's large-address-aware flag",
            Self::Gcmouse => "off, the cursor's movement",
            Self::ForceNx => "no-execute off when the executable or a DLL lacks the NX_COMPAT flag",
            Self::MetalHud => {
                "the MTL_HUD_ variables of the environment the process was started with"
            }
        }
    }

    /// Read `text` as a value of this setting, ASCII-case-insensitively.
    /// `default` is not a value: it resets the setting (see
    /// [`Setting::Reset`]).
    ///
    /// # Errors
    ///
    /// [`FieldError::BadValue`] when the setting does not take `text`. Its
    /// `expected` is [`Key::values`], `default` included, since it lists
    /// what a field of the key takes.
    pub fn parse_value(self, text: &str) -> Result<ScalarValue, FieldError> {
        let value = match self {
            Self::Dxgi => text.parse().ok().map(ScalarValue::Dxgi),
            Self::D3d9 => text.parse().ok().map(ScalarValue::D3d9),
            Self::Wined3d => text.parse().ok().map(ScalarValue::Wined3d),
            Self::OpenGl => text.parse().ok().map(ScalarValue::OpenGl),
            Self::DpiAware
            | Self::LargeAddressAware
            | Self::Gcmouse
            | Self::ForceNx
            | Self::MetalHud => parse_bool(text).map(ScalarValue::Bool),
        };
        value.ok_or_else(|| FieldError::BadValue {
            key: self.key(),
            value: text.to_owned(),
            expected: self.key().values(),
        })
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The value of a scalar setting.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ScalarValue {
    /// A value of [`Scalar::Dxgi`].
    Dxgi(Dxgi),
    /// A value of [`Scalar::D3d9`].
    D3d9(D3d9),
    /// A value of [`Scalar::Wined3d`].
    Wined3d(Wined3d),
    /// A value of [`Scalar::OpenGl`].
    OpenGl(OpenGl),
    /// A value of one of the true-or-false settings.
    Bool(bool),
}

impl ScalarValue {
    /// The value as the wire writes it, in lowercase.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dxgi(value) => value.as_str(),
            Self::D3d9(value) => value.as_str(),
            Self::Wined3d(value) => value.as_str(),
            Self::OpenGl(value) => value.as_str(),
            Self::Bool(true) => "true",
            Self::Bool(false) => "false",
        }
    }
}

impl fmt::Display for ScalarValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `true` or `false`, ASCII-case-insensitively.
const fn parse_bool(text: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
