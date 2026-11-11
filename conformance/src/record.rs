//! Records the outcomes of a run into an executable's baseline.
//!
//! A record runs every key in scope once, then once more each key whose
//! outcome differs from the baseline ([`to_confirm`]). A key whose two
//! outcomes differ is unstable ([`unstable`]): it is written with the worse
//! of the two ([`worse`]): the higher-ranked status, and each count the
//! larger of the two. The baseline then holds only results of the build it
//! names, and a later run that gives either result is no regression, except
//! when one was a crash and the other a stall, which share a rank: the
//! first one's status is written, and a run with the other still gates. The
//! report lists the key. [`merge`] then writes the outcomes in scope over
//! the old baseline.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Write as _},
};

use crate::{
    baseline::Baseline,
    diff::Scope,
    header::{self, Header, HeaderChange},
    outcome::{Outcome, TestId},
};

/// Why [`merge`] refuses a record.
#[derive(Debug)]
pub enum RecordError {
    /// A record of part of the tests on another build than the baseline's
    /// would leave a file that mixes two builds; record all of them.
    HeaderDiffers(Vec<HeaderChange>),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderDiffers(changes) => {
                let changes: Vec<String> = changes.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "the bundle is another build than the baseline's ({}): record every test, \
                     not part of them",
                    changes.join(", ")
                )
            }
        }
    }
}

/// A new baseline and what changed from the old one.
pub struct Merge {
    pub baseline: Baseline,
    /// Keys the old baseline lacked, with their outcome.
    pub added: Vec<(TestId, Outcome)>,
    /// Keys in scope the old baseline had and the new one does not, with
    /// their old outcome.
    pub dropped: Vec<(TestId, Outcome)>,
    /// Keys with another outcome, old then new.
    pub changed: Vec<(TestId, Outcome, Outcome)>,
    /// Keys in scope whose two runs differed, with the worse outcome they
    /// were written with; they are in `added` or `changed` as well when the
    /// old baseline differs.
    pub unstable: Vec<(TestId, Outcome)>,
}

impl Merge {
    /// What changed, one line per key, and a count line.
    #[must_use]
    pub fn text(&self) -> String {
        let exe = &self.baseline.exe;
        let mut text = String::new();
        for (id, outcome) in &self.added {
            let _ = writeln!(text, "added {exe} {id}: {outcome}");
        }
        for (id, outcome) in &self.dropped {
            let _ = writeln!(text, "dropped {exe} {id}: was {outcome}");
        }
        for (id, old, new) in &self.changed {
            let _ = writeln!(text, "changed {exe} {id}: {old} -> {new}");
        }
        for (id, outcome) in &self.unstable {
            let _ = writeln!(
                text,
                "unstable {exe} {id}: the two runs differ, wrote the worse: {outcome}"
            );
        }
        let _ = writeln!(
            text,
            "{exe}: {} added, {} dropped, {} changed, {} unstable",
            self.added.len(),
            self.dropped.len(),
            self.changed.len(),
            self.unstable.len()
        );
        text
    }
}

/// The keys of the first pass to run a second time: those whose outcome
/// differs from the `old` baseline's, a key it lacks included.
#[must_use]
pub fn to_confirm(old: Option<&Baseline>, first: &BTreeMap<TestId, Outcome>) -> BTreeSet<TestId> {
    first
        .iter()
        .filter(|(id, outcome)| old.and_then(|old| old.entries.get(*id)) != Some(*outcome))
        .map(|(id, _)| id.clone())
        .collect()
}

/// The keys of `confirm` whose two passes differ, with the outcome to write.
///
/// A key differs when its `second` outcome is not its `first`, or one pass
/// lacks it. The outcome is the [`worse`] of the two, or the one there is.
/// A key neither pass has did not run and is left out.
#[must_use]
pub fn unstable(
    confirm: &BTreeSet<TestId>,
    first: &BTreeMap<TestId, Outcome>,
    second: &BTreeMap<TestId, Outcome>,
) -> BTreeMap<TestId, Outcome> {
    confirm
        .iter()
        .filter_map(|id| {
            let outcome = match (first.get(id), second.get(id)) {
                (Some(first), Some(second)) if first == second => return None,
                (Some(first), Some(second)) => worse(*first, *second),
                (Some(only), None) | (None, Some(only)) => *only,
                (None, None) => return None,
            };
            Some((id.clone(), outcome))
        })
        .collect()
}

/// The worse of two outcomes of one key.
///
/// The status of the higher [`Status::rank`], `a`'s when they tie, and each
/// count the larger of the two, so neither outcome has a count above the
/// result.
///
/// [`Status::rank`]: crate::outcome::Status::rank
#[must_use]
pub fn worse(a: Outcome, b: Outcome) -> Outcome {
    Outcome {
        status: if b.status.rank() > a.status.rank() {
            b.status
        } else {
            a.status
        },
        failed: a.failed.max(b.failed),
        sites: a.sites.max(b.sites),
        todo_succeeded: a.todo_succeeded.max(b.todo_succeeded),
        skipped: a.skipped.max(b.skipped),
    }
}

/// The new baseline of `exe` from the `old` one and the `results` of a run
/// over `scope` on the build `header`, with each `unstable` key written
/// with its outcome from there ([`unstable`]).
///
/// Entries outside the scope stay as they are and results and unstable
/// keys outside it are ignored; an entry in scope without a result or an
/// unstable outcome is dropped. Every unstable key in scope is listed in
/// [`Merge::unstable`], whether `results` has it or not. A record of
/// [`Scope::All`] takes `header`; a smaller one keeps the old header.
///
/// # Errors
///
/// Returns [`RecordError::HeaderDiffers`] for a scope smaller than
/// [`Scope::All`] when the old baseline's header differs from `header`.
pub fn merge(
    exe: &str,
    old: Option<&Baseline>,
    header: &Header,
    results: &BTreeMap<TestId, Outcome>,
    scope: &Scope,
    unstable: &BTreeMap<TestId, Outcome>,
) -> Result<Merge, RecordError> {
    let empty = BTreeMap::new();
    let recorded = old.map_or(&empty, |old| &old.entries);
    let header = match (scope, old) {
        (Scope::All, _) | (_, None) => header.clone(),
        (_, Some(old)) => {
            let changes = header::changes(&old.header, header);
            if !changes.is_empty() {
                return Err(RecordError::HeaderDiffers(changes));
            }
            old.header.clone()
        }
    };
    let mut entries: BTreeMap<TestId, Outcome> = recorded
        .iter()
        .filter(|(id, _)| !scope.contains(id))
        .map(|(id, outcome)| (id.clone(), *outcome))
        .collect();
    for (id, outcome) in results.iter().filter(|(id, _)| scope.contains(id)) {
        entries.insert(id.clone(), *outcome);
    }
    let mut kept = Vec::new();
    for (id, outcome) in unstable.iter().filter(|(id, _)| scope.contains(id)) {
        entries.insert(id.clone(), *outcome);
        kept.push((id.clone(), *outcome));
    }
    let mut merge = Merge {
        baseline: Baseline {
            exe: exe.to_owned(),
            header,
            entries: BTreeMap::new(),
        },
        added: Vec::new(),
        dropped: Vec::new(),
        changed: Vec::new(),
        unstable: kept,
    };
    for (id, outcome) in &entries {
        match recorded.get(id) {
            None => merge.added.push((id.clone(), *outcome)),
            Some(old) if old != outcome => merge.changed.push((id.clone(), *old, *outcome)),
            Some(_) => {}
        }
    }
    merge.dropped = recorded
        .iter()
        .filter(|(id, _)| !entries.contains_key(*id))
        .map(|(id, outcome)| (id.clone(), *outcome))
        .collect();
    merge.baseline.entries = entries;
    Ok(merge)
}

#[cfg(test)]
mod tests;
