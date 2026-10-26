//! The typed parts of a rule: its `exe` and fingerprint values, and the
//! entries of its lists. Each part checks its text when it is made, so every
//! rule that can be built can also be written to [`ENV_VAR`] and read back.
//!
//! No part holds a NUL: the process APIs that take these texts end a string
//! at the first one.

use std::{fmt, str::FromStr};

use crate::{DEFAULT_VALUE, FieldError};
#[cfg(doc)]
use crate::{ENV_VAR, Rule};

/// `text` itself, or [`FieldError::Nul`] when it holds a NUL.
fn without_nul(text: String) -> Result<String, FieldError> {
    if text.contains('\0') {
        Err(FieldError::Nul)
    } else {
        Ok(text)
    }
}

/// The executable a rule matches: every process, or the processes whose image
/// basename is a name, compared ASCII-case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Exe {
    /// `*`: every process.
    Any,
    /// The processes with this image basename.
    Name(ExeName),
}

/// The image basename of an [`Exe::Name`]: not empty, not `*` and without a
/// NUL. [`Exe::new`] makes one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExeName(String);

impl Exe {
    /// The spelling of [`Exe::Any`].
    pub const ANY: &'static str = "*";

    /// The `exe` that `text` names: `*` is [`Exe::Any`], anything else the
    /// basename as written.
    ///
    /// # Errors
    ///
    /// [`FieldError::MissingExe`] for an empty `text`, [`FieldError::Nul`]
    /// for one with a NUL.
    pub fn new(text: impl Into<String>) -> Result<Self, FieldError> {
        let text = without_nul(text.into())?;
        match text.as_str() {
            "" => Err(FieldError::MissingExe),
            Self::ANY => Ok(Self::Any),
            _ => Ok(Self::Name(ExeName(text))),
        }
    }

    /// `text` as an `exe`, unchecked: only for the literals of this crate,
    /// which a test reads back.
    pub(crate) fn literal(text: &str) -> Self {
        if text == Self::ANY {
            Self::Any
        } else {
            Self::Name(ExeName(text.to_owned()))
        }
    }

    /// `*` or the basename, as the wire writes it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Any => Self::ANY,
            Self::Name(name) => name.as_str(),
        }
    }

    /// Whether this is [`Exe::Any`].
    #[must_use]
    pub const fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    /// Whether a process with the image basename `basename` matches.
    #[must_use]
    pub fn matches(&self, basename: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Name(name) => basename.eq_ignore_ascii_case(name.as_str()),
        }
    }
}

impl ExeName {
    /// The basename as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Exe {
    type Err = FieldError;

    /// See [`Exe::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for Exe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for ExeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for Exe {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// How specific a rule's matchers are, least specific first, which is the
/// order [`Table::resolve`](crate::Table::resolve) folds the rules of a layer
/// in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Specificity {
    /// `exe` `*` and no fingerprint: a default for every process.
    Any,
    /// `exe` `*` with a fingerprint.
    AnyWithFingerprint,
    /// An executable name without a fingerprint.
    Exe,
    /// An executable name with a fingerprint.
    ExeWithFingerprint,
}

/// A fingerprint value: a substring that one string of the image's version
/// resource must contain, compared ASCII-case-insensitively. Any text
/// without a NUL; an empty one matches every image.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// `text` as a fingerprint value.
    ///
    /// # Errors
    ///
    /// [`FieldError::Nul`] when `text` holds a NUL.
    pub fn new(text: impl Into<String>) -> Result<Self, FieldError> {
        without_nul(text.into()).map(Self)
    }

    /// `text` as a fingerprint value, unchecked: only for the literals of
    /// this crate, which a test reads back.
    pub(crate) fn literal(text: &str) -> Self {
        Self(text.to_owned())
    }

    /// The value as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the version string `actual` contains this value, compared
    /// ASCII-case-insensitively.
    #[must_use]
    pub fn matches(&self, actual: &str) -> bool {
        actual
            .to_ascii_lowercase()
            .contains(&self.0.to_ascii_lowercase())
    }
}

/// Text appended to the command line: not empty and without a NUL.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Argument(String);

impl Argument {
    /// `text` as an argument.
    ///
    /// # Errors
    ///
    /// [`FieldError::EmptyArgument`] for an empty `text`,
    /// [`FieldError::Nul`] for one with a NUL.
    pub fn new(text: impl Into<String>) -> Result<Self, FieldError> {
        let text = without_nul(text.into())?;
        if text.is_empty() {
            Err(FieldError::EmptyArgument)
        } else {
            Ok(Self(text))
        }
    }

    /// `text` as an argument, unchecked: only for the literals of this
    /// crate, which a test reads back.
    pub(crate) fn literal(text: &str) -> Self {
        Self(text.to_owned())
    }

    /// The text as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One `WINEDLLOVERRIDES` element, `<modules>=<order>`.
///
/// That is comma-separated module names, `=`, and a load order such as
/// `n,b`, empty to disable the modules, or `default` to take them out of the
/// overrides of the rules folded before. The text is kept as written.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DllOverride(String);

impl DllOverride {
    /// `text` as a DLL override.
    ///
    /// # Errors
    ///
    /// [`FieldError::InvalidDllOverride`] when `text` has no `=`,
    /// [`FieldError::Nul`] when it holds a NUL.
    pub fn new(text: impl Into<String>) -> Result<Self, FieldError> {
        let text = without_nul(text.into())?;
        if text.contains('=') {
            Ok(Self(text))
        } else {
            Err(FieldError::InvalidDllOverride(text))
        }
    }

    /// `text` as a DLL override, unchecked: only for the literals of this
    /// crate, which a test reads back, and for overrides made from the parts
    /// of a checked one.
    pub(crate) fn literal(text: &str) -> Self {
        Self(text.to_owned())
    }

    /// The text as written, which is also its form in `WINEDLLOVERRIDES`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The module names before the `=`, trimmed, without empty ones.
    pub fn modules(&self) -> impl Iterator<Item = &str> {
        self.names()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    /// The load order after the first `=`, as written.
    #[must_use]
    pub fn order(&self) -> &str {
        self.0.split_once('=').map_or("", |(_, order)| order)
    }

    /// Whether the order is `default`, in any case and with surrounding
    /// spaces: such an entry sets no load order, it takes its modules out of
    /// the entries before it.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.order().trim().eq_ignore_ascii_case(DEFAULT_VALUE)
    }

    /// The text before the first `=`.
    fn names(&self) -> &str {
        self.0.split_once('=').map_or("", |(names, _)| names)
    }
}

/// The name of an environment variable: not empty, without `=` and without a
/// NUL. It is kept as written; a process environment matches it
/// ASCII-case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnvName(String);

impl EnvName {
    /// `text` as a variable name.
    ///
    /// # Errors
    ///
    /// [`FieldError::InvalidEnvName`] when `text` is empty or holds a `=`,
    /// [`FieldError::Nul`] when it holds a NUL.
    pub fn new(text: impl Into<String>) -> Result<Self, FieldError> {
        let text = without_nul(text.into())?;
        if text.is_empty() || text.contains('=') {
            Err(FieldError::InvalidEnvName(text))
        } else {
            Ok(Self(text))
        }
    }

    /// The name as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One environment entry of a rule: a variable and the value it gets, or no
/// value, which removes the variable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnvEntry {
    /// The variable.
    name: EnvName,
    /// Its value, not empty, or `None` to remove it.
    value: Option<String>,
}

impl EnvEntry {
    /// The entry that sets `name` to `value`. An empty `value` removes the
    /// variable, as on the wire, where an empty value is a removal.
    ///
    /// # Errors
    ///
    /// [`FieldError::Nul`] when `value` holds a NUL.
    pub fn new(name: EnvName, value: impl Into<String>) -> Result<Self, FieldError> {
        let value = without_nul(value.into())?;
        Ok(Self {
            name,
            value: (!value.is_empty()).then_some(value),
        })
    }

    /// The entry that removes `name`.
    #[must_use]
    pub const fn remove(name: EnvName) -> Self {
        Self { name, value: None }
    }

    /// The entry that sets `name` to `value`, unchecked: only for the
    /// literals of this crate, which a test reads back, and for values made
    /// from checked ones.
    pub(crate) fn literal(name: &str, value: &str) -> Self {
        Self {
            name: EnvName(name.to_owned()),
            value: (!value.is_empty()).then(|| value.to_owned()),
        }
    }

    /// The variable.
    #[must_use]
    pub const fn name(&self) -> &EnvName {
        &self.name
    }

    /// The value it gets, or `None` when the entry removes it.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

impl fmt::Display for EnvEntry {
    /// `NAME=value`, or `NAME=` for a removal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.name, self.value().unwrap_or_default())
    }
}

impl FromStr for Fingerprint {
    type Err = FieldError;

    /// See [`Fingerprint::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl FromStr for Argument {
    type Err = FieldError;

    /// See [`Argument::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl FromStr for DllOverride {
    type Err = FieldError;

    /// See [`DllOverride::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl FromStr for EnvName {
    type Err = FieldError;

    /// See [`EnvName::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Argument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for DllOverride {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for EnvName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for Argument {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for Fingerprint {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<&str> for Argument {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<&str> for DllOverride {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<&str> for EnvName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests;
