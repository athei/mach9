//! The committed baseline of one test executable,
//! `conformance/baselines/<exe>.txt`: the [`Outcome`] of every test of
//! every leg, and the [`Header`] of the build it was recorded on.
//!
//! The file is written by [`Baseline::to_text`] and read back by
//! [`Baseline::parse`]; `record` writes it and nothing else should. It holds
//! one line per key, passing keys included, so a key that disappears is
//! visible, and no `file:line` sites, since Wine's line numbers move on
//! every update:
//!
//! ```text
//! # Conformance baseline of d3d8_test.exe, one line per test.
//! # Written by `cargo run -p conformance -- record d3d8`: do not edit.
//! # wine: wine-11.21-37-g53def89293a
//! # component: dxmt=8c0b5f1
//!
//! [i386-windows device]
//! test_filter fail failed=2 sites=1 todo_succeeded=0 skipped=0
//! test_fpu_setup ok failed=0 sites=0 todo_succeeded=0 skipped=1
//!
//! [i386-windows stateblock]
//! * ok failed=0 sites=0 todo_succeeded=0 skipped=0
//! ```
//!
//! A section is a leg and a unit; a unit that runs as a whole has the one
//! key `*`. Sections and keys are sorted, so an unchanged baseline writes
//! the same bytes.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

use crate::{
    header::{Header, parse_component},
    model::{PeDir, is_ident},
    outcome::{Key, Outcome, TestId},
};

/// The name of the triage file in the baselines directory, which is not a
/// baseline.
pub const TRIAGE_FILE: &str = "triage.txt";

/// The directory of the committed baselines, `conformance/baselines`.
#[must_use]
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("baselines")
}

/// The baseline of one test executable.
#[derive(PartialEq, Eq, Debug)]
pub struct Baseline {
    /// The executable's name without `_test.exe`, as `wine-tests.toml`
    /// names it.
    pub exe: String,
    pub header: Header,
    pub entries: BTreeMap<TestId, Outcome>,
}

impl Baseline {
    /// The text of the file.
    #[must_use]
    pub fn to_text(&self) -> String {
        let exe = &self.exe;
        let mut text = format!(
            "# Conformance baseline of {exe}_test.exe, one line per test.\n# Written by `cargo \
             run -p conformance -- record {exe}`: do not edit.\n# wine: {}\n",
            self.header.wine_version
        );
        for (name, commit) in &self.header.components {
            let _ = writeln!(text, "# component: {name}={commit}");
        }
        let mut section: Option<(PeDir, &str)> = None;
        for (id, outcome) in &self.entries {
            if section != Some((id.leg, &id.unit)) {
                section = Some((id.leg, &id.unit));
                let _ = write!(text, "\n[{} {}]\n", id.leg.as_str(), id.unit);
            }
            let _ = writeln!(text, "{} {outcome}", id.key);
        }
        text
    }

    /// Read the text of the baseline of `exe`; `file` names it in errors.
    ///
    /// The comment lines before the first section are the header: `# wine:
    /// <version>` once, `# component: <name>=<commit>` per component, and
    /// prose. A comment that starts with `wine:` or `component:` in another
    /// spelling is an error, not prose, so a mistyped field is not lost.
    /// After the header come blank lines, `[<leg> <unit>]` section lines and
    /// `<key> <outcome>` lines.
    ///
    /// # Errors
    ///
    /// Returns `<file>:<line>: <message>` for a header without `# wine:` or
    /// with it twice, a bad or repeated component, a comment after the
    /// first section, an unknown leg, a unit that is not a C name, a section
    /// given twice or without keys, a key line before any section, a bad
    /// key, a key given twice in a section, a `*` key next to others, and a
    /// bad outcome ([`Outcome::parse`]).
    pub fn parse(text: &str, exe: &str, file: &str) -> Result<Self, String> {
        let mut parser = Parser {
            line: 0,
            wine_version: None,
            header: Header {
                wine_version: String::new(),
                components: BTreeMap::new(),
            },
            section: None,
            sections: BTreeMap::new(),
            entries: BTreeMap::new(),
        };
        for line in text.lines() {
            parser.line += 1;
            parser
                .read(line)
                .map_err(|e| format!("{file}:{}: {e}", parser.line))?;
        }
        parser
            .close_section()
            .map_err(|e| format!("{file}:{}: {e}", parser.line))?;
        let mut header = parser.header;
        header.wine_version = parser
            .wine_version
            .ok_or_else(|| format!("{file}: no `# wine: <version>` line"))?;
        Ok(Self {
            exe: exe.to_owned(),
            header,
            entries: parser.entries,
        })
    }

    /// Read the baseline of `exe` from `path`, or `None` when the file does
    /// not exist.
    ///
    /// # Errors
    ///
    /// Returns a message for a file that cannot be read or does not parse.
    pub fn load(path: &Path, exe: &str) -> Result<Option<Self>, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, exe, &path.display().to_string()).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

/// Every baseline in `dir`: each `<exe>.txt` but [`TRIAGE_FILE`], in the
/// order of their names.
///
/// # Errors
///
/// Returns a message when the directory or a file cannot be read or a file
/// does not parse.
pub fn load_dir(dir: &Path) -> Result<Vec<Baseline>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let is_baseline = path.extension().is_some_and(|ext| ext == "txt")
            && path.file_name().is_some_and(|name| name != TRIAGE_FILE);
        if is_baseline {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let exe = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| format!("{}: not a UTF-8 name", path.display()))?;
            Baseline::load(path, exe)?.ok_or_else(|| format!("{}: vanished", path.display()))
        })
        .collect()
}

/// The state of [`Baseline::parse`] between lines.
struct Parser {
    /// The 1-based number of the line being read.
    line: usize,
    wine_version: Option<String>,
    /// The components; the version goes in at the end.
    header: Header,
    /// The section being read.
    section: Option<Section>,
    /// Every section so far, with the line it starts on.
    sections: BTreeMap<(PeDir, String), usize>,
    entries: BTreeMap<TestId, Outcome>,
}

/// The section [`Parser`] reads.
struct Section {
    leg: PeDir,
    unit: String,
    /// The line it starts on.
    line: usize,
    keys: usize,
    /// Whether its key is `*`.
    whole: bool,
}

impl Parser {
    fn read(&mut self, line: &str) -> Result<(), String> {
        if line.is_empty() {
            return Ok(());
        }
        if let Some(comment) = line.strip_prefix('#') {
            return self.comment(comment);
        }
        if let Some(inside) = line.strip_prefix('[') {
            return self.start_section(inside);
        }
        self.entry(line)
    }

    fn comment(&mut self, comment: &str) -> Result<(), String> {
        if self.section.is_some() {
            return Err(String::from(
                "a comment after the first section; the header holds them all",
            ));
        }
        if let Some(version) = comment.strip_prefix(" wine: ") {
            if self.wine_version.is_some() {
                return Err(String::from("a second `# wine:` line"));
            }
            if version.is_empty() || version.contains(char::is_whitespace) {
                return Err(format!("{version:?} is not a Wine version"));
            }
            self.wine_version = Some(version.to_owned());
        } else if let Some(component) = comment.strip_prefix(" component: ") {
            let (name, commit) = parse_component(component)?;
            self.header.add_component(name, commit)?;
        } else if ["wine:", "component:"]
            .iter()
            .any(|field| comment.trim_start().starts_with(field))
        {
            return Err(String::from(
                "a header field is `# wine: <version>` or `# component: <name>=<commit>`, \
                 with one space after `#` and after the colon",
            ));
        }
        Ok(())
    }

    fn start_section(&mut self, inside: &str) -> Result<(), String> {
        self.close_section()?;
        let inside = inside
            .strip_suffix(']')
            .ok_or_else(|| String::from("a section line must end in `]`"))?;
        let tokens: Vec<&str> = inside.split(' ').collect();
        let [leg, unit] = tokens[..] else {
            return Err(format!("expected `[<leg> <unit>]`, found [{inside}]"));
        };
        let leg = PeDir::parse(leg).ok_or_else(|| format!("unknown leg {leg:?}"))?;
        if !is_ident(unit) {
            return Err(format!("unit {unit:?} is not a C name"));
        }
        let line = self.line;
        if let Some(first) = self.sections.insert((leg, unit.to_owned()), line) {
            return Err(format!(
                "section [{} {unit}] again, first on line {first}",
                leg.as_str()
            ));
        }
        self.section = Some(Section {
            leg,
            unit: unit.to_owned(),
            line,
            keys: 0,
            whole: false,
        });
        Ok(())
    }

    fn entry(&mut self, line: &str) -> Result<(), String> {
        let section = self
            .section
            .as_mut()
            .ok_or_else(|| String::from("a key line before any section"))?;
        let tokens: Vec<&str> = line.split(' ').collect();
        let (&key, outcome) = tokens
            .split_first()
            .ok_or_else(|| String::from("an empty key line"))?;
        let key = Key::parse(key).ok_or_else(|| format!("{key:?} is not a key"))?;
        let outcome = Outcome::parse(outcome)?;
        let whole = key == Key::Whole;
        if section.keys > 0 && (whole || section.whole) {
            return Err(String::from(
                "a `*` key must be the only key of its section",
            ));
        }
        let id = TestId {
            leg: section.leg,
            unit: section.unit.clone(),
            key,
        };
        if self.entries.contains_key(&id) {
            return Err(format!("key {} again in this section", id.key));
        }
        self.entries.insert(id, outcome);
        section.keys += 1;
        section.whole = whole;
        Ok(())
    }

    /// Check that the section that ends now has a key.
    fn close_section(&mut self) -> Result<(), String> {
        match self.section.take() {
            Some(section) if section.keys == 0 => {
                Err(format!("the section of line {} has no keys", section.line))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
