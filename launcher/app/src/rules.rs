//! The per-process half of the settings schema, and the value readers both
//! halves share.
//!
//! A config file's `[process."<name>"]` blocks hold the same settings as the
//! top level, aimed at one process instead of the whole launch. This module
//! turns them, after the launcher's own rule for the top-level settings, into
//! the [`Table`] that travels to `compatdb.so` in
//! [`ENV_VAR`](compatdb_core::ENV_VAR). A block's name stays with the
//! launcher: the table carries no names, and the library's log names each of
//! its rules by its line, which [`LaunchRules`] maps back to the block.
//!
//! The launcher knows none of the rules' keys. compatdb-core's settings layer
//! does: [`Key::from_setting`] names the key of a field, its [`KeyKind`] says
//! whether TOML holds it as one value, a list or a table of variables, and
//! [`Rule::set`], [`Rule::set_list`] and [`Rule::set_env`] check the value
//! and take it. An error is a warning or refuses the file by its
//! [`FieldError::severity`]. The launcher adds only its own fields of a block,
//! `enabled`, and refuses its prefix-wide settings in one.

use compatdb_core::{
    DEFAULT_VALUE, Exe, FieldError, Key, KeyKind, Origin, Rule, ScalarValue, Severity, Table,
    ValueKind, builtin,
};
use log::warn;
use toml_edit::{Item, TableLike};

use crate::words;

/// The values a plain true-or-false setting takes, such as `enabled`.
pub const BOOL_VALUES: &str = "`true` or `false`";

/// The field of a block that only the launcher reads: `false` leaves the
/// block out.
const ENABLED: &str = "enabled";

/// Fields a rule may not hold, each with the clause that says why, which
/// follows the field's name in the error. These are the launcher's settings
/// that have no per-process form at all.
const REJECTED_FIELDS: &[(&str, &str)] = &[
    (
        "retina_mode",
        "can only be set for the whole launch, at the top level: winemac.drv reads it from the \
         prefix-wide key on purpose, so that every process agrees on the display modes",
    ),
    (
        "dpi",
        "can only be set for the whole launch, at the top level: win32u reads it from the \
         shared Control Panel\\Desktop key, and the desktop process fixes each monitor's DPI \
         when the wine session boots",
    ),
];

/// The [`Key::Exe`] value a block without one is checked with. Such a block
/// matches no process and is left out, but its values are still checked, and
/// a check that depends on the executable needs one.
const UNNAMED_EXE: &str = "unnamed.exe";

/// A field of a rule as a config gives it, as text for the settings layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// The value of a matcher or of a setting that holds one.
    One(String),
    /// The entries of a list, which may be none.
    List(Vec<String>),
    /// Environment entries, a name and a value each.
    Env(Vec<(String, String)>),
}

/// The fields a config file asks for that the launcher ignores, each with
/// its warning. They are taken out of the file before it is merged, so a
/// value the other file gives for the same key still applies.
#[derive(Debug, Default)]
pub struct Ignored {
    /// One warning for each field, for the log, with the file's path.
    pub warnings: Vec<String>,
    /// Each field by the block it is in, `None` for the top level, and its
    /// key as written.
    pub fields: Vec<(Option<String>, String)>,
}

impl Ignored {
    /// Note that the field `key` of `block` is ignored for `error`.
    pub fn push(&mut self, block: Option<&str>, key: &str, error: &FieldError) {
        let warning = block.map_or_else(
            || error.to_string(),
            |name| format!("process rule `{name}`: {error}"),
        );
        self.warnings.push(warning);
        self.fields.push((block.map(str::to_owned), key.to_owned()));
    }
}

/// What the launcher says about `error` from the field `written`, the key as
/// the config wrote it: the key, the crate's text, and for a value the key
/// does not take, the values it does.
pub fn describe(written: &str, error: &FieldError) -> String {
    match error {
        FieldError::BadValue { expected, .. } => format!(
            "`{written}`: {error}; it takes {}",
            words::listing(expected, "`")
        ),
        _ => format!("`{written}`: {error}"),
    }
}

/// The keys the top level and the command line take for the launcher's own
/// rule: every key a rule for every process allows, but the ones whose
/// names the launcher's own settings in `own` take.
pub fn launch_wide_keys(own: &[&str]) -> Vec<Key> {
    Key::ALL
        .iter()
        .copied()
        .filter(|key| !own.contains(&key.as_str()) && key.allowed_on_any())
        .collect()
}

/// Put `value` into the field `key` of `rule`. A list of entries adds each
/// one after the entries the field has; no entries make the field an empty
/// list, which for some keys is a value of its own.
pub fn apply(rule: &mut Rule, key: Key, value: &Value) -> Result<(), FieldError> {
    match value {
        Value::One(text) => rule.set(key, text),
        Value::List(entries) if entries.is_empty() => rule.set_list(key, entries),
        Value::List(entries) => entries.iter().try_for_each(|entry| rule.set(key, entry)),
        Value::Env(entries) => entries
            .iter()
            .try_for_each(|(name, value)| rule.set_env(name, value)),
    }
}

/// Check `value` for the field `key` of the launcher's own rule, the one
/// that matches every process.
pub fn check_launch_wide(key: Key, value: &Value) -> Result<(), FieldError> {
    apply(&mut Rule::new(Exe::Any), key, value)
}

/// The launcher's own rule, which matches every process and carries the
/// top-level settings in `fields`, each checked already.
pub fn launch_rule<'a>(
    fields: impl IntoIterator<Item = (&'a Key, &'a Value)>,
) -> Result<Rule, String> {
    let mut rule = Rule::new(Exe::Any);
    for (key, value) in fields {
        apply(&mut rule, *key, value).map_err(|e| describe(key.as_str(), &e))?;
    }
    Ok(rule)
}

/// One `[process."<name>"]` block, read into the rule it sends.
#[derive(Debug, PartialEq, Eq)]
pub struct NamedRule {
    pub name: String,
    pub rule: Rule,
}

/// A config's `[process]` rules, checked, with what is worth a warning about
/// them. The warnings are kept rather than logged, because a file is read on
/// its own before it is merged and only the merged rules deserve them.
#[derive(Debug, Default)]
pub struct Rules {
    pub rules: Vec<NamedRule>,
    pub warnings: Vec<String>,
}

/// The table the launcher sends, with the block each of its rules came from.
#[derive(Debug, Default)]
pub struct LaunchRules {
    /// Each rule's [`Source`](compatdb_core::Source) is its line in the
    /// table's text, as `compatdb.so` reads it, so a table resolved here
    /// names its values the way the library's log does.
    pub table: Table,
    /// The block name of each rule of `table`, at the same index: `None` for
    /// the launcher's own rule, which carries the top-level settings.
    pub names: Vec<Option<String>>,
}

impl LaunchRules {
    /// What the config calls the rule a value of a [`Table::resolve`] here
    /// came from: "top level", or "rule" and the block's name in backticks.
    /// `compatdb.so`'s log calls the same rule by its line.
    pub fn describe(&self, origin: &Origin) -> String {
        let name = match origin {
            Origin::Line(n) => n.checked_sub(1).and_then(|index| self.names.get(index)),
            Origin::BuiltIn | Origin::BuiltInFor(_) | Origin::FileLine(_) => None,
        };
        match name {
            Some(None) => String::from("top level"),
            Some(Some(name)) => format!("rule `{name}`"),
            None => origin.label(),
        }
    }

    /// A warning for each pair of rules that match exactly the same
    /// processes, which can only be a mistake.
    fn duplicate_warnings(&self) -> Vec<String> {
        let describe = |index: usize| self.describe(&Origin::Line(index.saturating_add(1)));
        self.table
            .duplicate_matchers()
            .into_iter()
            .map(|(one, two)| {
                format!(
                    "{} and {} match exactly the same processes; both apply, which is unlikely \
                     to be what was meant",
                    describe(one),
                    describe(two)
                )
            })
            .collect()
    }
}

/// Build the table from the launcher's own rule and a config's rules.
///
/// The launcher's own rule comes first and is left out when it sets
/// nothing. Order decides only between rules that `compatdb.so` folds in the
/// same place (see [`Table::resolve`]): a `[process]` block for
/// [`Exe::Any`] comes after the launcher's own rule, so it wins over the top
/// level.
pub fn table(own: Rule, rules: Vec<NamedRule>) -> LaunchRules {
    let mut launch = LaunchRules::default();
    let own = (!own.sets_nothing()).then_some((None, own));
    let named = rules
        .into_iter()
        .map(|named| (Some(named.name), named.rule));
    for (name, rule) in own.into_iter().chain(named) {
        launch.table.push(rule);
        launch.names.push(name);
    }
    for warning in launch.duplicate_warnings() {
        warn!("{warning}");
    }
    launch
}

/// Read a config's `[process]` item. The fields the launcher ignores, which
/// the merge never sees, go to `ignored`: their warnings belong with the
/// file's own, which are logged with its path.
pub fn parse(item: &Item, ignored: &mut Ignored) -> Result<Rules, String> {
    let Some(table) = item.as_table_like() else {
        return Err(format!(
            "`process` must be a table of rules, for example [process.\"my-rule\"], got {}",
            shown(item)
        ));
    };
    let mut rules = Rules::default();
    for (name, value) in table.iter() {
        let Some(fields) = value.as_table_like() else {
            return Err(format!(
                "process rule `{name}` must be a table, for example [process.\"{name}\"], got {}",
                shown(value)
            ));
        };
        let rule = parse_rule(name, fields, &mut rules.warnings, ignored)
            .map_err(|e| format!("process rule `{name}`: {e}"))?;
        if let Some(rule) = rule {
            rules.rules.push(NamedRule {
                name: name.to_owned(),
                rule,
            });
        }
    }
    Ok(rules)
}

/// One `[process."<name>"]` block, or `None` for a block that is left out: one
/// with `enabled = false`, and one without a [`Key::Exe`], which matches no
/// process. Every field is checked, also in a block that is left out, so a
/// value a key does not take is an error wherever it is written. The warnings
/// go to `warnings`, except those about a field the launcher ignores, which go
/// to `ignored` (see [`parse`]).
fn parse_rule(
    name: &str,
    fields: &dyn TableLike,
    warnings: &mut Vec<String>,
    ignored: &mut Ignored,
) -> Result<Option<Rule>, String> {
    let mut enabled = true;
    let mut exe = None;
    let mut values: Vec<(&str, Key, Value)> = Vec::new();
    for (written, item) in fields.iter() {
        if written == ENABLED {
            enabled = boolean(item, written)?;
            continue;
        }
        if let Some((_, reason)) = REJECTED_FIELDS.iter().find(|(key, _)| *key == written) {
            return Err(format!("`{written}` {reason}"));
        }
        let key = match Key::from_setting(written) {
            Ok(key) => key,
            Err(FieldError::UnknownKey(_)) => return Err(unknown_field(written)),
            Err(e) if e.severity() == Severity::Warning => {
                ignored.push(Some(name), written, &e);
                continue;
            }
            Err(e) => return Err(describe(written, &e)),
        };
        let seen = exe.iter().chain(&values).any(|(_, seen, _)| *seen == key);
        if seen {
            return Err(describe(written, &FieldError::Duplicate(key)));
        }
        let value = read(item, key, written)?;
        // The rule is made with its executable first, which some values
        // depend on.
        if key == Key::Exe {
            exe = Some((written, key, value));
        } else {
            values.push((written, key, value));
        }
    }
    let mut rule = Rule::new(Exe::new(UNNAMED_EXE).map_err(|e| describe(UNNAMED_EXE, &e))?);
    if let Some((written, key, value)) = &exe {
        apply(&mut rule, *key, value).map_err(|e| describe(written, &e))?;
    }
    for (written, key, value) in values {
        match apply(&mut rule, key, &value) {
            Ok(()) => {}
            Err(e) if e.severity() == Severity::Warning => ignored.push(Some(name), written, &e),
            Err(e) => return Err(describe(written, &e)),
        }
    }

    if exe.is_none() {
        warnings.push(left_out_without_exe(name, enabled));
        return Ok(None);
    }
    if !enabled {
        // How a game file turns off a rule of gamelauncher.toml. A block of
        // a former built-in rule's name used to turn that rule off too,
        // whatever executable it named, which it no longer does.
        if let Some(former) = builtin::former(name) {
            warnings.push(format!(
                "process rule `{name}` has `enabled = false`, so it is left out; {former}; {}",
                former.instead_of_turning_off()
            ));
        }
        return Ok(None);
    }
    if rule.is_unfingerprinted() {
        let fingerprints: Vec<&str> = Key::ALL
            .iter()
            .filter(|key| key.kind() == KeyKind::Matcher && **key != Key::Exe)
            .map(|key| key.as_str())
            .collect();
        warnings.push(format!(
            "process rule `{name}` matches every program called {}; add one of {} to pin it \
             to one",
            rule.exe(),
            words::listing(&fingerprints, "`")
        ));
    }
    Ok(Some(rule))
}

/// The error for a field a rule does not take, which lists those it does.
fn unknown_field(written: &str) -> String {
    let known: Vec<&str> = Key::ALL
        .iter()
        .map(|key| key.as_str())
        .chain([ENABLED])
        .collect();
    format!(
        "unknown field `{written}`; a rule takes {}",
        words::listing(&known, "`")
    )
}

/// The warning for a block without the matcher key [`Key::Exe`], which is
/// left out. A file written for an older launcher used such a block to extend
/// or turn off a built-in rule of `compatdb.so` by naming it, which no longer
/// exists; the warning says what does that now.
fn left_out_without_exe(name: &str, enabled: bool) -> String {
    let exe = Key::Exe;
    let what = if enabled {
        format!("sets no `{exe}`")
    } else {
        format!("has `{ENABLED} = false` and no `{exe}`")
    };
    let instead = match (builtin::former(name), enabled) {
        (Some(former), true) => format!("{former}; {}", former.instead_of_extending()),
        (Some(former), false) => format!("{former}; {}", former.instead_of_turning_off()),
        (None, true) => {
            format!("give the block an `{exe}` to make it a rule of its own, or remove it")
        }
        (None, false) => String::from("remove the block"),
    };
    format!("process rule `{name}` {what}, so it matches no process and is left out; {instead}")
}

/// The value of the field `key`, written `written`, as TOML holds it for the
/// kind of field the key names.
pub fn read(item: &Item, key: Key, written: &str) -> Result<Value, String> {
    match key.kind() {
        KeyKind::Scalar(ValueKind::Bool) => switch(item, written).map(Value::One),
        KeyKind::List => strings(item, written).map(Value::List),
        KeyKind::Env => env_pairs(item, written).map(Value::Env),
        _ => string(item, written).map(Value::One),
    }
}

// The value readers. A config's top level reads its settings with the same
// ones, so a value is held to the same type and refused with the same words
// wherever it is written: "`key` must be <what it takes>, got <what it is>".

/// A value as the file wrote it, for an error that says what was found.
pub fn shown(item: &Item) -> String {
    match item {
        Item::Value(value) => {
            let mut value = value.clone();
            value.decor_mut().clear();
            format!("`{value}`")
        }
        Item::Table(_) => String::from("a table"),
        Item::ArrayOfTables(_) => String::from("an array of tables"),
        Item::None => String::from("nothing"),
    }
}

/// Run `text` through a setting's parser, without regard to case. A value
/// the parser does not take is an error that names the ones it does.
pub fn parse_value<T>(
    key: &str,
    text: &str,
    parse: impl FnOnce(&str) -> Option<T>,
    valid: &str,
) -> Result<T, String> {
    parse(&text.to_ascii_lowercase())
        .ok_or_else(|| format!("`{key}` must be {valid}, got `{text}`"))
}

pub fn string(item: &Item, key: &str) -> Result<String, String> {
    item.as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("`{key}` must be a string, got {}", shown(item)))
}

/// A true-or-false value. Any other value is an error rather than something
/// that quietly reads as unset.
pub fn boolean(item: &Item, key: &str) -> Result<bool, String> {
    item.as_bool()
        .ok_or_else(|| format!("`{key}` must be {BOOL_VALUES}, got {}", shown(item)))
}

/// A true-or-false setting of a rule, as the settings layer reads it: a TOML
/// boolean, or the string that resets the setting, [`DEFAULT_VALUE`], in any
/// case.
fn switch(item: &Item, key: &str) -> Result<String, String> {
    if let Some(on) = item.as_bool() {
        return Ok(ScalarValue::Bool(on).to_string());
    }
    match item.as_str() {
        Some(text) if text.eq_ignore_ascii_case(DEFAULT_VALUE) => Ok(text.to_owned()),
        _ => Err(format!(
            "`{key}` must be `true`, `false` or `\"{DEFAULT_VALUE}\"`, got {}",
            shown(item)
        )),
    }
}

/// A list setting, which may also be written as a single string.
pub fn strings(item: &Item, key: &str) -> Result<Vec<String>, String> {
    if let Some(s) = item.as_str() {
        return Ok(vec![s.to_owned()]);
    }
    let not_strings = || {
        format!(
            "`{key}` must be a string or a list of strings, got {}",
            shown(item)
        )
    };
    item.as_array()
        .ok_or_else(not_strings)?
        .iter()
        .map(|value| value.as_str().map(str::to_owned).ok_or_else(not_strings))
        .collect()
}

/// An `env` table, `NAME = "value"` each.
pub fn env_pairs(item: &Item, key: &str) -> Result<Vec<(String, String)>, String> {
    let Some(env) = item.as_table_like() else {
        return Err(format!(
            "`{key}` must be a table of NAME = \"value\", got {}",
            shown(item)
        ));
    };
    env.iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| format!("`{key}.{name}` must be a string, got {}", shown(value)))?;
            Ok((name.to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests;
