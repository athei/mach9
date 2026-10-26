//! What can be wrong with one field of a rule.

use std::fmt;

use crate::Key;

/// Why a field of a rule cannot be read, or a value cannot be set.
///
/// The [`Display`](fmt::Display) form is the phrase `compatdb.so` logs after
/// `ignoring`, for example `unknown key "bogus"`.
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
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(key) => write!(f, "unknown key {key:?}"),
            Self::BadValue { key, value, .. } => write!(f, "unknown {key} value {value:?}"),
        }
    }
}

impl std::error::Error for FieldError {}
