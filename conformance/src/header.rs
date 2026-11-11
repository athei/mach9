//! Which build a set of results comes from: the Wine version of the test
//! executables and the commits of the components the bundle carries.
//!
//! [`Header::read`] reads them from a bundle's `lib/wine/tests`; a baseline
//! stores them in its header; [`changes`] lists what differs between two.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::Path,
};

/// The build a set of results comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Header {
    /// The first line of `lib/wine/tests/VERSION`: the `wine --version` of
    /// the Wine the tests were built from.
    pub wine_version: String,
    /// The `name=commit` lines of `lib/wine/tests/COMPONENTS`, by name.
    pub components: BTreeMap<String, String>,
}

/// One way two [`Header`]s differ.
#[derive(PartialEq, Eq, Debug)]
pub enum HeaderChange {
    /// The tests come from another Wine.
    Wine { from: String, to: String },
    /// A component has another commit, or exists on one side only.
    Component {
        name: String,
        from: Option<String>,
        to: Option<String>,
    },
}

impl fmt::Display for HeaderChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let side = |commit: &Option<String>| commit.clone().unwrap_or_else(|| String::from("none"));
        match self {
            Self::Wine { from, to } => write!(f, "Wine {from} -> {to}"),
            Self::Component { name, from, to } => {
                write!(f, "{name} {} -> {}", side(from), side(to))
            }
        }
    }
}

impl Header {
    /// Read the header of the tests in `tests`, a bundle's
    /// `lib/wine/tests`, from its `VERSION` and `COMPONENTS`.
    ///
    /// # Errors
    ///
    /// Returns a message naming the file when either cannot be read or
    /// [`Self::parse`] rejects it.
    pub fn read(tests: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            let path = tests.join(name);
            fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
        };
        Self::parse(&read("VERSION")?, &read("COMPONENTS")?)
            .map_err(|e| format!("{}: {e}", tests.display()))
    }

    /// The header of a `VERSION` text and a `COMPONENTS` text.
    ///
    /// # Errors
    ///
    /// Returns a message when the first line of `version` is empty or holds
    /// whitespace, or when a non-empty line of `components` is not
    /// a `name=commit` line (`parse_component`) or repeats a name.
    pub fn parse(version: &str, components: &str) -> Result<Self, String> {
        let wine_version = version.lines().next().unwrap_or_default().trim_end();
        if !is_word(wine_version) {
            return Err(format!(
                "VERSION: the first line {wine_version:?} is not a version"
            ));
        }
        let mut header = Self {
            wine_version: wine_version.to_owned(),
            components: BTreeMap::new(),
        };
        for (index, line) in components.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let (name, commit) =
                parse_component(line).map_err(|e| format!("COMPONENTS:{}: {e}", index + 1))?;
            header.add_component(name, commit)?;
        }
        Ok(header)
    }

    /// Add the component `name` at `commit`.
    ///
    /// # Errors
    ///
    /// Returns a message when the header has `name` already.
    pub(crate) fn add_component(&mut self, name: &str, commit: &str) -> Result<(), String> {
        if self
            .components
            .insert(name.to_owned(), commit.to_owned())
            .is_some()
        {
            return Err(format!("component {name} is named twice"));
        }
        Ok(())
    }
}

/// The name and commit of a `name=commit` line.
///
/// # Errors
///
/// Returns a message when there is no `=`, the name is empty or holds
/// anything but letters, digits, `_`, `-` and `.`, or the commit is empty or
/// holds whitespace.
pub(crate) fn parse_component(line: &str) -> Result<(&str, &str), String> {
    let (name, commit) = line
        .split_once('=')
        .ok_or_else(|| format!("{line:?} is not name=commit"))?;
    let name_valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !name_valid {
        return Err(format!("{name:?} is not a component name"));
    }
    if !is_word(commit) {
        return Err(format!("{commit:?} is not a commit"));
    }
    Ok((name, commit))
}

/// Whether `text` is non-empty and holds no whitespace.
fn is_word(text: &str) -> bool {
    !text.is_empty() && !text.contains(char::is_whitespace)
}

/// What differs between the header of a baseline and that of a run: the
/// Wine version, then each component by name.
///
/// Versions and commits that name one commit with hashes of different
/// lengths are the same (`same_commit`).
#[must_use]
pub fn changes(baseline: &Header, run: &Header) -> Vec<HeaderChange> {
    let mut changes = Vec::new();
    if !same_wine_version(&baseline.wine_version, &run.wine_version) {
        changes.push(HeaderChange::Wine {
            from: baseline.wine_version.clone(),
            to: run.wine_version.clone(),
        });
    }
    let names: BTreeSet<&String> = baseline
        .components
        .keys()
        .chain(run.components.keys())
        .collect();
    for name in names {
        let from = baseline.components.get(name);
        let to = run.components.get(name);
        let same = matches!((from, to), (Some(from), Some(to)) if same_commit(from, to));
        if !same {
            changes.push(HeaderChange::Component {
                name: name.clone(),
                from: from.cloned(),
                to: to.cloned(),
            });
        }
    }
    changes
}

/// Whether two `wine --version` strings name one build.
///
/// A `git describe` version ends in `-g` and an abbreviated hash whose
/// length git picks per repository, so one commit can read
/// `wine-11.21-37-g53def89293a` in one build and `wine-11.21-37-g53def89293`
/// in another. They are one build when the text before `-g` is equal and the
/// hashes are [`same_commit`]. A `-dirty` after the hash, which `git
/// describe --dirty` adds for a tree with changes, must be on both or
/// neither.
#[must_use]
pub(crate) fn same_wine_version(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (a, a_dirty) = a.strip_suffix("-dirty").map_or((a, false), |a| (a, true));
    let (b, b_dirty) = b.strip_suffix("-dirty").map_or((b, false), |b| (b, true));
    if a_dirty != b_dirty {
        return false;
    }
    match (a.rsplit_once("-g"), b.rsplit_once("-g")) {
        (Some((a_base, a_hash)), Some((b_base, b_hash))) => {
            a_base == b_base && same_commit(a_hash, b_hash)
        }
        _ => false,
    }
}

/// Whether two commit hashes name one commit: they are equal, or both are
/// at least seven hex digits, git's shortest abbreviation, and the shorter
/// is a prefix of the longer.
#[must_use]
pub(crate) fn same_commit(a: &str, b: &str) -> bool {
    let is_hash = |hash: &str| hash.len() >= 7 && hash.bytes().all(|c| c.is_ascii_hexdigit());
    a == b || (is_hash(a) && is_hash(b) && (a.starts_with(b) || b.starts_with(a)))
}

#[cfg(test)]
mod tests;
