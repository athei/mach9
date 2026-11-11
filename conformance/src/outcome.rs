//! The result of one test: its [`Outcome`], keyed by its [`TestId`], as the
//! output parser reads it from a run and the baseline stores it.
//!
//! A test is one key of a wrapped unit, or a whole unit, in one leg (a PE
//! directory). The executable is not part of the id: each executable has a
//! baseline of its own.

use std::fmt;

use crate::model::{PeDir, is_ident};

/// How a test's process tree ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// The root process printed its summary, exited with its failure count
    /// and no failure line was printed.
    Ok,
    /// As [`Self::Ok`], but at least one `Test failed:` line was printed.
    Fail,
    /// The process ended as [`Self::Ok`] or [`Self::Fail`] would, but never
    /// printed the `wt-run` marker of the key: the key did not run.
    NotReached,
    /// The root process died by a signal, printed no summary, exited with
    /// another code than its failure count, or a child process died of an
    /// unhandled exception.
    Crash,
    /// The watchdog killed the process tree.
    Stall,
}

impl Status {
    /// Every status, in the order of [`Self::rank`].
    pub const ALL: [Self; 5] = [
        Self::Ok,
        Self::Fail,
        Self::NotReached,
        Self::Crash,
        Self::Stall,
    ];

    /// The spelling in baselines and resume files.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Fail => "fail",
            Self::NotReached => "not-reached",
            Self::Crash => "crash",
            Self::Stall => "stall",
        }
    }

    /// The status [`Self::as_str`] spells `text`, if any.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == text)
    }

    /// How bad the status is, from 0 for [`Self::Ok`] up. A key that did
    /// not run is worse than one that failed, since it tests nothing.
    /// [`Self::Crash`] and [`Self::Stall`] share the worst rank: neither is
    /// better than the other.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Fail => 1,
            Self::NotReached => 2,
            Self::Crash | Self::Stall => 3,
        }
    }

    /// Whether the process tree ended before the test did, so its counts
    /// cover only the part that ran.
    #[must_use]
    pub const fn is_truncated(self) -> bool {
        matches!(self, Self::Crash | Self::Stall)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which part of a unit a test runs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Key {
    /// The whole unit, for a unit that runs as a whole; spelled `*`.
    Whole,
    /// The key `winetest_selected()` of `include/wine/test.h` receives: a
    /// test function's name, with the prefix of its dispatch helper.
    Function(String),
}

impl Key {
    /// The key spelled `text`: `*` for [`Self::Whole`], else a function key
    /// as `wine-macros` generates it: a C name, or a dispatch prefix (a C
    /// name and `:`) and a C name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text == "*" {
            return Some(Self::Whole);
        }
        let valid = match text.split_once(':') {
            Some((prefix, name)) => is_ident(prefix) && is_ident(name),
            None => is_ident(text),
        };
        valid.then(|| Self::Function(text.to_owned()))
    }

    /// The spelling [`Self::parse`] reads.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Whole => "*",
            Self::Function(key) => key,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One test of an executable: its leg, unit and key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TestId {
    pub leg: PeDir,
    /// The `START_TEST` name, the argument that selects the unit.
    pub unit: String,
    pub key: Key,
}

impl fmt::Display for TestId {
    /// `<leg> <unit>/<key>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}/{}", self.leg.as_str(), self.unit, self.key)
    }
}

/// What one run of a test gave: its status and four counts over every
/// process of its tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Outcome {
    pub status: Status,
    /// The `Test failed:` lines.
    pub failed: u32,
    /// The distinct `file:line` locations of the `Test failed:` lines.
    pub sites: u32,
    /// The `Test succeeded inside todo block:` lines.
    pub todo_succeeded: u32,
    /// The skips the summary lines count, and those that
    /// `WINETEST_MUTE_THRESHOLD` silenced.
    pub skipped: u32,
}

impl Outcome {
    /// The field names of the counts, in the order [`fmt::Display`] writes
    /// them.
    const COUNTS: [&str; 4] = ["failed", "sites", "todo_succeeded", "skipped"];

    /// Read the tokens [`fmt::Display`] writes: the status, then
    /// `failed=<n>`, `sites=<n>`, `todo_succeeded=<n>` and `skipped=<n>` in
    /// this order.
    ///
    /// # Errors
    ///
    /// Returns a message for a missing, unknown or extra token, a count out
    /// of order, and a count that is not a decimal `u32`.
    pub fn parse(tokens: &[&str]) -> Result<Self, String> {
        let Some((&status, counts)) = tokens.split_first() else {
            return Err(String::from("no status"));
        };
        let status = Status::parse(status).ok_or_else(|| format!("unknown status {status:?}"))?;
        if let Some(extra) = counts.get(Self::COUNTS.len()) {
            return Err(format!("unexpected {extra:?} after the counts"));
        }
        let mut values = [0u32; 4];
        for (index, name) in Self::COUNTS.iter().enumerate() {
            let token = counts
                .get(index)
                .ok_or_else(|| format!("missing {name}=<n>"))?;
            let value = token
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('='))
                .ok_or_else(|| format!("expected {name}=<n>, found {token:?}"))?;
            values[index] =
                parse_count(value).ok_or_else(|| format!("{name} {value:?} is not a count"))?;
        }
        let [failed, sites, todo_succeeded, skipped] = values;
        Ok(Self {
            status,
            failed,
            sites,
            todo_succeeded,
            skipped,
        })
    }
}

impl fmt::Display for Outcome {
    /// `<status> failed=<n> sites=<n> todo_succeeded=<n> skipped=<n>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed={} sites={} todo_succeeded={} skipped={}",
            self.status, self.failed, self.sites, self.todo_succeeded, self.skipped
        )
    }
}

/// A decimal count without sign or leading zeros, as `{}` writes a `u32`.
fn parse_count(text: &str) -> Option<u32> {
    let canonical = text.bytes().all(|b| b.is_ascii_digit())
        && !text.is_empty()
        && (text == "0" || !text.starts_with('0'));
    canonical.then(|| text.parse().ok()).flatten()
}

#[cfg(test)]
mod tests;
