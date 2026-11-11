//! The human-owned notes on deqp-vk cases,
//! `conformance/baselines/deqp-vk/triage.txt`: per case pattern an issue
//! number and whether its results flake.
//!
//! A line is a pattern over case names ([`glob`]) and its notes; `#` starts
//! a comment, a whole line or the rest of one:
//!
//! ```text
//! dEQP-VK.synchronization.timeline_semaphore.wait.poll_signal_from_device issue=63 flaky
//! dEQP-VK.draw.renderpass.shader_layer.* issue=61  # needs layered rendering
//! ```
//!
//! The diff reports a flaky case's changes without gating on them, except
//! a new crash or stall, and `record` writes the worst of several runs of
//! it.

use std::{fs, io, path::Path};

use crate::deqp::groups::glob;

/// The name of the triage file in the expected results' directory, which
/// is no group's.
pub const TRIAGE_FILE: &str = "triage.txt";

/// The notes on the cases one pattern matches.
#[derive(PartialEq, Eq, Debug)]
pub struct Note {
    pub pattern: String,
    /// The issue that tracks them.
    pub issue: Option<u32>,
    /// Whether their results change between runs of one build.
    pub flaky: bool,
}

/// The notes of the triage file, in file order.
#[derive(Default)]
pub struct Triage {
    pub notes: Vec<Note>,
}

impl Triage {
    /// Read the triage text; `file` names it in errors.
    ///
    /// # Errors
    ///
    /// Returns `<file>:<line>: <message>` for a pattern that does not start
    /// with `dEQP-VK.`, a pattern given twice, a token other than
    /// `issue=<n>` and `flaky`, and either of them twice on one line.
    pub fn parse(text: &str, file: &str) -> Result<Self, String> {
        let mut notes: Vec<Note> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let at = |e: String| format!("{file}:{}: {e}", index + 1);
            let content = line.split_once('#').map_or(line, |(before, _)| before);
            let mut tokens = content.split_whitespace();
            let Some(pattern) = tokens.next() else {
                continue;
            };
            if !pattern.starts_with("dEQP-VK.") {
                return Err(at(format!("{pattern:?} is not a deqp-vk case pattern")));
            }
            if notes.iter().any(|note| note.pattern == pattern) {
                return Err(at(format!("{pattern} again")));
            }
            let mut note = Note {
                pattern: pattern.to_owned(),
                issue: None,
                flaky: false,
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
                        "unknown token {token:?}; a case takes `issue=<n>` and `flaky`"
                    )));
                }
            }
            notes.push(note);
        }
        Ok(Self { notes })
    }

    /// Read the triage file at `path`; no file is an empty triage.
    ///
    /// # Errors
    ///
    /// Returns a message when it cannot be read or does not parse.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, &path.display().to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Whether a pattern that marks its cases flaky matches `case`.
    #[must_use]
    pub fn is_flaky(&self, case: &str) -> bool {
        self.notes
            .iter()
            .any(|note| note.flaky && glob(&note.pattern, case))
    }
}

#[cfg(test)]
mod tests;
