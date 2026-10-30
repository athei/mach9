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
use crate::{Exe, Rule, Setting, Table, metal_hud_env};

/// The one-line summary of each key in plain text: the first paragraph of
/// the key's variant doc, and what [`Key::summary`] returns.
macro_rules! summary {
    (Exe) => {
        "The image basename a rule matches, in any case, * for every process, or steam:440 for \
         that Steam app."
    };
    (Company) => {
        "A text the company name of the image's version resource must contain."
    };
    (Product) => {
        "A text the product name of the image's version resource must contain."
    };
    (OriginalFilename) => {
        "A text the original file name of the image's version resource must contain."
    };
    (Dxgi) => {
        "The implementation of D3D10 to D3D12 the process loads."
    };
    (D3d9) => {
        "The D3D9 implementation the process loads."
    };
    (Wined3d) => {
        "The renderer wined3d draws with in the process."
    };
    (OpenGl) => {
        "Which OpenGL the Mac driver gives the process."
    };
    (DpiAware) => {
        "Whether the process is DPI-aware."
    };
    (LargeAddressAware) => {
        "Whether a 32-bit process gets the 4 GB address space whatever its executable's flag \
         says."
    };
    (Gcmouse) => {
        "Whether raw mouse movement comes from GameController instead of the cursor."
    };
    (ForceNx) => {
        "Whether no-execute stays on in a 32-bit process."
    };
    (MetalHud) => {
        "Whether the Metal performance HUD shows."
    };
    (DllOverrides) => {
        "DLL load-order overrides, one WINEDLLOVERRIDES element each."
    };
    (ArgumentsOverride) => {
        "Arguments that replace those the rules folded before give the command line."
    };
    (Arguments) => {
        "Text appended to the command line, one argument string each."
    };
    (DebugChannels) => {
        "WINEDEBUG channels for the process, one entry each."
    };
    (Env) => {
        "Environment variables the process gets; an empty value removes the variable."
    };
}

/// The key of a field of a rule, which is also the name of the field on the
/// wire.
///
/// Each variant says what the field does and what a process gets without
/// it; the getter of [`Rule`] it links to has the details. The text
/// `compatdb.so` logs for an unset setting is [`Key::upstream`].
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Key {
    #[doc = summary!(Exe)]
    ///
    /// `exe`, see [`Exe`]. Every rule has one.
    Exe,
    #[doc = summary!(Company)]
    ///
    /// `company`, a substring of the version string `CompanyName`, compared
    /// ASCII-case-insensitively (see [`Rule::company`]). Without it, any
    /// company matches.
    Company,
    #[doc = summary!(Product)]
    ///
    /// `product`, a substring of the version string `ProductName`, compared
    /// ASCII-case-insensitively (see [`Rule::product`]). Without it, any
    /// product matches.
    Product,
    #[doc = summary!(OriginalFilename)]
    ///
    /// `original_filename`, a substring of the version string
    /// `OriginalFilename`, compared ASCII-case-insensitively (see
    /// [`Rule::original_filename`]). Without it, any original filename
    /// matches.
    OriginalFilename,
    #[doc = summary!(Dxgi)]
    ///
    /// `dxgi`, a [`Dxgi`] (see [`Rule::dxgi`]). Unset: wined3d, as in
    /// upstream Wine.
    Dxgi,
    #[doc = summary!(D3d9)]
    ///
    /// `d3d9`, a [`D3d9`] (see [`Rule::d3d9`]). Unset: wined3d, as in
    /// upstream Wine.
    D3d9,
    #[doc = summary!(Wined3d)]
    ///
    /// `wined3d`, a [`Wined3d`] for the whole process (see
    /// [`Rule::wined3d`]). Unset: `WINE_D3D_CONFIG` or wined3d's registry
    /// settings decide, and without them OpenGL.
    Wined3d,
    #[doc = summary!(OpenGl)]
    ///
    /// `opengl`, an [`OpenGl`] (see [`Rule::opengl`]). Unset: Apple's
    /// OpenGL, as in upstream Wine.
    OpenGl,
    #[doc = summary!(DpiAware)]
    ///
    /// `dpi_aware` (see [`Rule::dpi_aware`]). Unset: the registry and the
    /// manifest decide, and without them the process is unaware.
    DpiAware,
    #[doc = summary!(LargeAddressAware)]
    ///
    /// `large_address_aware` (see [`Rule::large_address_aware`]). Unset:
    /// the flag decides.
    LargeAddressAware,
    #[doc = summary!(Gcmouse)]
    ///
    /// `gcmouse` (see [`Rule::gcmouse`]). Unset: from the cursor, as in
    /// upstream Wine.
    Gcmouse,
    #[doc = summary!(ForceNx)]
    ///
    /// `force_nx` (see [`Rule::force_nx`]). Unset: Wine turns it off for an
    /// image without `NX_COMPAT`.
    ForceNx,
    #[doc = summary!(MetalHud)]
    ///
    /// `metal_hud`, through the variables of [`metal_hud_env`] (see
    /// [`Rule::metal_hud`]). Unset: the process keeps the variables it was
    /// started with.
    MetalHud,
    #[doc = summary!(DllOverrides)]
    ///
    /// `dll_overrides`, repeatable (see [`Rule::dll_overrides`]). None:
    /// `WINEDLLOVERRIDES` and the registry decide.
    DllOverrides,
    #[doc = summary!(ArgumentsOverride)]
    ///
    /// `arguments_override`, repeatable (see [`Rule::arguments_override`]).
    /// None: the arguments of every rule append.
    ArgumentsOverride,
    #[doc = summary!(Arguments)]
    ///
    /// `arguments`, repeatable (see [`Rule::arguments`]). None: the command
    /// line stays as it was.
    Arguments,
    #[doc = summary!(DebugChannels)]
    ///
    /// `debug_channels`, repeatable (see [`Rule::debug_channels`]). Wine
    /// reads `WINEDEBUG` before `compatdb.so` loads, so the library only
    /// logs them; a launcher puts them into the variable (see
    /// [`Table::winedebug`]). None: the `WINEDEBUG` the process was started
    /// with.
    DebugChannels,
    #[doc = summary!(Env)]
    ///
    /// `env`, one `NAME=value` entry each, repeatable (see [`Rule::env`]).
    /// None: the environment the process was started with.
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

    /// What the field does, in one line of plain text without markup, such
    /// as a launcher's template puts above the setting. It is the first
    /// paragraph of the key's doc, which [`Key::docs_url`] links to.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Exe => summary!(Exe),
            Self::Company => summary!(Company),
            Self::Product => summary!(Product),
            Self::OriginalFilename => summary!(OriginalFilename),
            Self::Dxgi => summary!(Dxgi),
            Self::D3d9 => summary!(D3d9),
            Self::Wined3d => summary!(Wined3d),
            Self::OpenGl => summary!(OpenGl),
            Self::DpiAware => summary!(DpiAware),
            Self::LargeAddressAware => summary!(LargeAddressAware),
            Self::Gcmouse => summary!(Gcmouse),
            Self::ForceNx => summary!(ForceNx),
            Self::MetalHud => summary!(MetalHud),
            Self::DllOverrides => summary!(DllOverrides),
            Self::ArgumentsOverride => summary!(ArgumentsOverride),
            Self::Arguments => summary!(Arguments),
            Self::DebugChannels => summary!(DebugChannels),
            Self::Env => summary!(Env),
        }
    }

    /// A value a launcher's template can show for the field, as
    /// [`Rule::set`] takes it: a matcher's value, one of [`Key::values`]
    /// other than `default` for a scalar setting, one entry for a list, and
    /// `NAME=value` for `env`. A launcher that writes `env` as a table of
    /// names splits it at the first `=`.
    ///
    /// The `exe` is a program's, not `*`, since `arguments_override` needs
    /// one; every example is a value a rule for that program takes.
    #[must_use]
    pub const fn example(self) -> &'static str {
        match self {
            Self::Exe | Self::OriginalFilename => "Game.exe",
            Self::Company => "Some Vendor",
            Self::Product => "Some Game",
            Self::Dxgi | Self::D3d9 => "wined3d",
            Self::Wined3d => "vulkan",
            Self::OpenGl => "cgl",
            Self::DpiAware | Self::Gcmouse | Self::ForceNx => "false",
            Self::LargeAddressAware | Self::MetalHud => "true",
            Self::DllOverrides => "d3dcompiler_47=n",
            Self::ArgumentsOverride | Self::Arguments => "-windowed",
            Self::DebugChannels => "+relay",
            Self::Env => "DXMT_CONFIG=d3d11.preferredMaxFrameRate=60",
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

/// A setting that holds one value (see [`KeyKind::Scalar`]), or is reset
/// with `default` (see [`Setting::Reset`]).
///
/// Each variant is the setting of its [`Key`], whose doc says what it does
/// and what a process gets without it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Scalar {
    /// [`Key::Dxgi`]: the implementation of D3D10 to D3D12. Unset:
    /// wined3d.
    Dxgi,
    /// [`Key::D3d9`]: the D3D9 implementation. Unset: wined3d.
    D3d9,
    /// [`Key::Wined3d`]: the renderer of wined3d. Unset: what
    /// `WINE_D3D_CONFIG` or the registry says, otherwise OpenGL.
    Wined3d,
    /// [`Key::OpenGl`]: the Mac driver's OpenGL. Unset: Apple's.
    OpenGl,
    /// [`Key::DpiAware`]: DPI awareness. Unset: the registry and the
    /// manifest decide.
    DpiAware,
    /// [`Key::LargeAddressAware`]: 4 GB for a 32-bit process. Unset: the
    /// executable's flag decides.
    LargeAddressAware,
    /// [`Key::Gcmouse`]: raw mouse movement from GameController. Unset:
    /// from the cursor.
    Gcmouse,
    /// [`Key::ForceNx`]: no-execute kept on in a 32-bit process. Unset:
    /// Wine's behaviour.
    ForceNx,
    /// [`Key::MetalHud`]: the Metal performance HUD. Unset: the inherited
    /// variables decide.
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
