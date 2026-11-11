//! Reads the output of a Wine test's process tree: the run of one key
//! ([`parse_run`]) or the listing of a unit's keys ([`parse_listing`]).
//!
//! The input is stdout and stderr merged, from the test process and every
//! child it starts, with the lines of other processes (wineserver,
//! `compatdb:`, `fixme:` and `err:`) in between. Every format below is the
//! one `include/wine/test.h` of the Wine fork prints:
//!
//! - A check prints `<file>:<line>:<elapsed> <type><contexts><message>`
//!   (`winetest_print_location`, `winetest_print_context`). `<file>` is the
//!   source's base name, `<elapsed>` is empty unless `WINETEST_TIME` is set
//!   and then `%.3f` seconds, and each context is `<text>: `. The types read
//!   here are `Test failed: `, `Test succeeded inside todo block: `, the
//!   flaky ones `Test marked flaky: ` and `Test succeeded inside flaky todo
//!   block: ` (ignored), `Tests skipped: ` and the line of
//!   `winetest_wait_child_process`, `unhandled exception %08x in child
//!   process %04x`.
//! - The summary of a process, from `run_test`:
//!   `%04x:<unit>:<elapsed> %d tests executed (%d marked as todo, %d as
//!   flaky, %d failure[s]), %d skipped.`, after
//!   `%04x:<unit>:<elapsed> Silenced %d todos, %d skips and %d traces.` when
//!   `WINETEST_MUTE_THRESHOLD` silenced any.
//! - `winetest_selected` prints `%04x:wt-list:<key>` and `%04x:wt-run:<key>`.
//!
//! A line may end in `\r\n`, since the tests write through the CRT in text
//! mode, and may start with the escape codes of `WINETEST_COLOR`; both are
//! dropped. A check message without its own newline runs into the next line
//! and hides it, and a line of another process can split a check line; the
//! parser counts neither line, but the root's summary still counts such a
//! failure, which [`parse_run`] turns into [`Status::Fail`].

use std::collections::BTreeSet;

use crate::outcome::{Key, Outcome, Status};

/// How the root process of a test ended, as the caller saw it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// It exited with this code, the low 8 bits of its Windows exit code.
    Exited(i32),
    /// A signal killed it.
    Signaled(i32),
    /// The caller's watchdog killed the process tree.
    Stalled,
}

/// Why a run is a [`Status::Crash`].
#[derive(PartialEq, Eq, Debug)]
pub enum Crash {
    /// The root process died by this signal.
    Signal(i32),
    /// The root process printed no summary: it ended before `run_test` of
    /// `test.h` returned.
    NoSummary,
    /// The root process exited with `code`, not with the failure count of
    /// its summary capped at 255, which `run_test` returns: it died after
    /// its summary, for example of an unhandled exception, whose code Wine
    /// passes on.
    ///
    /// Only the low 8 bits of the exit status reach the caller, so a crash
    /// after the summary whose code has the failure count as its low byte
    /// (an exception code `0xc0000005` after five failures) is not seen and
    /// the run reads as [`Status::Fail`] or [`Status::Ok`].
    ExitCode { code: i32, failures: u32 },
    /// A child process died of an unhandled exception; the line of
    /// `winetest_wait_child_process` that says so.
    Child(String),
    /// A process other than the root printed a `wt-run` marker but no
    /// summary: it ended before `run_test` returned, for example killed by
    /// a signal, which leaves no line in the parent's output. The process
    /// id.
    ChildNoSummary(u32),
}

/// What a run of one key, or of a whole unit, gave.
pub struct Run {
    pub outcome: Outcome,
    /// Why the outcome is a crash, when it is one.
    pub crash: Option<Crash>,
    /// The process id of the root process, when the parser found it.
    pub root: Option<u32>,
}

/// The keys a unit's listing printed.
pub struct Listing {
    /// The keys in the order of their first `wt-list` line, each once.
    pub keys: Vec<String>,
    /// The `Test failed:` and `Test succeeded inside todo block:` lines the
    /// listing printed.
    pub failures: Vec<String>,
}

impl Listing {
    /// Whether the listing can be taken as the unit's keys: the setup code
    /// that runs before the keys printed no failure, so it took the path a
    /// clean run takes.
    #[must_use]
    pub const fn is_trustworthy(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Why a listing gave no keys to run.
#[derive(PartialEq, Eq, Debug)]
pub enum ListingProblem {
    /// No process printed a summary for the unit.
    NoSummary,
    /// The listing process printed a `wt-list` line with this text, which
    /// is no function key.
    BadKey(String),
    /// It printed its summary but no `wt-list` line.
    NoKeys,
}

/// A listing that cannot be used, with the lines that may say why.
#[derive(Debug)]
pub struct ListingError {
    pub problem: ListingProblem,
    /// The `Tests skipped:` lines.
    pub skips: Vec<String>,
    /// The `Test failed:` and `Test succeeded inside todo block:` lines.
    pub failures: Vec<String>,
}

/// Read the output of the run of `key` of `unit`, whose root process ended
/// as `ending`.
///
/// `output` is the one stream of stdout and stderr of the whole process
/// tree, merged in the order the bytes arrived, decoded lossily as UTF-8.
/// The results are only valid when the test ran with `WINETEST_DEBUG`
/// unset or not `0` (at 0 `run_test` prints no summary at all), and with
/// `WINETEST_REPORT_FLAKY` and `WINETEST_PLATFORM` unset (the first adds
/// the flaky failures to the exit code, the second changes which
/// `todo_wine` checks are todo); `WINETEST_COLOR` is allowed.
///
/// The root process of a key is the one that printed the first `wt-run`
/// line of the key. For a whole unit, and for a key without that line, it
/// is the process of the last summary line of the unit: a process prints its
/// summary after its test function returned, and a test waits for its
/// children in it (`winetest_wait_child_process`), so a parent's summary
/// comes after its children's. A child the parent does not wait for, whose
/// summary comes after the parent's, would be taken as the root.
///
/// The status, in this order:
///
/// - [`Status::Stall`] for [`Ending::Stalled`];
/// - [`Status::Crash`] when the root died by a signal, has no summary,
///   exited with another code than its summary's failures (at most 255), a
///   child died of an unhandled exception, or another process printed a
///   `wt-run` line but no summary ([`Crash`]);
/// - [`Status::NotReached`] for a key without its `wt-run` line;
/// - [`Status::Fail`] when any process printed a `Test failed:` line, or
///   the root's summary counts more failures than the failure and todo
///   success lines explain, as when a line was lost;
/// - [`Status::Ok`].
///
/// The counts are lines of every process of the tree, so nothing is counted
/// twice: a parent adds its child's failures to its own summary and exit
/// code (`winetest_wait_child_process`), but the child prints its own
/// failure lines and the parent none for them. `skipped` adds up the
/// summary lines of all processes, since a parent does not add its
/// children's skips, and the skips of their `Silenced` lines, so it does not
/// depend on `WINETEST_MUTE_THRESHOLD`; a process without a summary adds
/// none.
#[must_use]
pub fn parse_run(output: &str, unit: &str, key: &Key, ending: Ending) -> Run {
    let lines: Vec<Line<'_>> = output.split('\n').map(Line::parse).collect();
    let marker = match key {
        Key::Whole => None,
        Key::Function(key) => lines.iter().find_map(|line| match line {
            Line::Run { pid, key: printed } if printed == key => Some(*pid),
            _ => None,
        }),
    };
    let summary = root_summary(&lines, unit, marker);
    let root = marker.or_else(|| summary.map(|summary| summary.pid));
    let mut failed = 0;
    let mut sites = BTreeSet::new();
    let mut todo_succeeded = 0;
    let mut skipped = 0;
    let mut child_crash = None;
    for line in &lines {
        match line {
            Line::Check {
                file,
                line,
                kind: CheckKind::Failed,
                ..
            } => {
                failed += 1;
                sites.insert((*file, *line));
            }
            Line::Check {
                kind: CheckKind::TodoSucceeded,
                ..
            } => todo_succeeded += 1,
            Line::Check {
                kind: CheckKind::ChildCrash,
                text,
                ..
            } => {
                child_crash.get_or_insert_with(|| (*text).to_owned());
            }
            Line::Summary(summary) => skipped += summary.skipped,
            Line::Silenced { skips } => skipped += skips,
            _ => {}
        }
    }
    let crash = match ending {
        Ending::Stalled => None,
        Ending::Signaled(signal) => Some(Crash::Signal(signal)),
        Ending::Exited(code) => match summary {
            None => Some(Crash::NoSummary),
            Some(summary) if u32::try_from(code) != Ok(summary.failures.min(255)) => {
                Some(Crash::ExitCode {
                    code,
                    failures: summary.failures,
                })
            }
            Some(_) => child_crash
                .map(Crash::Child)
                .or_else(|| unfinished_child(&lines, root).map(Crash::ChildNoSummary)),
        },
    };
    let unexplained = summary.is_some_and(|summary| summary.failures > failed + todo_succeeded);
    let status = if matches!(ending, Ending::Stalled) {
        Status::Stall
    } else if crash.is_some() {
        Status::Crash
    } else if matches!(key, Key::Function(_)) && marker.is_none() {
        Status::NotReached
    } else if failed > 0 || unexplained {
        Status::Fail
    } else {
        Status::Ok
    };
    Run {
        outcome: Outcome {
            status,
            failed,
            sites: u32::try_from(sites.len()).unwrap_or(u32::MAX),
            todo_succeeded,
            skipped,
        },
        crash,
        root,
    }
}

/// Read the output of the listing of `unit`, a run with `WT_LIST` set.
///
/// `output` is lossily decoded text of one merged stream, as for
/// [`parse_run`], under the same environment contract.
///
/// The listing process is the one of the last summary line of the unit, as
/// the root of a whole unit is for [`parse_run`]; only its `wt-list` lines
/// count. Each of them must hold a function key as [`Key::parse`] reads it,
/// so a line of another process glued to a key's line is an error rather
/// than a key.
///
/// # Errors
///
/// Returns [`ListingProblem::NoSummary`] when no process printed a summary
/// for the unit, else [`ListingProblem::BadKey`] for a `wt-list` line of
/// the listing process that holds no function key, else
/// [`ListingProblem::NoKeys`] when it printed no `wt-list` line, each with
/// the skip and failure lines of the output.
pub fn parse_listing(output: &str, unit: &str) -> Result<Listing, ListingError> {
    let lines: Vec<Line<'_>> = output.split('\n').map(Line::parse).collect();
    let root = root_summary(&lines, unit, None).map(|summary| summary.pid);
    let mut keys: Vec<String> = Vec::new();
    let mut bad = None;
    let mut skips = Vec::new();
    let mut failures = Vec::new();
    for line in &lines {
        match line {
            Line::List { pid, key } if Some(*pid) == root => {
                if !matches!(Key::parse(key), Some(Key::Function(_))) {
                    bad.get_or_insert_with(|| (*key).to_owned());
                } else if !keys.iter().any(|known| known == key) {
                    keys.push((*key).to_owned());
                }
            }
            Line::Check {
                kind: CheckKind::Failed | CheckKind::TodoSucceeded,
                text,
                ..
            } => failures.push((*text).to_owned()),
            Line::Check {
                kind: CheckKind::Skipped,
                text,
                ..
            } => skips.push((*text).to_owned()),
            _ => {}
        }
    }
    let problem = if root.is_none() {
        Some(ListingProblem::NoSummary)
    } else if let Some(key) = bad {
        Some(ListingProblem::BadKey(key))
    } else if keys.is_empty() {
        Some(ListingProblem::NoKeys)
    } else {
        None
    };
    match problem {
        Some(problem) => Err(ListingError {
            problem,
            skips,
            failures,
        }),
        None => Ok(Listing { keys, failures }),
    }
}

/// The first process other than `root` that printed a `wt-run` line and no
/// summary. A summary of any unit counts: a child started for another unit
/// prints that unit's summary.
fn unfinished_child(lines: &[Line<'_>], root: Option<u32>) -> Option<u32> {
    let mut started = BTreeSet::new();
    let mut summarized = BTreeSet::new();
    for line in lines {
        match line {
            Line::Run { pid, .. } => {
                started.insert(*pid);
            }
            Line::Summary(summary) => {
                summarized.insert(summary.pid);
            }
            _ => {}
        }
    }
    started
        .difference(&summarized)
        .copied()
        .find(|pid| Some(*pid) != root)
}

/// The summary of the root process of `unit`: the last summary of the unit
/// by `pid`, or, without a `pid`, the last summary of the unit.
///
/// Without a `pid` this relies on a parent waiting for its children before
/// it prints its own summary; a child the parent does not wait for, whose
/// summary comes last, is taken as the root.
fn root_summary<'l>(lines: &'l [Line<'_>], unit: &str, pid: Option<u32>) -> Option<&'l Summary> {
    lines.iter().rev().find_map(|line| match line {
        Line::Summary(summary)
            if summary.unit == unit && pid.is_none_or(|pid| pid == summary.pid) =>
        {
            Some(summary)
        }
        _ => None,
    })
}

/// A summary line, `%04x:<unit>:<elapsed> %d tests executed (...)`.
struct Summary {
    pid: u32,
    unit: String,
    /// The failures and todo successes of the process, and the failures it
    /// added for its children: what `run_test` returns, uncapped.
    failures: u32,
    /// The skips it printed; its `Silenced` line counts those it did not.
    skipped: u32,
}

/// What a check line of `winetest_print_context` is.
enum CheckKind {
    Failed,
    TodoSucceeded,
    Skipped,
    ChildCrash,
    /// A trace, a todo or flaky check, a child's failure count, or anything
    /// else the parser does not count.
    Other,
}

/// One line of the output, as far as the parser reads it.
enum Line<'a> {
    /// `%04x:wt-run:<key>`.
    Run {
        pid: u32,
        key: &'a str,
    },
    /// `%04x:wt-list:<key>`.
    List {
        pid: u32,
        key: &'a str,
    },
    Summary(Summary),
    /// `%04x:<unit>:<elapsed> Silenced %d todos, %d skips and %d traces.`
    Silenced {
        skips: u32,
    },
    /// `<file>:<line>:<elapsed> <type>...`, with the line's text.
    Check {
        file: &'a str,
        line: u32,
        kind: CheckKind,
        text: &'a str,
    },
    Other,
}

impl<'a> Line<'a> {
    fn parse(raw: &'a str) -> Self {
        let text = strip_colors(raw.strip_suffix('\r').unwrap_or(raw));
        Self::process_line(text)
            .or_else(|| Self::check(text))
            .unwrap_or(Self::Other)
    }

    /// A line that starts with a process id: a selection marker, a summary
    /// or a `Silenced` line.
    fn process_line(text: &'a str) -> Option<Self> {
        let (pid, rest) = text.split_once(':')?;
        if pid.len() < 4
            || !pid
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let pid = u32::from_str_radix(pid, 16).ok()?;
        if let Some(key) = rest.strip_prefix("wt-run:") {
            return Some(Self::Run { pid, key });
        }
        if let Some(key) = rest.strip_prefix("wt-list:") {
            return Some(Self::List { pid, key });
        }
        let (unit, rest) = rest.split_once(':')?;
        if !crate::model::is_ident(unit) {
            return None;
        }
        let rest = strip_elapsed(rest)?;
        if let Some(rest) = rest.strip_prefix("Silenced ") {
            let (_, rest) = number(rest)?;
            let (skips, rest) = number(rest.strip_prefix(" todos, ")?)?;
            let (_, rest) = number(rest.strip_prefix(" skips and ")?)?;
            return (rest == " traces.").then_some(Self::Silenced { skips });
        }
        let (_, rest) = number(rest)?;
        let (_, rest) = number(rest.strip_prefix(" tests executed (")?)?;
        let (_, rest) = number(rest.strip_prefix(" marked as todo, ")?)?;
        let (failures, rest) = number(rest.strip_prefix(" as flaky, ")?)?;
        let rest = rest
            .strip_prefix(" failures), ")
            .or_else(|| rest.strip_prefix(" failure), "))?;
        let (skipped, rest) = number(rest)?;
        (rest == " skipped.").then(|| {
            Self::Summary(Summary {
                pid,
                unit: unit.to_owned(),
                failures,
                skipped,
            })
        })
    }

    /// A line of `winetest_print_location`: `<file>:<line>:<elapsed> `
    /// and the rest.
    fn check(text: &'a str) -> Option<Self> {
        let (file, rest) = text.split_once(':')?;
        if file.is_empty() || file.contains(char::is_whitespace) {
            return None;
        }
        let (line, rest) = number(rest)?;
        let message = strip_elapsed(rest.strip_prefix(':')?)?;
        let kind = if message.starts_with("Test failed: ") {
            CheckKind::Failed
        } else if message.starts_with("Test succeeded inside todo block: ") {
            CheckKind::TodoSucceeded
        } else if message.starts_with("Tests skipped: ") {
            CheckKind::Skipped
        } else if message.starts_with("unhandled exception ")
            && message.contains(" in child process ")
        {
            CheckKind::ChildCrash
        } else {
            CheckKind::Other
        };
        Some(Self::Check {
            file,
            line,
            kind,
            text,
        })
    }
}

/// `text` without the `ESC [ ... m` sequences at its start, which
/// `WINETEST_COLOR` puts before a line and, for the reset that follows a
/// message's newline, before the next one.
fn strip_colors(mut text: &str) -> &str {
    while let Some(rest) = text.strip_prefix("\x1b[") {
        let Some(end) = rest.find(|c: char| !(c.is_ascii_digit() || c == ';')) else {
            break;
        };
        if rest.as_bytes()[end] != b'm' {
            break;
        }
        text = &rest[end + 1..];
    }
    text
}

/// The text after the `<elapsed> ` that follows a location's last `:`: a
/// space when `WINETEST_TIME` is off, else `%.3f` seconds and a space.
fn strip_elapsed(text: &str) -> Option<&str> {
    if let Some(rest) = text.strip_prefix(' ') {
        return Some(rest);
    }
    let (_, rest) = number(text)?;
    let fraction = rest.strip_prefix('.')?;
    let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
    (digits == 3).then(|| fraction[3..].strip_prefix(' '))?
}

/// The decimal number at the start of `text` and the text after it.
fn number(text: &str) -> Option<(u32, &str)> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let value = text[..digits].parse().ok()?;
    Some((value, &text[digits..]))
}

#[cfg(test)]
mod tests;
