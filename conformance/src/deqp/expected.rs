//! The committed expected results of one deqp-vk group,
//! `conformance/baselines/deqp-vk/<group>.txt`, and the diff of a run with
//! them.
//!
//! The file lists every case of the group that does not pass, with its
//! [`Status`]; a case of the group it does not list is expected to pass.
//! Where every case of the group under a node of the case tree has one
//! status, one line `<node>.*` stands for them all, so a group whose
//! driver lacks a feature does not list each case that needs it. Its
//! header names the build it was recorded on and the number of cases the
//! group selected. `record` writes it ([`Expected::to_text`]) and nothing
//! else should:
//!
//! ```text
//! # Expected deqp-vk results of the group draw: every case that does not pass.
//! # Written by `cargo run -p conformance -- deqp-vk record --group draw`: do not edit.
//! # component: mesa=0123456789abcdef0123456789abcdef01234567
//! # component: vk-gl-cts=89abcdef0123456789abcdef0123456789abcdef
//! # component: vulkan-loader=fedcba9876543210fedcba9876543210fedcba98
//! # cases: 1200
//!
//! dEQP-VK.draw.renderpass.depth_bias.depth_bias_triangle_list_point fail
//! dEQP-VK.draw.renderpass.multi_draw.* not-supported
//! ```
//!
//! The lines are sorted, so an unchanged result writes the same bytes. A
//! `.*` line stands only for the cases the group selects: a case the group
//! selects after a change of its patterns is not covered until the group
//! is recorded again, which its new case count shows.
//!
//! | Change | Class |
//! |---|---|
//! | worse [`Status::rank`], or crash and stall swapped | regression |
//! | better rank | stale |
//! | listed, but the group no longer selects it | stale |
//! | the group selects another number of cases than `# cases:` | stale |
//!
//! A case the triage marks flaky gates only on a new crash or stall. The
//! header's differences are reported and never gate.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Write as _},
    fs, io,
    path::{Path, PathBuf},
};

use crate::{
    deqp::triage::Triage,
    diff::{Class, Verdict},
    header::{HeaderChange, parse_component, same_commit},
};

/// The directory of the committed expected results,
/// `conformance/baselines/deqp-vk`.
#[must_use]
pub fn dir() -> PathBuf {
    crate::baseline::dir().join("deqp-vk")
}

/// How one case ended, in the order of [`Self::rank`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Status {
    /// deqp's `Pass`.
    Pass,
    /// A pass with a note: `QualityWarning`, `CompatibilityWarning`,
    /// `CapabilityWarning`.
    Warn,
    /// `Fail`, and the results deqp gives for a case it could not finish:
    /// `ResourceError`, `InternalError`, `Pending` and the `Enforce*` ones.
    Fail,
    /// `NotSupported` or `Waiver`: the driver lacks what the case needs, so
    /// it tests nothing.
    NotSupported,
    /// The process died during the case, or deqp said `Crash` or
    /// `DeviceLost`.
    Crash,
    /// The watchdog killed the process during the case, or deqp said
    /// `Timeout`.
    Stall,
}

impl Status {
    /// Every status, in the order of [`Self::rank`].
    pub const ALL: [Self; 6] = [
        Self::Pass,
        Self::Warn,
        Self::Fail,
        Self::NotSupported,
        Self::Crash,
        Self::Stall,
    ];

    /// The spelling in the expected results.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::NotSupported => "not-supported",
            Self::Crash => "crash",
            Self::Stall => "stall",
        }
    }

    /// The status [`Self::as_str`] spells `text`, if any.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == text)
    }

    /// The status of a result name deqp prints (`qpGetTestResultName`),
    /// if it is one.
    #[must_use]
    pub fn from_deqp(name: &str) -> Option<Self> {
        Some(match name {
            "Pass" => Self::Pass,
            "QualityWarning" | "CompatibilityWarning" | "CapabilityWarning" => Self::Warn,
            "Fail"
            | "ResourceError"
            | "InternalError"
            | "Pending"
            | "EnforceDefaultContext"
            | "EnforceDefaultInstance" => Self::Fail,
            "NotSupported" | "Waiver" => Self::NotSupported,
            "Crash" | "DeviceLost" => Self::Crash,
            "Timeout" => Self::Stall,
            _ => return None,
        })
    }

    /// How bad the status is, from 0 for [`Self::Pass`] up. A case that is
    /// not supported is worse than one that fails, since it tests nothing,
    /// as for the Wine suites. [`Self::Crash`] and [`Self::Stall`] share
    /// the worst rank.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Pass => 0,
            Self::Warn => 1,
            Self::Fail => 2,
            Self::NotSupported => 3,
            Self::Crash | Self::Stall => 4,
        }
    }

    /// Whether the process ended during the case.
    #[must_use]
    pub const fn is_crash_or_stall(self) -> bool {
        matches!(self, Self::Crash | Self::Stall)
    }

    /// The class of a change from `self` to `run`, `None` for no change.
    #[must_use]
    pub fn change_to(self, run: Self) -> Option<Class> {
        if self == run {
            None
        } else if run.rank() < self.rank() {
            Some(Class::Stale)
        } else {
            Some(Class::Regression)
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The expected results of one group.
#[derive(PartialEq, Eq, Debug)]
pub struct Expected {
    pub group: String,
    /// The commits of the build they were recorded on, by name: `mesa`,
    /// `vulkan-loader` and `vk-gl-cts`.
    pub components: BTreeMap<String, String>,
    /// How many cases the group selected.
    pub cases: usize,
    /// Every case that did not pass, and every `<node>.*` that stands for
    /// the cases under a node that share a status other than pass.
    pub results: BTreeMap<String, Status>,
}

/// The node a `<node>.*` key stands for, with the trailing dot: `a.b.` for
/// `a.b.*`; `None` for the key of a single case.
fn node_of(key: &str) -> Option<&str> {
    key.strip_suffix('*').filter(|node| node.ends_with('.'))
}

impl Expected {
    /// The text of the file.
    #[must_use]
    pub fn to_text(&self) -> String {
        let group = &self.group;
        let mut text = format!(
            "# Expected deqp-vk results of the group {group}: every case that does not pass.\n# \
             Written by `cargo run -p conformance -- deqp-vk record --group {group}`: do not \
             edit.\n"
        );
        for (name, commit) in &self.components {
            let _ = writeln!(text, "# component: {name}={commit}");
        }
        let _ = writeln!(text, "# cases: {}", self.cases);
        if !self.results.is_empty() {
            text.push('\n');
        }
        for (case, status) in &self.results {
            let _ = writeln!(text, "{case} {status}");
        }
        text
    }

    /// Read the text of the expected results of `group`; `file` names it in
    /// errors.
    ///
    /// The comment lines before the first case are the header: a
    /// `# component: <name>=<commit>` line per component, `# cases: <n>`
    /// once, and prose. After it come blank lines and `<case> <status>` or
    /// `<node>.* <status>` lines.
    ///
    /// # Errors
    ///
    /// Returns `<file>:<line>: <message>` for a bad or repeated component,
    /// a missing or repeated `# cases:` line, a comment after the first
    /// case, a key that does not start with `dEQP-VK.` or has a `*` other
    /// than a final `.*`, a key listed twice or under a `.*` line, `pass` or
    /// an unknown status, and the lines out of order.
    pub fn parse(text: &str, group: &str, file: &str) -> Result<Self, String> {
        let mut expected = Self {
            group: group.to_owned(),
            components: BTreeMap::new(),
            cases: 0,
            results: BTreeMap::new(),
        };
        let mut cases = None;
        let mut last: Option<&str> = None;
        // The node of the last `<node>.*` line, which sorts right before
        // every key under it.
        let mut covering: Option<&str> = None;
        for (index, line) in text.lines().enumerate() {
            let at = |e: String| format!("{file}:{}: {e}", index + 1);
            if line.is_empty() {
                continue;
            }
            if let Some(comment) = line.strip_prefix('#') {
                if last.is_some() {
                    return Err(at(String::from(
                        "a comment after the first case; the header holds them all",
                    )));
                }
                if let Some(component) = comment.strip_prefix(" component: ") {
                    let (name, commit) = parse_component(component).map_err(at)?;
                    if expected
                        .components
                        .insert(name.to_owned(), commit.to_owned())
                        .is_some()
                    {
                        return Err(at(format!("component {name} is named twice")));
                    }
                } else if let Some(count) = comment.strip_prefix(" cases: ") {
                    let count = count
                        .parse()
                        .map_err(|_| at(format!("{count:?} is not a number of cases")))?;
                    if cases.replace(count).is_some() {
                        return Err(at(String::from("a second `# cases:` line")));
                    }
                }
                continue;
            }
            let (case, status) = line
                .split_once(' ')
                .ok_or_else(|| at(format!("{line:?} is not `<case> <status>`")))?;
            let stem = node_of(case).unwrap_or(case);
            if !stem.starts_with("dEQP-VK.")
                || stem.len() == "dEQP-VK.".len() && node_of(case).is_none()
                || stem.contains(|c: char| c.is_whitespace() || c == '*')
            {
                return Err(at(format!("{case:?} is not a deqp-vk case or `<node>.*`")));
            }
            let status = Status::parse(status)
                .filter(|&status| status != Status::Pass)
                .ok_or_else(|| {
                    at(format!(
                        "{status:?} is not a status of a case that does not pass"
                    ))
                })?;
            if last.is_some_and(|last| last >= case) {
                return Err(at(format!("{case} is out of order or listed twice")));
            }
            if let Some(node) = covering.filter(|node| case.starts_with(node)) {
                return Err(at(format!("{case} is under {node}*, listed before it")));
            }
            covering = node_of(case).or(covering);
            last = Some(case);
            expected.results.insert(case.to_owned(), status);
        }
        expected.cases = cases.ok_or_else(|| format!("{file}: no `# cases: <n>` line"))?;
        Ok(expected)
    }

    /// Read the expected results of `group` from `path`, or `None` when the
    /// file does not exist.
    ///
    /// # Errors
    ///
    /// Returns a message for a file that cannot be read or does not parse.
    pub fn load(path: &Path, group: &str) -> Result<Option<Self>, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, group, &path.display().to_string()).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The status `case` is expected to end with: that of its own line, or
    /// of the `.*` line of the nearest node above it, or pass.
    #[must_use]
    pub fn status(&self, case: &str) -> Status {
        if let Some(&status) = self.results.get(case) {
            return status;
        }
        let mut node = case;
        while let Some((parent, _)) = node.rsplit_once('.') {
            if let Some(&status) = self.results.get(&format!("{parent}.*")) {
                return status;
            }
            node = parent;
        }
        Status::Pass
    }
}

/// What differs between the components of two builds, by name.
#[must_use]
pub fn component_changes(
    expected: &BTreeMap<String, String>,
    run: &BTreeMap<String, String>,
) -> Vec<HeaderChange> {
    let names: BTreeSet<&String> = expected.keys().chain(run.keys()).collect();
    names
        .into_iter()
        .filter_map(|name| {
            let from = expected.get(name);
            let to = run.get(name);
            let same = matches!((from, to), (Some(from), Some(to)) if same_commit(from, to));
            (!same).then(|| HeaderChange::Component {
                name: name.clone(),
                from: from.cloned(),
                to: to.cloned(),
            })
        })
        .collect()
}

/// A case whose run differs from its expected result.
pub struct Change {
    pub case: String,
    pub expected: Status,
    /// Its run's status; `None` for a listed case the group no longer
    /// selects.
    pub run: Option<Status>,
    /// Whether the triage marks it flaky.
    pub flaky: bool,
}

impl Change {
    /// The class the change gates with: that of the status change, stale
    /// for a case no longer selected, and for a flaky case regression on a
    /// new crash or stall and `None` otherwise.
    #[must_use]
    pub fn class(&self) -> Option<Class> {
        let Some(run) = self.run else {
            return Some(Class::Stale);
        };
        if self.flaky {
            return (run.is_crash_or_stall() && run != self.expected).then_some(Class::Regression);
        }
        self.expected.change_to(run)
    }

    /// The label in [`Report::text`] and the slot of its count.
    fn label(&self) -> (&'static str, usize) {
        match self.class() {
            Some(Class::Regression) => ("REGRESSION", 0),
            Some(Class::Stale) => ("stale", 1),
            None => ("flaky", 2),
        }
    }
}

/// The diff of a run of one group with its expected results.
pub struct Report {
    pub group: String,
    /// How the run's build differs from the recorded one.
    pub header: Vec<HeaderChange>,
    /// The cases that differ, regressions first, then stale cases, then
    /// flaky cases that do not gate, each by name.
    pub changes: Vec<Change>,
    /// The cases that match.
    pub matched: usize,
    /// For a run of every case of the group, the number of cases the
    /// expected results were recorded for and the number the group selects
    /// now, when they differ.
    pub count: Option<(usize, usize)>,
}

impl Report {
    /// The class of the worst change, or [`Verdict::Match`]; at least
    /// [`Verdict::Stale`] when the group selects another number of cases
    /// than it was recorded with.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        let worst = self
            .changes
            .iter()
            .filter_map(Change::class)
            .chain(self.count.map(|_| Class::Stale))
            .max();
        worst.map_or(Verdict::Match, |class| match class {
            Class::Stale => Verdict::Stale,
            Class::Regression => Verdict::Regression,
        })
    }

    /// The report as text: the header's differences, one line per change,
    /// and a count line.
    #[must_use]
    pub fn text(&self) -> String {
        let group = &self.group;
        let mut text = String::new();
        for change in &self.header {
            let _ = writeln!(text, "deqp-vk {group}: header: {change}");
        }
        if let Some((recorded, selected)) = self.count {
            let _ = writeln!(
                text,
                "stale deqp-vk {group}: the group selects {selected} cases, its expected results \
                 were recorded for {recorded}; record it again"
            );
        }
        let mut counts = [0usize; 3];
        for change in &self.changes {
            let (label, slot) = change.label();
            counts[slot] += 1;
            let run = change
                .run
                .map_or_else(|| String::from("not selected"), |run| run.to_string());
            let _ = writeln!(
                text,
                "{label} deqp-vk {group} {}: {} -> {run}",
                change.case, change.expected
            );
        }
        let [regressions, stale, flaky] = counts;
        let _ = writeln!(
            text,
            "deqp-vk {group}: {regressions} regressions, {stale} stale, {flaky} flaky, {} match",
            self.matched
        );
        text
    }
}

/// Compare the `results` of a run of `selected`, the cases of the group in
/// scope, with the group's `expected` results.
///
/// `whole` says whether `selected` is every case the group selects: only
/// then is a listed case outside it no longer selected, and only then does
/// a number of cases other than [`Expected::cases`] make the results stale.
/// A case of
/// `selected` without a result counts as a crash; the runner gives every
/// case one.
#[must_use]
pub fn diff(
    expected: &Expected,
    selected: &[&str],
    whole: bool,
    results: &BTreeMap<String, Status>,
    components: &BTreeMap<String, String>,
    triage: &Triage,
) -> Report {
    let mut changes = Vec::new();
    let mut matched = 0;
    for &case in selected {
        let want = expected.status(case);
        let run = results.get(case).copied().unwrap_or(Status::Crash);
        if want == run {
            matched += 1;
            continue;
        }
        changes.push(Change {
            case: case.to_owned(),
            expected: want,
            run: Some(run),
            flaky: triage.is_flaky(case),
        });
    }
    if whole {
        let selected: BTreeSet<&str> = selected.iter().copied().collect();
        for (case, &status) in &expected.results {
            let covers = node_of(case).map_or_else(
                || selected.contains(case.as_str()),
                |node| {
                    selected
                        .range(node..)
                        .next()
                        .is_some_and(|first| first.starts_with(node))
                },
            );
            if !covers {
                changes.push(Change {
                    case: case.clone(),
                    expected: status,
                    run: None,
                    flaky: triage.is_flaky(case),
                });
            }
        }
    }
    changes.sort_by(|a, b| {
        a.label()
            .1
            .cmp(&b.label().1)
            .then_with(|| a.case.cmp(&b.case))
    });
    Report {
        group: expected.group.clone(),
        header: component_changes(&expected.components, components),
        changes,
        matched,
        count: (whole && selected.len() != expected.cases)
            .then_some((expected.cases, selected.len())),
    }
}

/// The expected results a record of `group` writes from the runs of
/// `selected`.
///
/// Each case gets its status in `first`, or the worse of its two statuses
/// where `second` ran it again and differs; [`compress`] turns them into
/// lines. Returns them with the cases whose two runs differ, by name, each
/// with its two statuses.
#[must_use]
pub fn record(
    group: &str,
    selected: &[&str],
    first: &BTreeMap<String, Status>,
    second: &BTreeMap<String, Status>,
    components: &BTreeMap<String, String>,
) -> (Expected, Vec<(String, Status, Status)>) {
    let mut statuses = BTreeMap::new();
    let mut unstable = Vec::new();
    for &case in selected {
        let one = first.get(case).copied().unwrap_or(Status::Crash);
        let status = match second.get(case) {
            Some(&two) if two != one => {
                unstable.push((case.to_owned(), one, two));
                if two.rank() > one.rank() { two } else { one }
            }
            _ => one,
        };
        statuses.insert(case, status);
    }
    let cases: Vec<(&str, Status)> = statuses.into_iter().collect();
    let mut results = BTreeMap::new();
    compress(&cases, "", &mut results);
    let expected = Expected {
        group: group.to_owned(),
        components: components.clone(),
        cases: selected.len(),
        results,
    };
    (expected, unstable)
}

/// Write into `out` the lines of the expected results for `cases`.
///
/// `cases` are sorted by name and all under `node`, `""` or a node with its
/// trailing dot. They get one line `<node>*` with their status when they
/// share one, other than pass, and else the lines of each child node and
/// case in turn. A node with one case gets that case's own line. Cases that
/// pass get no line.
pub fn compress(cases: &[(&str, Status)], node: &str, out: &mut BTreeMap<String, Status>) {
    let Some(&(first, status)) = cases.first() else {
        return;
    };
    if !node.is_empty() && cases.iter().all(|&(_, s)| s == status) {
        if status != Status::Pass {
            let key = if cases.len() == 1 {
                first.to_owned()
            } else {
                format!("{node}*")
            };
            out.insert(key, status);
        }
        return;
    }
    // The child each case is under, with its trailing dot; `None` for a
    // case right under `node`.
    let child = |case: &str| -> Option<usize> {
        case[node.len()..].find('.').map(|at| node.len() + at + 1)
    };
    let mut start = 0;
    while start < cases.len() {
        let (case, status) = cases[start];
        let Some(end) = child(case) else {
            if status != Status::Pass {
                out.insert(case.to_owned(), status);
            }
            start += 1;
            continue;
        };
        let prefix = &case[..end];
        let run = cases[start..]
            .iter()
            .take_while(|(other, _)| other.starts_with(prefix))
            .count();
        compress(&cases[start..start + run], prefix, out);
        start += run;
    }
}

#[cfg(test)]
mod tests;
