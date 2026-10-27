//! TOML lines and names for the tests, spelled from compatdb-core's own
//! names, so that no test writes a key, a value or a retired name of the
//! rules itself.

use std::fmt::Display;

use compatdb_core::{
    Key, KeyKind, ScalarValue, Severity, retired_envs, retired_keys, retired_values,
};

/// `key = "value"`, a field whose value TOML holds as a string. A dotted
/// `key` such as `env.NAME` is written as TOML's dotted key.
pub fn text(key: impl Display, value: impl Display) -> String {
    format!("{key} = {}\n", toml_edit::Value::from(value.to_string()))
}

/// `key = true` or `false`, a true-or-false field.
pub fn flag(key: impl Display, on: bool) -> String {
    format!("{key} = {on}\n")
}

/// `key = ["a", "b"]`, a list.
pub fn list(key: impl Display, entries: &[&str]) -> String {
    let array: toml_edit::Array = entries.iter().copied().collect();
    format!("{key} = {array}\n")
}

/// The field `key` with the crate's example value, as a file writes it: a
/// boolean, a list of one entry, an `env` entry under its name, or a string.
pub fn example(key: Key) -> String {
    let value = key.example();
    match key.kind() {
        KeyKind::List => list(key, &[value]),
        KeyKind::Env => {
            let (name, value) = value.split_once('=').expect("NAME=value");
            text(format!("{key}.{name}"), value)
        }
        _ => match key
            .scalar()
            .and_then(|scalar| scalar.parse_value(value).ok())
        {
            Some(ScalarValue::Bool(on)) => flag(key, on),
            _ => text(key, value),
        },
    }
}

/// A word a scalar setting `key` takes other than `default`, the `n`th.
pub fn word(key: Key, n: usize) -> &'static str {
    key.values()
        .iter()
        .copied()
        .filter(|value| !value.eq_ignore_ascii_case(compatdb_core::DEFAULT_VALUE))
        .nth(n)
        .unwrap_or_else(|| panic!("{key} has no word {n}"))
}

/// A key the settings layer no longer reads, of `severity`.
pub fn retired_key(severity: Severity) -> &'static str {
    retired_keys()
        .find(|(_, of)| *of == severity)
        .map(|(key, _)| key)
        .expect("a retired key of that severity")
}

/// A value the settings layer no longer reads, of `severity`, with its key.
pub fn retired_value(severity: Severity) -> (Key, &'static str) {
    retired_values()
        .find(|(_, _, of)| *of == severity)
        .map(|(key, value, _)| (key, value))
        .expect("a retired value of that severity")
}

/// An environment variable that a setting of the rules replaced.
pub fn retired_env() -> &'static str {
    retired_envs()
        .next()
        .map(|(name, _)| name)
        .expect("a retired variable")
}
