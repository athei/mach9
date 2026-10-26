//! What can be wrong with one field of a rule, and with a line of
//! [`ENV_VAR`].

use std::fmt;

#[cfg(doc)]
use crate::Table;
use crate::{ENV_VAR, Key};

/// Why a field of a rule cannot be read, or a value cannot be set.
///
/// The [`Display`](fmt::Display) form is a short phrase, for example
/// `unknown key "bogus"`, which `compatdb.so` logs after `ignoring`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldError {
    /// A key the format does not know, as it was written.
    UnknownKey(String),
    /// A value that `key` does not take.
    ///
    /// From a field of a rule and from
    /// [`Scalar::parse_value`](crate::Scalar::parse_value), `expected` is
    /// [`Key::values`], `default` included. From the `FromStr` of a value
    /// type such as [`Dxgi`](crate::Dxgi), it lists that type's values
    /// alone, since `default` is none of them.
    BadValue {
        /// The key whose value it is.
        key: Key,
        /// The value as it was written, decoded.
        value: String,
        /// The spellings the value can have there, in lowercase.
        expected: &'static [&'static str],
    },
    /// A NUL in a value, which the process APIs that take it would end the
    /// text at.
    Nul,
    /// An `exe` that is empty, or a rule without one, which matches no
    /// process.
    MissingExe,
    /// An empty argument, which would append nothing.
    EmptyArgument,
    /// An empty `debug_channels` entry, which names no channel.
    EmptyDebugChannel,
    /// A `debug_channels` entry with a `,`, as it was written, which
    /// `WINEDEBUG` would read as more than one entry.
    CommaInDebugChannel(String),
    /// A variable name that is empty or holds a `=`, as it was written.
    InvalidEnvName(String),
    /// A DLL override without the `=` between its modules and its load
    /// order, as it was written.
    InvalidDllOverride(String),
    /// A `%` in a value that two hex digits do not follow.
    BadEscape,
    /// A value whose escapes decode to bytes that are not UTF-8.
    InvalidUtf8,
    /// A field without the `=` between its key and its value.
    MissingEquals,
    /// A second field for a key that holds one value: a matcher or a scalar
    /// setting.
    Duplicate(Key),
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(key) => write!(f, "unknown key {key:?}"),
            Self::BadValue { key, value, .. } => write!(f, "unknown {key} value {value:?}"),
            Self::Nul => f.write_str("a NUL in its value"),
            Self::MissingExe => f.write_str("no exe"),
            Self::EmptyArgument => f.write_str("an empty argument"),
            Self::EmptyDebugChannel => f.write_str("an empty debug channel"),
            Self::CommaInDebugChannel(text) => write!(f, "debug channel {text:?} with a ','"),
            Self::InvalidEnvName(name) => write!(f, "invalid variable name {name:?}"),
            Self::InvalidDllOverride(text) => {
                write!(f, "DLL override {text:?} without '=' before its load order")
            }
            Self::BadEscape => f.write_str("a '%' without two hex digits after it"),
            Self::InvalidUtf8 => f.write_str("escapes that are not UTF-8"),
            Self::MissingEquals => f.write_str("a field without '='"),
            Self::Duplicate(key) => write!(f, "{key} given more than once"),
        }
    }
}

impl std::error::Error for FieldError {}

/// One problem with the value of [`ENV_VAR`]: the line, the field as it was
/// written, if the problem is with one, and what is wrong.
///
/// Its [`Display`](fmt::Display) form is the warning `compatdb.so` logs, for
/// example `WINE_COMPATDB line 2: ignoring unknown key "bogus"`, which also
/// says what [`Table::parse_lenient`] did about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The line, counted from 1, blank lines and a legacy header included.
    pub line: usize,
    /// The field as it was written, or `None` for a problem with the whole
    /// line.
    pub field: Option<String>,
    /// What is wrong.
    pub error: FieldError,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = self.field.as_deref().unwrap_or_default();
        write!(f, "{ENV_VAR} line {}: ", self.line)?;
        match &self.error {
            FieldError::MissingExe => f.write_str("ignoring the line, it has no exe"),
            FieldError::MissingEquals => write!(f, "ignoring field {field:?} without '='"),
            FieldError::Duplicate(key) => {
                write!(f, "{key} given more than once, the last one applies")
            }
            FieldError::BadEscape => write!(
                f,
                "field {field:?} has {}, which is kept as written",
                self.error
            ),
            FieldError::Nul
            | FieldError::InvalidUtf8
            | FieldError::EmptyArgument
            | FieldError::EmptyDebugChannel => {
                write!(f, "ignoring field {field:?}: {}", self.error)
            }
            error => write!(f, "ignoring {error}"),
        }
    }
}

impl std::error::Error for Diagnostic {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Why [`Table::parse_strict`] refused a value of [`ENV_VAR`]: every
/// problem it found, in line order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub(crate) Vec<Diagnostic>);

impl ParseError {
    /// Every problem, in line order; never empty.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0
    }
}

impl fmt::Display for ParseError {
    /// The diagnostics, one per line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, diagnostic) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests;
