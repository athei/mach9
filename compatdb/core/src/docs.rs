//! Links into this crate's published rustdoc, for a launcher's templates and
//! messages. Every link starts with [`DOCS_URL`](crate::DOCS_URL).

use crate::Key;

/// A section of the crate's front page, the spec of the format and the rules.
///
/// Each variant is one heading of that page; [`DocSection::url`] links to it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DocSection {
    /// Quick start: how a launcher builds its rules and starts Wine with them.
    QuickStart,
    /// Format: the text of `WINE_COMPATDB`.
    Format,
    /// Strict and lenient parsing: how the text is read, a part of Format.
    StrictAndLenientParsing,
    /// Matching: which processes a rule applies to.
    Matching,
    /// Folding: how the rules that match a process combine.
    Folding,
    /// Keys: the fields of a rule.
    Keys,
    /// Built-in rules: the rules `compatdb.so` has compiled in.
    BuiltInRules,
    /// Recipes: one rule for each common goal.
    Recipes,
    /// Launcher responsibilities: what a launcher does beyond the rules.
    LauncherResponsibilities,
}

/// The link to the heading whose rustdoc id is `$anchor`.
macro_rules! section {
    ($anchor:literal) => {
        concat!(docs_url!(), "#", $anchor)
    };
}

/// The link to the doc of the variant `$variant` of [`Key`].
macro_rules! variant {
    ($variant:ident) => {
        concat!(docs_url!(), "enum.Key.html#variant.", stringify!($variant))
    };
}

impl DocSection {
    /// Every section, in the order of the page.
    pub const ALL: &'static [Self] = &[
        Self::QuickStart,
        Self::Format,
        Self::StrictAndLenientParsing,
        Self::Matching,
        Self::Folding,
        Self::Keys,
        Self::BuiltInRules,
        Self::Recipes,
        Self::LauncherResponsibilities,
    ];

    /// The full URL of the section's heading.
    #[must_use]
    pub const fn url(self) -> &'static str {
        match self {
            Self::QuickStart => section!("quick-start"),
            Self::Format => section!("format"),
            Self::StrictAndLenientParsing => section!("strict-and-lenient-parsing"),
            Self::Matching => section!("matching"),
            Self::Folding => section!("folding"),
            Self::Keys => section!("keys"),
            Self::BuiltInRules => section!("built-in-rules"),
            Self::Recipes => section!("recipes"),
            Self::LauncherResponsibilities => section!("launcher-responsibilities"),
        }
    }
}

impl Key {
    /// The full URL of this key's doc, the doc of its variant of [`Key`].
    #[must_use]
    pub const fn docs_url(self) -> &'static str {
        match self {
            Self::Exe => variant!(Exe),
            Self::Company => variant!(Company),
            Self::Product => variant!(Product),
            Self::OriginalFilename => variant!(OriginalFilename),
            Self::Dxgi => variant!(Dxgi),
            Self::D3d9 => variant!(D3d9),
            Self::Wined3d => variant!(Wined3d),
            Self::OpenGl => variant!(OpenGl),
            Self::DpiAware => variant!(DpiAware),
            Self::LargeAddressAware => variant!(LargeAddressAware),
            Self::Gcmouse => variant!(Gcmouse),
            Self::ForceNx => variant!(ForceNx),
            Self::MetalHud => variant!(MetalHud),
            Self::DllOverrides => variant!(DllOverrides),
            Self::ArgumentsOverride => variant!(ArgumentsOverride),
            Self::Arguments => variant!(Arguments),
            Self::DebugChannels => variant!(DebugChannels),
            Self::Env => variant!(Env),
        }
    }
}

#[cfg(test)]
mod tests;
