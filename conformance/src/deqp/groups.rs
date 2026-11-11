//! The typed form of `deqp-vk.toml`: the groups of the deqp-vk suite, each
//! a set of case patterns over the mustpass.
//!
//! [`Groups::builtin`] reads the copy compiled into the binary.

use std::collections::BTreeSet;

use toml_edit::{DocumentMut, Item, Table, Value};

/// The `deqp-vk.toml` next to `Cargo.toml`.
const DEQP_VK: &str = include_str!("../../deqp-vk.toml");

/// Every group, in file order.
pub struct Groups {
    pub groups: Vec<Group>,
}

/// One group: a name, the issues it serves and the cases it runs.
pub struct Group {
    /// Letters, digits, `_` and `-`; the stem of its expected results.
    pub name: String,
    /// The mach9 issues whose changes the group checks.
    pub issues: Vec<u32>,
    /// Patterns over case names ([`glob`]).
    pub cases: Vec<String>,
    /// Patterns of cases the group leaves out although `cases` matches
    /// them: features KosmicKrisp does not have at all.
    pub exclude: Vec<String>,
    /// Every how many matching cases one runs; 1 for all of them.
    pub fraction: usize,
}

impl Groups {
    /// The groups compiled into the binary.
    ///
    /// # Errors
    ///
    /// Returns a message when the compiled-in file does not parse; a unit
    /// test keeps that from shipping.
    pub fn builtin() -> Result<Self, String> {
        Self::parse(DEQP_VK)
    }

    /// Parse a `deqp-vk.toml` text.
    ///
    /// # Errors
    ///
    /// Returns a message naming the group for a TOML syntax error, an
    /// unknown or missing key, a value of the wrong type, a bad name or the
    /// name `triage`, a group named twice, a group without patterns, a
    /// pattern of `cases` or `exclude` that does not start with `dEQP-VK.`
    /// or holds whitespace, an issue that is not a positive number and a
    /// fraction below 1.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc: DocumentMut = text.parse().map_err(|e| format!("deqp-vk.toml: {e}"))?;
        if let Some((key, _)) = doc.as_table().iter().find(|(key, _)| *key != "group") {
            return Err(format!("deqp-vk.toml: unknown key `{key}`"));
        }
        let Some(item) = doc.get("group") else {
            return Ok(Self { groups: Vec::new() });
        };
        let tables = item
            .as_array_of_tables()
            .ok_or_else(|| String::from("deqp-vk.toml: `group` must be an array of tables"))?;
        let groups = tables
            .iter()
            .map(parse_group)
            .collect::<Result<Vec<_>, _>>()?;
        let mut seen = BTreeSet::new();
        for group in &groups {
            if !seen.insert(group.name.as_str()) {
                return Err(format!("deqp-vk.toml: group {} twice", group.name));
            }
        }
        Ok(Self { groups })
    }

    /// The groups `names` name, all of them for none, in file order.
    ///
    /// # Errors
    ///
    /// Returns a message for a name no group has.
    pub fn select(&self, names: &[String]) -> Result<Vec<&Group>, String> {
        if let Some(unknown) = names
            .iter()
            .find(|name| !self.groups.iter().any(|group| group.name == **name))
        {
            let known: Vec<&str> = self
                .groups
                .iter()
                .map(|group| group.name.as_str())
                .collect();
            return Err(format!(
                "unknown deqp-vk group {unknown:?}; the groups are {}",
                known.join(", ")
            ));
        }
        Ok(self
            .groups
            .iter()
            .filter(|group| names.is_empty() || names.contains(&group.name))
            .collect())
    }
}

impl Group {
    /// Whether `case` matches one of the group's patterns and none of its
    /// exclusions.
    #[must_use]
    pub fn matches(&self, case: &str) -> bool {
        self.cases.iter().any(|pattern| glob(pattern, case))
            && !self.exclude.iter().any(|pattern| glob(pattern, case))
    }

    /// The cases of `mustpass`, in its order, that the group runs: every
    /// [`Self::fraction`]th one that [`Self::matches`], starting with the
    /// first.
    #[must_use]
    pub fn select<'m>(&self, mustpass: &[&'m str]) -> Vec<&'m str> {
        mustpass
            .iter()
            .filter(|case| self.matches(case))
            .step_by(self.fraction)
            .copied()
            .collect()
    }
}

/// Whether `text` matches `pattern`, in which `*` matches any run of
/// characters, an empty one and dots included, and every other character
/// itself.
#[must_use]
pub fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

fn parse_group(table: &Table) -> Result<Group, String> {
    let name = table
        .get("name")
        .and_then(Item::as_str)
        .filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        })
        .ok_or_else(|| {
            String::from("deqp-vk.toml: a group needs a `name` of letters, digits, `_` and `-`")
        })?
        .to_owned();
    let context = format!("deqp-vk.toml: group {name}");
    if format!("{name}.txt") == crate::deqp::triage::TRIAGE_FILE {
        return Err(format!(
            "{context}: the name is taken by the triage file of the expected results"
        ));
    }
    if let Some((key, _)) = table
        .iter()
        .find(|(key, _)| !["name", "issues", "cases", "exclude", "fraction"].contains(key))
    {
        return Err(format!("{context}: unknown key `{key}`"));
    }
    let issues = match table.get("issues") {
        Some(item) => array(item, "issues", &context)?
            .iter()
            .map(|value| {
                value
                    .as_integer()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("{context}: `issues` holds issue numbers"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let cases = patterns(
        table
            .get("cases")
            .ok_or_else(|| format!("{context}: missing `cases`"))?,
        "cases",
        &context,
    )?;
    if cases.is_empty() {
        return Err(format!("{context}: `cases` is empty"));
    }
    let exclude = match table.get("exclude") {
        Some(item) => patterns(item, "exclude", &context)?,
        None => Vec::new(),
    };
    let fraction = match table.get("fraction") {
        Some(item) => item
            .as_integer()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n >= 1)
            .ok_or_else(|| format!("{context}: `fraction` must be a positive integer"))?,
        None => 1,
    };
    Ok(Group {
        name,
        issues,
        cases,
        exclude,
        fraction,
    })
}

/// The case patterns of the array `item`, the group's `key`.
fn patterns(item: &Item, key: &str, context: &str) -> Result<Vec<String>, String> {
    array(item, key, context)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|pattern| {
                    pattern.starts_with("dEQP-VK.") && !pattern.contains(char::is_whitespace)
                })
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!("{context}: each of `{key}` is a pattern that starts with `dEQP-VK.`")
                })
        })
        .collect()
}

fn array<'t>(item: &'t Item, key: &str, context: &str) -> Result<&'t toml_edit::Array, String> {
    item.as_value()
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}: `{key}` must be an array"))
}

#[cfg(test)]
mod tests;
