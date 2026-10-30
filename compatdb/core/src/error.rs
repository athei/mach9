//! What can be wrong with one field of a rule, and with a line of
//! [`ENV_VAR`].

use std::fmt;

use crate::{ENV_VAR, Key};
#[cfg(doc)]
use crate::{Rule, Table, retired_env};

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
    /// An `exe` that starts with `steam:` and has no app id after it, as it
    /// was written (see [`Exe::new`](crate::Exe::new)).
    BadSteamApp(String),
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
    /// A key that the settings of a launcher may still hold but that is no
    /// longer read (see [`Key::from_setting`]).
    RetiredKey {
        /// The key as it was written.
        key: String,
        /// Why it is no longer read and what to do instead, a clause that
        /// follows the key.
        reason: &'static str,
        /// Whether a launcher ignores it or refuses it.
        severity: Severity,
    },
    /// A value of `key` that is no longer read (see [`Rule::set`]).
    RetiredValue {
        /// The key whose value it is.
        key: Key,
        /// The value as it was written.
        value: String,
        /// Why it is no longer read and what to do instead, a clause that
        /// follows the value.
        reason: &'static str,
        /// Whether a launcher ignores it or refuses it.
        severity: Severity,
    },
    /// An environment variable that a setting replaced, as it was written,
    /// with the key of that setting (see [`retired_env`]).
    RetiredEnv {
        /// The variable as it was written.
        name: String,
        /// The setting that replaced it.
        key: Key,
    },
    /// `arguments_override` in a rule whose `exe` is `*` (see [`Rule::set`]).
    ArgumentsOverrideOnAny,
    /// A setting that a rule whose `exe` is a Steam app id refuses (see
    /// [`Rule::set`]).
    RefusedOnSteamApp(Key),
    /// A list of values for a key that holds one (see [`Rule::set_list`]).
    NotAList(Key),
}

/// How a launcher treats a [`FieldError`] from its own settings, as
/// [`FieldError::severity`] gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    /// The value is ignored, the rule is left as it was, and the launcher
    /// prints the error as a warning and goes on.
    Warning,
    /// The launcher refuses the settings: what they ask for would not
    /// happen.
    Error,
}

impl FieldError {
    /// How a launcher treats this error from [`Rule::set`],
    /// [`Rule::set_list`], [`Rule::set_env`] or [`Key::from_setting`]:
    /// [`Severity::Warning`] for a retired key or value that asks for
    /// nothing this Wine could still do, [`Severity::Error`] for everything
    /// else.
    ///
    /// [`Table::parse_lenient`] ignores every field it cannot read, whatever
    /// its severity.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        match self {
            Self::RetiredKey { severity, .. } | Self::RetiredValue { severity, .. } => *severity,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(key) => write!(f, "unknown key {key:?}"),
            Self::BadValue { key, value, .. } => write!(f, "unknown {key} value {value:?}"),
            Self::Nul => f.write_str("a NUL in its value"),
            Self::MissingExe => f.write_str("no exe"),
            Self::BadSteamApp(text) => write!(f, "invalid Steam app id {text:?}"),
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
            Self::RetiredKey { key, reason, .. } => write!(f, "{key} {reason}"),
            Self::RetiredValue {
                key, value, reason, ..
            } => write!(f, "{key} value {value:?} {reason}"),
            Self::RetiredEnv { name, key } => {
                write!(f, "env {name} is no longer read; use the {key} setting")
            }
            Self::ArgumentsOverrideOnAny => f.write_str(
                "arguments_override needs an exe other than *: for every process it would also \
                 take away the arguments the built-in rules give single programs, which some of \
                 them need to work",
            ),
            Self::RefusedOnSteamApp(Key::DebugChannels) => f.write_str(
                "debug_channels needs an exe that names a program: WINEDEBUG scopes an entry by \
                 the image name, which a Steam app id does not give",
            ),
            Self::RefusedOnSteamApp(key) => write!(
                f,
                "{key} needs an exe that names a program: a Steam app id can match more than one \
                 program, and it would take away the arguments the built-in rules give them, \
                 which some of them need to work"
            ),
            Self::NotAList(key) => write!(f, "{key} holds one value, not a list"),
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
