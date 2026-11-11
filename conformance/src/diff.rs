//! Compares the outcomes of a run of one executable with its baseline, key
//! by key, for a person to read after a fork sync.
//!
//! Each difference is a [`Delta`] of one class:
//!
//! | Delta | Regression | Stale baseline |
//! |---|---|---|
//! | status | worse [`Status::rank`], or crash and stall swapped | better rank |
//! | failures, sites | more | fewer |
//! | skipped | more: coverage lost | fewer |
//! | todo successes | | any change |
//! | key | in the baseline, not listed by the run; new with a crash or stall | new with another status |
//!
//! A todo that starts or stops succeeding is stale either way: the check
//! inside the todo block is Wine's to fix, and the change only means the
//! baseline describes another build.
//!
//! A crash or stall counts only what ran before it ([`Status::is_truncated`]),
//! so counts are not compared between a truncated outcome and a full one.
//! Between two truncated outcomes, more failures or more sites are a
//! regression: whatever ran before the crash failed more. Fewer of either,
//! and any change of the skips or todo successes, are listed as
//! information that does not change the verdict: they may only mean that
//! the crash moved earlier. A change of status between them (crash and
//! stall swapped) still gates. A key the triage marks flaky is reported but gates
//! only on a new crash or stall. The header's differences are reported and
//! never gate: after a fork sync they are expected.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Write as _},
};

use crate::{
    baseline::Baseline,
    header::{self, Header, HeaderChange},
    model::PeDir,
    outcome::{Outcome, Status, TestId},
    triage::Triage,
};

/// Which tests a run covered.
pub enum Scope {
    /// Every test of the executable.
    All,
    /// Every test of one leg.
    Leg(PeDir),
    /// These tests only, for a run of selected keys.
    Tests(BTreeSet<TestId>),
}

impl Scope {
    /// Whether the scope covers `id`.
    #[must_use]
    pub fn contains(&self, id: &TestId) -> bool {
        match self {
            Self::All => true,
            Self::Leg(leg) => id.leg == *leg,
            Self::Tests(ids) => ids.contains(id),
        }
    }
}

/// What a diff, or a run as a whole, says; ordered from best to worst, so
/// the worst of several is their `max`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Verdict {
    /// No change gates: every key in scope has its baseline's outcome, or
    /// differs only where the diff does not gate (a flaky key without a new
    /// crash or stall, fewer failures or other count changes between two
    /// truncated outcomes).
    Match,
    /// Something improved or is new: the baseline is behind, record again.
    Stale,
    /// Results changed between two runs of one build; the caller decides
    /// this, the diff never does.
    Unstable,
    /// Something got worse.
    Regression,
}

/// Whether a [`Delta`] makes the baseline stale or is a regression.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Class {
    Stale,
    Regression,
}

/// One difference between a key's baseline outcome and its run.
#[derive(PartialEq, Eq, Debug)]
pub enum Delta {
    /// The run listed the key, with this status; the baseline lacks it.
    New(Status),
    /// The baseline has the key; the run did not list it.
    Dropped,
    Status {
        from: Status,
        to: Status,
    },
    Failed {
        from: u32,
        to: u32,
    },
    Sites {
        from: u32,
        to: u32,
    },
    Skipped {
        from: u32,
        to: u32,
    },
    TodoSucceeded {
        from: u32,
        to: u32,
    },
}

impl Delta {
    /// The class of the difference, as the module's table gives it.
    #[must_use]
    pub const fn class(&self) -> Class {
        match *self {
            Self::New(status) => {
                if status.is_truncated() {
                    Class::Regression
                } else {
                    Class::Stale
                }
            }
            Self::TodoSucceeded { .. } => Class::Stale,
            Self::Dropped => Class::Regression,
            Self::Status { from, to } => {
                if to.rank() < from.rank() {
                    Class::Stale
                } else {
                    Class::Regression
                }
            }
            Self::Failed { from, to } | Self::Sites { from, to } | Self::Skipped { from, to } => {
                if to < from {
                    Class::Stale
                } else {
                    Class::Regression
                }
            }
        }
    }

    /// Whether the difference is a crash or stall the baseline did not
    /// have, a new key's included, which gates even on a flaky key.
    #[must_use]
    pub const fn is_new_crash_or_stall(&self) -> bool {
        matches!(
            self,
            Self::Status {
                to: Status::Crash | Status::Stall,
                ..
            } | Self::New(Status::Crash | Status::Stall)
        )
    }

    /// Whether the difference is one of the four counts.
    #[must_use]
    pub const fn is_count(&self) -> bool {
        matches!(
            self,
            Self::Failed { .. }
                | Self::Sites { .. }
                | Self::Skipped { .. }
                | Self::TodoSucceeded { .. }
        )
    }
}

impl fmt::Display for Delta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::New(status) => write!(f, "new key ({status})"),
            Self::Dropped => f.write_str("not listed by the run"),
            Self::Status { from, to } => write!(f, "status {from} -> {to}"),
            Self::Failed { from, to } => write!(f, "failed {from} -> {to}"),
            Self::Sites { from, to } => write!(f, "sites {from} -> {to}"),
            Self::Skipped { from, to } => write!(f, "skipped {from} -> {to}"),
            Self::TodoSucceeded { from, to } => write!(f, "todo_succeeded {from} -> {to}"),
        }
    }
}

/// A key whose run differs from its baseline.
pub struct Change {
    pub id: TestId,
    /// Its baseline outcome; `None` for a new key.
    pub baseline: Option<Outcome>,
    /// Its run's outcome; `None` for a key the run did not list.
    pub run: Option<Outcome>,
    pub deltas: Vec<Delta>,
    /// Whether the triage marks it flaky.
    pub flaky: bool,
}

impl Change {
    /// The class the change gates with: that of its worst delta, or, for a
    /// flaky key, regression on a new crash or stall and `None` otherwise.
    /// When both outcomes are truncated, only more failures or sites gate
    /// among the counts; the other count deltas are information and have no
    /// class, and a change with nothing else gates with `None`.
    #[must_use]
    pub fn class(&self) -> Option<Class> {
        let both_truncated = matches!(
            (self.baseline, self.run),
            (Some(before), Some(after))
                if before.status.is_truncated() && after.status.is_truncated()
        );
        let mut gating = self.deltas.iter().filter(|delta| {
            !both_truncated
                || !delta.is_count()
                || matches!(
                    delta,
                    Delta::Failed { from, to } | Delta::Sites { from, to } if to > from
                )
        });
        if self.flaky {
            return gating
                .any(Delta::is_new_crash_or_stall)
                .then_some(Class::Regression);
        }
        gating.map(Delta::class).max()
    }

    /// The label of the change in [`Report::text`] and the slot of its
    /// count: `REGRESSION`, `stale`, `flaky` for a flaky key that does not
    /// gate, and `info` for any other change that does not gate.
    fn label(&self) -> (&'static str, usize) {
        match self.class() {
            Some(Class::Regression) => ("REGRESSION", 0),
            Some(Class::Stale) => ("stale", 1),
            None if self.flaky => ("flaky", 2),
            None => ("info", 3),
        }
    }
}

/// The diff of a run with a baseline.
pub struct Report {
    /// The executable's name.
    pub exe: String,
    /// How the run's header differs from the baseline's.
    pub header: Vec<HeaderChange>,
    /// The keys that differ, regressions first, then stale keys, then flaky
    /// keys that do not gate, then the other changes that do not gate, each
    /// by [`TestId`].
    pub changes: Vec<Change>,
    /// The keys in scope that match.
    pub matched: usize,
}

impl Report {
    /// The verdict of the changes: the class of the worst one, or
    /// [`Verdict::Match`].
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        self.changes
            .iter()
            .filter_map(Change::class)
            .max()
            .map_or(Verdict::Match, |class| match class {
                Class::Stale => Verdict::Stale,
                Class::Regression => Verdict::Regression,
            })
    }

    /// The report as text: the header's differences, one line per change
    /// with both outcomes, and a count line.
    #[must_use]
    pub fn text(&self) -> String {
        let mut text = String::new();
        for change in &self.header {
            let _ = writeln!(text, "{}: header: {change}", self.exe);
        }
        let mut counts = [0usize; 4];
        for change in &self.changes {
            let (label, slot) = change.label();
            counts[slot] += 1;
            let deltas: Vec<String> = change.deltas.iter().map(ToString::to_string).collect();
            let side = |outcome: Option<Outcome>| {
                outcome.map_or_else(|| String::from("none"), |outcome| outcome.to_string())
            };
            let _ = writeln!(
                text,
                "{label} {} {}: {}\n  baseline {}\n  run      {}",
                self.exe,
                change.id,
                deltas.join(", "),
                side(change.baseline),
                side(change.run),
            );
        }
        let [regressions, stale, flaky, info] = counts;
        let _ = writeln!(
            text,
            "{}: {regressions} regressions, {stale} stale, {flaky} flaky, {info} info, {} match",
            self.exe, self.matched
        );
        text
    }
}

/// Compare the `results` of a run of `exe` over `scope`, on the build
/// `header`, with its `baseline`, or with an empty one when it has none.
///
/// Only keys in `scope` count: a baseline key outside it is not dropped, and
/// a result outside it is ignored.
#[must_use]
pub fn diff(
    exe: &str,
    baseline: Option<&Baseline>,
    header: &Header,
    results: &BTreeMap<TestId, Outcome>,
    scope: &Scope,
    triage: &Triage,
) -> Report {
    let empty = BTreeMap::new();
    let recorded = baseline.map_or(&empty, |baseline| &baseline.entries);
    let ids: BTreeSet<&TestId> = recorded
        .keys()
        .chain(results.keys())
        .filter(|id| scope.contains(id))
        .collect();
    let mut changes = Vec::new();
    let mut matched = 0;
    for id in ids {
        let before = recorded.get(id).copied();
        let after = results.get(id).copied();
        let deltas = match (before, after) {
            (Some(before), Some(after)) => deltas(&before, &after),
            (None, Some(after)) => vec![Delta::New(after.status)],
            (_, None) => vec![Delta::Dropped],
        };
        if deltas.is_empty() {
            matched += 1;
            continue;
        }
        changes.push(Change {
            id: id.clone(),
            baseline: before,
            run: after,
            deltas,
            flaky: triage.is_flaky(exe, &id.unit, &id.key),
        });
    }
    changes.sort_by(|a, b| {
        let rank = |change: &Change| change.label().1;
        rank(a).cmp(&rank(b)).then_with(|| a.id.cmp(&b.id))
    });
    Report {
        exe: exe.to_owned(),
        header: baseline.map_or_else(Vec::new, |baseline| {
            header::changes(&baseline.header, header)
        }),
        changes,
        matched,
    }
}

/// The differences between a key's baseline outcome and its run.
fn deltas(before: &Outcome, after: &Outcome) -> Vec<Delta> {
    let mut deltas = Vec::new();
    if before.status != after.status {
        deltas.push(Delta::Status {
            from: before.status,
            to: after.status,
        });
    }
    let comparable = before.status.is_truncated() == after.status.is_truncated();
    if !comparable {
        return deltas;
    }
    let (from, to) = (before.failed, after.failed);
    if from != to {
        deltas.push(Delta::Failed { from, to });
    }
    let (from, to) = (before.sites, after.sites);
    if from != to {
        deltas.push(Delta::Sites { from, to });
    }
    let (from, to) = (before.skipped, after.skipped);
    if from != to {
        deltas.push(Delta::Skipped { from, to });
    }
    let (from, to) = (before.todo_succeeded, after.todo_succeeded);
    if from != to {
        deltas.push(Delta::TodoSucceeded { from, to });
    }
    deltas
}

#[cfg(test)]
mod tests;
