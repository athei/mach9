//! The human-owned notes on tests, `conformance/baselines/triage.txt`: per
//! test an issue number and whether its results flake.
//!
//! A line names a test of every leg and its notes; `#` starts a comment, a
//! whole line or the rest of one:
//!
//! ```text
//! d3d11/d3d11/test_swapchain_flip issue=12 flaky  # races the present thread
//! ddraw/visual/* issue=40
//! ```
//!
//! The diff reports a flaky test's changes without gating on them, except
//! a new crash or stall. A unit test fails on a line that names a test no
//! committed baseline has.

use std::{collections::BTreeMap, fmt, fs, path::Path};

use crate::{model::is_ident, outcome::Key};

/// A test of every leg: `<exe>/<unit>/<key>`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TestPath {
    pub exe: String,
    pub unit: String,
    pub key: Key,
}

impl TestPath {
    /// The test `text` names.
    ///
    /// # Errors
    ///
    /// Returns a message when it is not three `/`-separated parts, an exe
    /// name of letters, digits, `_` and `-`, a unit that is a C name, and a
    /// key ([`Key::parse`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split('/').collect();
        let [exe, unit, key] = parts[..] else {
            return Err(format!("{text:?} is not <exe>/<unit>/<key>"));
        };
        let exe_valid = !exe.is_empty()
            && exe
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
        if !exe_valid {
            return Err(format!("{exe:?} is not an exe name"));
        }
        if !is_ident(unit) {
            return Err(format!("unit {unit:?} is not a C name"));
        }
        let key = Key::parse(key).ok_or_else(|| format!("{key:?} is not a key"))?;
        Ok(Self {
            exe: exe.to_owned(),
            unit: unit.to_owned(),
            key,
        })
    }
}

impl fmt::Display for TestPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.exe, self.unit, self.key)
    }
}

/// The notes on one test.
#[derive(PartialEq, Eq, Debug)]
pub struct Note {
    /// The issue that tracks it.
    pub issue: Option<u32>,
    /// Whether its results change between runs of one build.
    pub flaky: bool,
    /// The 1-based line of the triage file that holds it.
    pub line: usize,
}

/// The notes of the triage file, by test.
pub struct Triage {
    pub notes: BTreeMap<TestPath, Note>,
}

impl Triage {
    /// The path of the committed triage file.
    #[must_use]
    pub fn path() -> std::path::PathBuf {
        crate::baseline::dir().join(crate::baseline::TRIAGE_FILE)
    }

    /// Read the triage text; `file` names it in errors.
    ///
    /// # Errors
    ///
    /// Returns `<file>:<line>: <message>` for a bad test ([`TestPath::parse`]),
    /// a test named twice, a token other than `issue=<n>` and `flaky`, and
    /// either of them twice on one line.
    pub fn parse(text: &str, file: &str) -> Result<Self, String> {
        let mut notes: BTreeMap<TestPath, Note> = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            let content = line.split_once('#').map_or(line, |(before, _)| before);
            let mut tokens = content.split_whitespace();
            let Some(name) = tokens.next() else {
                continue;
            };
            let at = |e: String| format!("{file}:{number}: {e}");
            let path = TestPath::parse(name).map_err(at)?;
            let mut note = Note {
                issue: None,
                flaky: false,
                line: number,
            };
            for token in tokens {
                if token == "flaky" {
                    if note.flaky {
                        return Err(at(String::from("`flaky` twice")));
                    }
                    note.flaky = true;
                } else if let Some(issue) = token.strip_prefix("issue=") {
                    let issue = issue
                        .parse::<u32>()
                        .ok()
                        .filter(|&issue| issue > 0)
                        .ok_or_else(|| at(format!("{issue:?} is not an issue number")))?;
                    if note.issue.replace(issue).is_some() {
                        return Err(at(String::from("two issues")));
                    }
                } else {
                    return Err(at(format!(
                        "unknown token {token:?}; a test takes `issue=<n>` and `flaky`"
                    )));
                }
            }
            if let Some(first) = notes.get(&path) {
                return Err(at(format!("{path} again, first on line {}", first.line)));
            }
            notes.insert(path, note);
        }
        Ok(Self { notes })
    }

    /// Read the triage file at `path`.
    ///
    /// # Errors
    ///
    /// Returns a message when it cannot be read or does not parse.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text, &path.display().to_string())
    }

    /// Whether the triage marks `key` of `unit` of `exe` flaky.
    #[must_use]
    pub fn is_flaky(&self, exe: &str, unit: &str, key: &Key) -> bool {
        let test = TestPath {
            exe: exe.to_owned(),
            unit: unit.to_owned(),
            key: key.clone(),
        };
        self.notes.get(&test).is_some_and(|note| note.flaky)
    }
}

#[cfg(test)]
mod tests;
