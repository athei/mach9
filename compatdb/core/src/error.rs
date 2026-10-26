//! What can be wrong with one field of a rule.

use std::fmt;

use crate::Key;

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
    /// A variable name that is empty or holds a `=`, as it was written.
    InvalidEnvName(String),
    /// A DLL override without the `=` between its modules and its load
    /// order, as it was written.
    InvalidDllOverride(String),
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(key) => write!(f, "unknown key {key:?}"),
            Self::BadValue { key, value, .. } => write!(f, "unknown {key} value {value:?}"),
            Self::Nul => f.write_str("a NUL in its value"),
            Self::MissingExe => f.write_str("no exe"),
            Self::EmptyArgument => f.write_str("an empty argument"),
            Self::InvalidEnvName(name) => write!(f, "invalid variable name {name:?}"),
            Self::InvalidDllOverride(text) => {
                write!(f, "DLL override {text:?} without '=' before its load order")
            }
        }
    }
}

impl std::error::Error for FieldError {}
