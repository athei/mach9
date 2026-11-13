//! The `run` and `record` commands.
//!
//! Both run the keys of each selected executable, one process tree per key
//! ([`crate::exec`]) in one session of a fresh prefix ([`crate::session`]),
//! leg by leg and unit by unit: a wrapped unit's keys come from its listing
//! (`WT_LIST=1`), which must be trustworthy, and a whole unit is one test.
//! Before any result of an executable counts, the `compatdb:` block of its
//! test process must name the implementations `wine-tests.toml` expects
//! ([`check_expect`]).
//!
//! - `run` compares the results with the committed baseline ([`diff`]). A
//!   key whose change gates runs a second time; when the second result is
//!   the baseline's, the key is unstable, not a regression.
//! - `record` runs again each key whose result differs from the old
//!   baseline, writes the unstable keys with the worse of their two
//!   results ([`record`]) and writes `<baselines>/<exe>.txt`.
//!
//! The decisions are the pure functions [`run_again`], [`decide_run`] and
//! [`decide_record`]; the resume records split by pass in
//! [`split_records`].
//!
//! Every process's output is kept in `runs/<run id>/<leg>/<exe>/<unit>/`
//! of the state directory, with how it ended appended; `record` copies the
//! logs of the recorded run to `baseline-raw/<exe>/`. A [`Resume`] file per
//! leg keeps the finished tests, so an interrupted invocation continues
//! where it stopped when it is started again with the same arguments.
//!
//! The resume files of every executable stay until all executables are
//! done and the session has ended. Only then does `record` write its
//! baselines and raw logs, and the resume files go last (`finalize` says
//! what an error or a kill leaves at each step). An interrupted `record
//! all` has written no baseline yet, so the invocation that resumes it
//! reads the same old baselines, takes both passes of the finished keys
//! from the resume files, and decides as the uninterrupted one would have.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};

use compatdb_core::Wined3d;

use crate::{
    baseline::{Baseline, TRIAGE_FILE},
    compatdb_log::Processes,
    diff::{self, Scope, Verdict},
    environment,
    exec::{self, Finished, Limits, Sampler, Spec},
    header::{self, Header},
    layout::{self, BASELINE_RAW_TREE, Bundle, State},
    model::{Config, Exe, Expect, PeDir, Selection},
    outcome::{Key, Outcome, Status, TestId},
    output::{self, Ending},
    record::{self, RecordError},
    resume::{Identity, Mode, Pass, Record, Resume},
    session::{self, Failure, Session},
    triage::Triage,
};

/// macOS's sampler, which a stalled test is sampled with.
const SAMPLE_TOOL: &str = "/usr/bin/sample";

/// How many of the slowest tests the summary names.
const SLOWEST: usize = 10;

/// What an invocation of `run` or `record` was asked to do.
pub struct Options {
    pub mode: Mode,
    /// Executable names, or `all`.
    pub suites: Vec<String>,
    /// The one leg to run, or every leg an executable is built for.
    pub leg: Option<PeDir>,
    /// The `--only` text: `<unit>/<key>[,...]`.
    pub only: Option<String>,
    pub bundle: PathBuf,
    pub state: PathBuf,
    /// The directory of the baselines and the triage file.
    pub baselines: PathBuf,
    pub limits: Limits,
}

/// How a command ended when it got that far.
#[derive(Debug)]
pub enum Conclusion {
    /// `run`: the worst verdict over the executables.
    Run(Verdict),
    /// `record`: the number of unstable keys it wrote.
    Record { unstable: usize },
}

impl Conclusion {
    /// 0 for a match or a stable record, else 1.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Run(Verdict::Match) | Self::Record { unstable: 0 } => 0,
            Self::Run(_) | Self::Record { .. } => 1,
        }
    }

    /// The last line the command prints, which names the conclusion.
    #[must_use]
    pub fn last_line(&self) -> String {
        match self {
            Self::Run(Verdict::Match) => String::from("conformance: match"),
            Self::Run(Verdict::Stale) => String::from(
                "conformance: stale baseline: results got better or keys are new; record again",
            ),
            Self::Run(Verdict::Unstable) => String::from(
                "conformance: unstable keys: their second run gave the baseline's result",
            ),
            Self::Run(Verdict::Regression) => String::from("conformance: regression"),
            Self::Record { unstable: 0 } => String::from("conformance: recorded a stable baseline"),
            Self::Record { unstable } => format!(
                "conformance: unstable keys: {unstable} were written with the worse of their two \
                 results"
            ),
        }
    }
}

/// One executable to run: its legs, the keys `--only` selects, and the
/// scope of the diff or record.
pub struct Plan<'c> {
    pub exe: &'c Exe,
    pub legs: Vec<PeDir>,
    /// The selected `(unit, key)` pairs, or `None` for every key.
    pub only: Option<BTreeSet<(String, Key)>>,
    pub scope: Scope,
}

impl Plan<'_> {
    /// The scope as the resume identity spells it.
    #[must_use]
    pub fn scope_text(&self) -> String {
        let legs: Vec<&str> = self.legs.iter().map(PeDir::as_str).collect();
        let only = self.only.as_ref().map_or_else(
            || String::from("all"),
            |only| {
                let keys: Vec<String> = only
                    .iter()
                    .map(|(unit, key)| format!("{unit}/{key}"))
                    .collect();
                keys.join(",")
            },
        );
        format!("legs={} only={only}", legs.join(","))
    }

    /// Whether the plan runs any key of `unit`.
    fn runs_unit(&self, unit: &str) -> bool {
        self.only
            .as_ref()
            .is_none_or(|only| only.iter().any(|(selected, _)| selected == unit))
    }
}

/// The executables `suites` name, each with its legs and scope.
///
/// # Errors
///
/// Returns a message for no suite, an unknown suite, a leg a named
/// executable is not built for, `--only` with other than one executable,
/// and a bad `--only` ([`parse_only`]).
pub fn plan<'c>(
    config: &'c Config,
    suites: &[String],
    leg: Option<PeDir>,
    only: Option<&str>,
) -> Result<Vec<Plan<'c>>, String> {
    if suites.is_empty() {
        return Err(String::from("no suite given"));
    }
    let mut exes: Vec<&Exe> = Vec::new();
    for suite in suites {
        let named: Vec<&Exe> = if suite == "all" {
            config
                .exes
                .iter()
                .filter(|exe| leg.is_none_or(|leg| exe.pe_dirs.contains(&leg)))
                .collect()
        } else {
            let exe = config
                .exes
                .iter()
                .find(|exe| exe.name == *suite)
                .ok_or_else(|| {
                    let names: Vec<&str> =
                        config.exes.iter().map(|exe| exe.name.as_str()).collect();
                    format!(
                        "unknown suite {suite:?}; the suites are {} and all",
                        names.join(", ")
                    )
                })?;
            if let Some(leg) = leg
                && !exe.pe_dirs.contains(&leg)
            {
                return Err(format!("{suite} is not built for {}", leg.as_str()));
            }
            vec![exe]
        };
        for exe in named {
            if !exes.iter().any(|known| known.name == exe.name) {
                exes.push(exe);
            }
        }
    }
    if only.is_some() && exes.len() != 1 {
        return Err(String::from("--only needs exactly one suite"));
    }
    exes.into_iter()
        .map(|exe| {
            let legs = leg.map_or_else(|| exe.pe_dirs.clone(), |leg| vec![leg]);
            let only = only.map(|text| parse_only(text, exe)).transpose()?;
            let scope = match (&only, legs.as_slice()) {
                (Some(keys), _) => Scope::Tests(
                    legs.iter()
                        .flat_map(|&leg| {
                            keys.iter().map(move |(unit, key)| TestId {
                                leg,
                                unit: unit.clone(),
                                key: key.clone(),
                            })
                        })
                        .collect(),
                ),
                (None, _) if legs == exe.pe_dirs => Scope::All,
                (None, [leg]) => Scope::Leg(*leg),
                (None, _) => Scope::All,
            };
            Ok(Plan {
                exe,
                legs,
                only,
                scope,
            })
        })
        .collect()
}

/// The `(unit, key)` pairs of an `--only` text, `<unit>/<key>[,...]`.
///
/// # Errors
///
/// Returns a message for an empty item, an item without `/`, a unit `exe`
/// does not have, a key that is not one, `*` for a wrapped unit and a
/// function key for a whole unit.
pub fn parse_only(text: &str, exe: &Exe) -> Result<BTreeSet<(String, Key)>, String> {
    let mut keys = BTreeSet::new();
    for item in text.split(',') {
        let (unit, key) = item
            .split_once('/')
            .ok_or_else(|| format!("--only {item:?} is not <unit>/<key>"))?;
        let found = exe
            .units
            .iter()
            .find(|known| known.name == unit)
            .ok_or_else(|| format!("--only {item:?}: {} has no unit {unit:?}", exe.name))?;
        let key =
            Key::parse(key).ok_or_else(|| format!("--only {item:?}: {key:?} is not a key"))?;
        match (&found.selection, &key) {
            (Selection::Whole, Key::Whole) | (Selection::Wrapped(_), Key::Function(_)) => {}
            (Selection::Whole, Key::Function(_)) => {
                return Err(format!(
                    "--only {item:?}: {unit} runs as a whole, select it as {unit}/*"
                ));
            }
            (Selection::Wrapped(_), Key::Whole) => {
                return Err(format!(
                    "--only {item:?}: {unit} has keys of its own, name one"
                ));
            }
        }
        keys.insert((unit.to_owned(), key));
    }
    Ok(keys)
}

/// Check that the `compatdb:` block of `exe_file`'s process in `output`
/// names what `expect` asks for; a `wined3d` line that says `unset` counts
/// as [`Wined3d::Gl`].
///
/// # Errors
///
/// Returns a message naming the setting and what the block says.
pub fn check_expect(expect: &Expect, output: &str, exe_file: &str) -> Result<(), String> {
    let processes = Processes::parse(output);
    let mismatch = |key: &str, got: &str, want: &str| {
        format!("{exe_file} runs with {key} = {got}, not {want}")
    };
    let unknown = |key: &str, e: crate::compatdb_log::ImplError| {
        format!("cannot tell the {key} of {exe_file}: {e}")
    };
    if let Some(want) = expect.dxgi {
        let got = processes.dxgi(exe_file).map_err(|e| unknown("dxgi", e))?;
        if got != want {
            return Err(mismatch("dxgi", got.as_str(), want.as_str()));
        }
    }
    if let Some(want) = expect.opengl {
        let got = processes
            .opengl(exe_file)
            .map_err(|e| unknown("opengl", e))?;
        if got != want {
            return Err(mismatch("opengl", got.as_str(), want.as_str()));
        }
    }
    if let Some(want) = expect.wined3d {
        let got = processes
            .wined3d(exe_file)
            .map_err(|e| unknown("wined3d", e))?
            .unwrap_or(Wined3d::Gl);
        if got != want {
            return Err(mismatch("wined3d", got.as_str(), want.as_str()));
        }
    }
    Ok(())
}

/// What [`prepare`] checked and claimed: everything [`Prepared::run`]
/// needs before it starts the session.
pub struct Prepared<'c> {
    plans: Vec<Plan<'c>>,
    pub bundle: Bundle,
    header: Header,
    triage: Triage,
    olds: Vec<Option<Baseline>>,
    /// The state directory, locked until the run ends.
    pub state: State,
}

/// Check the inputs of `options` over `config` and claim the state
/// directory, all of which needs no Wine session.
///
/// # Errors
///
/// Returns [`Failure::Setup`] for a bad selection, a bundle without tests,
/// a missing baseline of `run`, a partial `record` on another build, a
/// state directory that is not the runner's or is in use, and a wineserver
/// of the bundle that serves another prefix.
pub fn prepare<'c>(config: &'c Config, options: &Options) -> Result<Prepared<'c>, Failure> {
    let plans = plan(
        config,
        &options.suites,
        options.leg,
        options.only.as_deref(),
    )
    .map_err(Failure::Setup)?;
    let bundle = Bundle::open(&options.bundle).map_err(Failure::Setup)?;
    let header = Header::read(&bundle.tests).map_err(Failure::Setup)?;
    let triage = Triage::load(&options.baselines.join(TRIAGE_FILE)).map_err(Failure::Setup)?;
    let mut olds = Vec::new();
    for plan in &plans {
        let path = baseline_path(&options.baselines, &plan.exe.name);
        let old = Baseline::load(&path, &plan.exe.name).map_err(Failure::Setup)?;
        match (&options.mode, &old) {
            (Mode::Run, None) => {
                return Err(Failure::Setup(format!(
                    "no baseline {}: record one first",
                    path.display()
                )));
            }
            (Mode::Record, Some(old)) if !matches!(plan.scope, Scope::All) => {
                let changes = header::changes(&old.header, &header);
                if !changes.is_empty() {
                    return Err(Failure::Setup(format!(
                        "{}: {}",
                        plan.exe.name,
                        RecordError::HeaderDiffers(changes)
                    )));
                }
            }
            _ => {}
        }
        olds.push(old);
    }
    let state = State::claim(&options.state).map_err(Failure::Setup)?;
    session::check_servers(&bundle, &state.prefix())?;
    Ok(Prepared {
        plans,
        bundle,
        header,
        triage,
        olds,
        state,
    })
}

impl Prepared<'_> {
    /// Run what [`prepare`] checked: start the session, run or record each
    /// executable, end the session.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Setup`] for a wineserver of the bundle that runs
    /// by now, a failed session step, an untrustworthy listing, an
    /// executable on the wrong implementation and an I/O error, and
    /// [`Failure::Interrupted`] once `stop` is set.
    pub fn run(
        &self,
        options: &Options,
        caller_env: Vec<(OsString, OsString)>,
        stop: &AtomicBool,
    ) -> Result<Conclusion, Failure> {
        let state = &self.state;
        let run_dir = state.runs().join(layout::run_id(SystemTime::now()));
        fs::create_dir_all(&run_dir).map_err(|e| setup_io(&run_dir, &e))?;
        println!("conformance: raw logs in {}", run_dir.display());
        let env = environment::wine_env(caller_env, &state.prefix(), &state.work());
        let session = Session::start(
            &self.bundle,
            state,
            &env,
            &run_dir.join("session.log"),
            stop,
        )?;
        let mut runner = Runner {
            bundle: &self.bundle,
            state,
            run_dir,
            env: &env,
            limits: options.limits,
            session,
            stop,
            timings: Vec::new(),
            resumed: 0,
            started: Instant::now(),
            finished: Vec::new(),
            pending: Vec::new(),
        };
        let mut conclusion = match options.mode {
            Mode::Run => Conclusion::Run(Verdict::Match),
            Mode::Record => Conclusion::Record { unstable: 0 },
        };
        for (plan, old) in self.plans.iter().zip(&self.olds) {
            conclusion = match (conclusion, old) {
                (Conclusion::Run(verdict), Some(baseline)) => Conclusion::Run(
                    verdict.max(runner.run_exe(plan, &self.header, baseline, &self.triage)?),
                ),
                (Conclusion::Record { unstable }, old) => Conclusion::Record {
                    unstable: unstable
                        + runner.record_exe(
                            plan,
                            &self.header,
                            old.as_ref(),
                            &options.baselines,
                        )?,
                },
                (Conclusion::Run(_), None) => {
                    return Err(Failure::Setup(format!("no baseline of {}", plan.exe.name)));
                }
            };
        }
        runner.session.end().map_err(Failure::Setup)?;
        let finished = std::mem::take(&mut runner.finished);
        finalize(state, &runner.run_dir, &runner.pending, finished)?;
        print!("{}", runner.totals());
        Ok(conclusion)
    }
}

/// `<baselines>/<exe>.txt`.
fn baseline_path(baselines: &Path, exe: &str) -> PathBuf {
    baselines.join(format!("{exe}.txt"))
}

/// A setup failure for an I/O error on `path`.
fn setup_io(path: &Path, e: &std::io::Error) -> Failure {
    Failure::Setup(format!("{}: {e}", path.display()))
}

/// The state of one leg of an executable: its resume file and the
/// outcomes it holds.
struct LegRun {
    leg: PeDir,
    resume: Resume,
    first: BTreeMap<(String, Key), Outcome>,
    confirm: BTreeMap<(String, Key), Outcome>,
}

/// What every test of an invocation shares.
struct Runner<'a> {
    bundle: &'a Bundle,
    state: &'a State,
    run_dir: PathBuf,
    env: &'a [(OsString, OsString)],
    limits: Limits,
    session: Session<'a>,
    stop: &'a AtomicBool,
    /// Each test run in this invocation: its time, label and status.
    timings: Vec<(Duration, String, Status)>,
    /// The tests taken from resume files.
    resumed: usize,
    started: Instant,
    /// The resume files of the executables that are done, removed once
    /// every executable is and the session has ended.
    finished: Vec<Resume>,
    /// The recorded executables, written after the resume files are gone.
    pending: Vec<Pending>,
}

impl Runner<'_> {
    /// `run` of one executable: the verdict of its diff, with the keys
    /// whose second run gave the baseline's result as unstable.
    fn run_exe(
        &mut self,
        plan: &Plan<'_>,
        header: &Header,
        baseline: &Baseline,
        triage: &Triage,
    ) -> Result<Verdict, Failure> {
        let exe = &plan.exe.name;
        let mut legs = self.open_legs(plan, Mode::Run, header)?;
        let first = self.first_pass(plan, &mut legs)?;
        let report = diff::diff(exe, Some(baseline), header, &first, &plan.scope, triage);
        let mut second = BTreeMap::new();
        for (id, _) in run_again(&report) {
            second.insert(id.clone(), self.confirm(plan, &mut legs, &id)?);
        }
        let decided = decide_run(exe, baseline, header, first, &second, &plan.scope, triage);
        print!("{}", decided.report.text());
        for (id, first) in &decided.unstable {
            println!(
                "unstable {exe} {id}: the first run gave {first}, the second the baseline's result"
            );
        }
        self.finished.extend(legs.into_iter().map(|leg| leg.resume));
        Ok(decided.verdict)
    }

    /// `record` of one executable: its new baseline, kept to be written
    /// once every executable is done; the number of unstable keys.
    fn record_exe(
        &mut self,
        plan: &Plan<'_>,
        header: &Header,
        old: Option<&Baseline>,
        baselines: &Path,
    ) -> Result<usize, Failure> {
        let exe = &plan.exe.name;
        let mut legs = self.open_legs(plan, Mode::Record, header)?;
        let first = self.first_pass(plan, &mut legs)?;
        let mut second = BTreeMap::new();
        for id in record::to_confirm(old, &first) {
            let outcome = self.confirm(plan, &mut legs, &id)?;
            second.insert(id, outcome);
        }
        let merge = decide_record(exe, old, header, &first, &second, &plan.scope)
            .map_err(|e| Failure::Setup(format!("{exe}: {e}")))?;
        let unstable = merge.unstable.len();
        self.finished.extend(legs.into_iter().map(|leg| leg.resume));
        self.pending.push(Pending {
            exe: exe.clone(),
            legs: plan.legs.clone(),
            all: matches!(plan.scope, Scope::All),
            path: baseline_path(baselines, exe),
            merge,
            results: first,
        });
        Ok(unstable)
    }

    /// Open the resume file of each leg of `plan`.
    fn open_legs(
        &self,
        plan: &Plan<'_>,
        mode: Mode,
        header: &Header,
    ) -> Result<Vec<LegRun>, Failure> {
        let exe = &plan.exe.name;
        plan.legs
            .iter()
            .map(|&leg| {
                let path = self.bundle.test_exe(leg, exe);
                let digest = layout::file_digest(&path).map_err(|e| setup_io(&path, &e))?;
                let identity = Identity {
                    mode,
                    exe: exe.clone(),
                    leg,
                    digest,
                    scope: plan.scope_text(),
                    header: header.clone(),
                };
                let dir = self.state.resume();
                let opened = Resume::open(&dir, &identity).map_err(|e| setup_io(&dir, &e))?;
                if let Some(why) = &opened.discarded {
                    println!(
                        "conformance: {exe} {}: discarded the earlier state: {why}",
                        leg.as_str()
                    );
                }
                if !opened.records.is_empty() {
                    println!(
                        "conformance: {exe} {}: resuming with {} finished tests",
                        leg.as_str(),
                        opened.records.len()
                    );
                }
                let mut run = LegRun {
                    leg,
                    resume: opened.resume,
                    first: BTreeMap::new(),
                    confirm: BTreeMap::new(),
                };
                (run.first, run.confirm) = split_records(opened.records);
                Ok(run)
            })
            .collect()
    }

    /// Run every key of `plan` once, leg by leg, taking the resumed ones
    /// from the leg's state.
    fn first_pass(
        &mut self,
        plan: &Plan<'_>,
        legs: &mut [LegRun],
    ) -> Result<BTreeMap<TestId, Outcome>, Failure> {
        let mut results = BTreeMap::new();
        for leg_run in legs {
            let leg = leg_run.leg;
            for unit in plan
                .exe
                .units
                .iter()
                .filter(|unit| plan.runs_unit(&unit.name))
            {
                let keys = match &unit.selection {
                    Selection::Whole => vec![Key::Whole],
                    Selection::Wrapped(_) => self.listed_keys(plan, leg, &unit.name)?,
                };
                for key in keys {
                    let slot = (unit.name.clone(), key.clone());
                    let outcome = if let Some(outcome) = leg_run.first.get(&slot) {
                        self.resumed += 1;
                        *outcome
                    } else {
                        let outcome = self.run_test(plan, leg, &unit.name, &key, Pass::First)?;
                        append(&mut leg_run.resume, Pass::First, &unit.name, &key, outcome)?;
                        outcome
                    };
                    results.insert(
                        TestId {
                            leg,
                            unit: unit.name.clone(),
                            key,
                        },
                        outcome,
                    );
                }
            }
        }
        Ok(results)
    }

    /// Run `id` a second time, or take its second result from the state.
    fn confirm(
        &mut self,
        plan: &Plan<'_>,
        legs: &mut [LegRun],
        id: &TestId,
    ) -> Result<Outcome, Failure> {
        let leg_run = legs
            .iter_mut()
            .find(|run| run.leg == id.leg)
            .ok_or_else(|| Failure::Setup(format!("{id} is outside the run's legs")))?;
        if let Some(outcome) = leg_run.confirm.get(&(id.unit.clone(), id.key.clone())) {
            self.resumed += 1;
            return Ok(*outcome);
        }
        let outcome = self.run_test(plan, id.leg, &id.unit, &id.key, Pass::Confirm)?;
        append(
            &mut leg_run.resume,
            Pass::Confirm,
            &id.unit,
            &id.key,
            outcome,
        )?;
        Ok(outcome)
    }

    /// The keys of `unit` the plan runs: those its listing prints, or the
    /// `--only` ones, which the listing must print.
    fn listed_keys(
        &mut self,
        plan: &Plan<'_>,
        leg: PeDir,
        unit: &str,
    ) -> Result<Vec<Key>, Failure> {
        let listed = self.list(plan, leg, unit)?;
        let Some(only) = &plan.only else {
            return Ok(listed.into_iter().map(Key::Function).collect());
        };
        let wanted: Vec<Key> = only
            .iter()
            .filter(|(selected, _)| selected == unit)
            .map(|(_, key)| key.clone())
            .collect();
        for key in &wanted {
            if !listed.iter().any(|name| key.as_str() == name) {
                return Err(Failure::Setup(format!(
                    "{} {}: the listing of {unit} has no key {key}; it lists {}",
                    plan.exe.name,
                    leg.as_str(),
                    listed.join(", ")
                )));
            }
        }
        Ok(wanted)
    }

    /// The listing of `unit` on `leg`, which must be trustworthy and come
    /// from a process on the expected implementation.
    fn list(&mut self, plan: &Plan<'_>, leg: PeDir, unit: &str) -> Result<Vec<String>, Failure> {
        let exe = &plan.exe.name;
        let log = self
            .run_dir
            .join(leg.as_str())
            .join(exe)
            .join(format!("{unit}.list.log"));
        let finished = self.spawn(plan, leg, unit, Some(("WT_LIST", "1")), &log, "listing")?;
        let label = format!("{exe} {} {unit}", leg.as_str());
        let see = format!("its output is in {}", log.display());
        if finished.ending == Ending::Stalled {
            return Err(Failure::Setup(format!(
                "{label}: the listing stalled; {see}"
            )));
        }
        let text = String::from_utf8_lossy(&finished.output);
        check_expect(&plan.exe.expect, &text, &format!("{exe}_test.exe"))
            .map_err(|e| Failure::Setup(format!("{label}: {e}; {see}")))?;
        let listing = output::parse_listing(&text, unit).map_err(|e| {
            let mut message = format!("{label}: the listing has no keys ({:?})", e.problem);
            for line in e.skips.iter().chain(&e.failures) {
                let _ = write!(message, "\n  {line}");
            }
            let _ = write!(message, "\n{see}");
            Failure::Setup(message)
        })?;
        if !listing.is_trustworthy() {
            let mut message = format!(
                "{label}: the listing's setup printed failures, so its keys are not trusted:"
            );
            for line in &listing.failures {
                let _ = write!(message, "\n  {line}");
            }
            let _ = write!(message, "\n{see}");
            return Err(Failure::Setup(message));
        }
        Ok(listing.keys)
    }

    /// Run one test, again after a session restart when a key was not
    /// reached, and print its progress line.
    fn run_test(
        &mut self,
        plan: &Plan<'_>,
        leg: PeDir,
        unit: &str,
        key: &Key,
        pass: Pass,
    ) -> Result<Outcome, Failure> {
        let exe = &plan.exe.name;
        let log = self
            .run_dir
            .join(leg.as_str())
            .join(exe)
            .join(unit)
            .join(layout::log_file(key));
        let selection = match key {
            Key::Whole => None,
            Key::Function(name) => Some(("WT_ONLY", name.as_str())),
        };
        let pass_name = match pass {
            Pass::First => "first",
            Pass::Confirm => "second",
        };
        let start = Instant::now();
        let mut attempt = 1;
        let outcome = loop {
            let what = format!("{pass_name} run, attempt {attempt}");
            let finished = self.spawn(plan, leg, unit, selection, &log, &what)?;
            let text = String::from_utf8_lossy(&finished.output);
            if *key == Key::Whole && finished.ending != Ending::Stalled {
                check_expect(&plan.exe.expect, &text, &format!("{exe}_test.exe")).map_err(|e| {
                    Failure::Setup(format!(
                        "{exe} {} {unit}: {e}; its output is in {}",
                        leg.as_str(),
                        log.display()
                    ))
                })?;
            }
            let run = output::parse_run(&text, unit, key, finished.ending);
            append_log(
                &log,
                &format!(
                    "[conformance] parsed: {}{}\n",
                    run.outcome,
                    run.crash
                        .as_ref()
                        .map_or_else(String::new, |crash| format!(", {crash:?}"))
                ),
            )?;
            if run.outcome.status == Status::NotReached && attempt == 1 {
                self.session.restart()?;
                attempt += 1;
                continue;
            }
            break run.outcome;
        };
        let elapsed = start.elapsed();
        let label = format!("{exe} {} {unit}/{key}", leg.as_str());
        let note = if pass == Pass::Confirm {
            " (second run)"
        } else {
            ""
        };
        println!("{label} {outcome} {:.1}s{note}", elapsed.as_secs_f64());
        self.timings.push((elapsed, label, outcome.status));
        Ok(outcome)
    }

    /// Run the test process of `unit` on `leg` with the selection variable
    /// `selection`, append its output and ending to `log`, and restart the
    /// session after a stall.
    fn spawn(
        &mut self,
        plan: &Plan<'_>,
        leg: PeDir,
        unit: &str,
        selection: Option<(&str, &str)>,
        log: &Path,
        what: &str,
    ) -> Result<Finished, Failure> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(Failure::Interrupted);
        }
        let dir = log.parent().unwrap_or(&self.run_dir).to_path_buf();
        fs::create_dir_all(&dir).map_err(|e| setup_io(&dir, &e))?;
        let exe_path = self.bundle.test_exe(leg, &plan.exe.name);
        let args = [exe_path.clone().into_os_string(), OsString::from(unit)];
        let mut env = self.env.to_vec();
        if let Some((name, value)) = selection {
            env.push((OsString::from(name), OsString::from(value)));
        }
        let spec = Spec {
            program: &self.bundle.wine,
            args: &args,
            env: &env,
            cwd: &self.state.work(),
        };
        let sampler = Sampler {
            tool: PathBuf::from(SAMPLE_TOOL),
            dir,
            stem: log
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned()),
            wineserver: Some(self.session.server()),
        };
        let finished = exec::run_tree(&spec, &self.limits, Some(&sampler), self.stop).map_err(
            |e| match e {
                exec::Error::Interrupted => Failure::Interrupted,
                e => Failure::Setup(format!("{} {e}", self.bundle.wine.display())),
            },
        )?;
        let selected =
            selection.map_or_else(String::new, |(name, value)| format!(" {name}={value}"));
        let mut text = format!(
            "[conformance] {what}:{selected} {} {} {unit}\n",
            self.bundle.wine.display(),
            exe_path.display()
        );
        text.push_str(&String::from_utf8_lossy(&finished.output));
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&finished.trailer(&self.limits));
        append_log(log, &text)?;
        if finished.ending == Ending::Stalled {
            self.session.restart()?;
        }
        Ok(finished)
    }

    /// The totals of the invocation and its slowest tests.
    fn totals(&self) -> String {
        let mut counts = String::new();
        for status in Status::ALL {
            let count = self.timings.iter().filter(|(_, _, s)| *s == status).count();
            let _ = write!(counts, ", {count} {status}");
        }
        let mut text = format!(
            "conformance: {} tests run, {} resumed, in {:.1} s{counts}\n",
            self.timings.len(),
            self.resumed,
            self.started.elapsed().as_secs_f64()
        );
        let mut slowest: Vec<&(Duration, String, Status)> = self.timings.iter().collect();
        slowest.sort_by_key(|timing| std::cmp::Reverse(timing.0));
        if !slowest.is_empty() {
            text.push_str("conformance: the slowest tests:\n");
        }
        for (elapsed, label, status) in slowest.into_iter().take(SLOWEST) {
            let _ = writeln!(text, "  {:7.1}s {label} {status}", elapsed.as_secs_f64());
        }
        text
    }
}

/// Append one finished test to a leg's resume file.
fn append(
    resume: &mut Resume,
    pass: Pass,
    unit: &str,
    key: &Key,
    outcome: Outcome,
) -> Result<(), Failure> {
    resume
        .append(&Record {
            pass,
            unit: unit.to_owned(),
            key: key.clone(),
            outcome,
        })
        .map_err(|e| Failure::Setup(format!("cannot write the resume state: {e}")))
}

/// The first-pass and the second-pass outcomes of a leg's resume records,
/// each by unit and key; a later record of a key replaces an earlier one
/// of its pass.
#[must_use]
pub fn split_records(records: Vec<Record>) -> (Outcomes, Outcomes) {
    let mut first = BTreeMap::new();
    let mut confirm = BTreeMap::new();
    for record in records {
        let slot = match record.pass {
            Pass::First => &mut first,
            Pass::Confirm => &mut confirm,
        };
        slot.insert((record.unit, record.key), record.outcome);
    }
    (first, confirm)
}

/// The outcomes of one leg by unit and key.
pub type Outcomes = BTreeMap<(String, Key), Outcome>;

/// The keys `run` runs a second time, each with its baseline outcome: those
/// whose change gates and that the run listed.
#[must_use]
pub fn run_again(report: &diff::Report) -> Vec<(TestId, Outcome)> {
    report
        .changes
        .iter()
        .filter(|change| change.run.is_some() && change.class().is_some())
        .filter_map(|change| Some((change.id.clone(), change.baseline?)))
        .collect()
}

/// What `run` concludes for one executable.
pub struct RunDecision {
    /// The diff of the results, the unstable keys at their baseline
    /// outcome.
    pub report: diff::Report,
    /// The unstable keys with their first outcome.
    pub unstable: Vec<(TestId, Outcome)>,
    /// The report's verdict, at least [`Verdict::Unstable`] when a key is.
    pub verdict: Verdict,
}

/// Decide `run` for one executable from its `first` pass and the `second`
/// outcomes of the keys [`run_again`] named.
///
/// A key whose second outcome is its baseline's is unstable: the diff takes
/// the baseline's outcome for it, so it is no regression, and the verdict
/// is at least [`Verdict::Unstable`]. Any other key keeps its first
/// outcome; a second outcome of a key `run_again` did not name is ignored.
#[must_use]
pub fn decide_run(
    exe: &str,
    baseline: &Baseline,
    header: &Header,
    first: BTreeMap<TestId, Outcome>,
    second: &BTreeMap<TestId, Outcome>,
    scope: &Scope,
    triage: &Triage,
) -> RunDecision {
    let report = diff::diff(exe, Some(baseline), header, &first, scope, triage);
    let mut results = first;
    let mut unstable = Vec::new();
    for (id, expected) in run_again(&report) {
        if second.get(&id) == Some(&expected)
            && let Some(first) = results.insert(id.clone(), expected)
        {
            unstable.push((id, first));
        }
    }
    let report = diff::diff(exe, Some(baseline), header, &results, scope, triage);
    let verdict = if unstable.is_empty() {
        report.verdict()
    } else {
        report.verdict().max(Verdict::Unstable)
    };
    RunDecision {
        report,
        unstable,
        verdict,
    }
}

/// Decide `record` for one executable: its new baseline.
///
/// The baseline comes from the `old` one, the `first` pass and the
/// `second` outcomes of the keys [`record::to_confirm`] named, with the
/// keys whose two outcomes differ written as [`record::unstable`] gives
/// them; a second outcome of another key is ignored.
///
/// # Errors
///
/// As [`record::merge`].
pub fn decide_record(
    exe: &str,
    old: Option<&Baseline>,
    header: &Header,
    first: &BTreeMap<TestId, Outcome>,
    second: &BTreeMap<TestId, Outcome>,
    scope: &Scope,
) -> Result<record::Merge, RecordError> {
    let confirm = record::to_confirm(old, first);
    let unstable = record::unstable(&confirm, first, second);
    record::merge(exe, old, header, first, scope, &unstable)
}

/// End an invocation whose executables are all done and whose session has
/// ended: write what `record` recorded, then remove the resume files.
///
/// Every step that can fail comes before anything is discarded, in this
/// order, and an error or a kill at each point leaves:
///
/// 1. Each `baseline-raw/<exe>` is claimed: an unmarked directory or a
///    symbolic link fails here. The old baselines and the resume files
///    stay.
/// 2. Each baseline is written to `<exe>.txt.new` beside its file. The old
///    baselines and the resume files stay; a `.txt.new` left behind is
///    replaced by the next record.
/// 3. The `.txt.new` files are renamed into place, one rename each. A kill
///    between two renames leaves the renamed baselines new and the others
///    old, with the resume files.
/// 4. The raw logs are copied. The new baselines and the resume files
///    stay; a resumed `record` takes every result from them and writes the
///    same baselines, though it may count fewer unstable keys, since it
///    compares with the new baselines.
/// 5. The resume files are removed.
///
/// # Errors
///
/// Returns [`Failure::Setup`] for the first step that fails.
fn finalize(
    state: &State,
    run_dir: &Path,
    pending: &[Pending],
    finished: Vec<Resume>,
) -> Result<(), Failure> {
    for record in pending {
        layout::claim_inner(&state.baseline_raw(&record.exe), BASELINE_RAW_TREE)
            .map_err(Failure::Setup)?;
    }
    let mut written = Vec::new();
    for record in pending {
        let new = record.path.with_extension("txt.new");
        fs::write(&new, record.merge.baseline.to_text()).map_err(|e| setup_io(&new, &e))?;
        written.push((new, &record.path));
    }
    for (new, path) in written {
        fs::rename(&new, path).map_err(|e| setup_io(path, &e))?;
    }
    for record in pending {
        copy_raw(state, run_dir, record)?;
        print!("{}", record.merge.text());
        println!("conformance: wrote {}", record.path.display());
    }
    for resume in finished {
        resume
            .finish()
            .map_err(|e| Failure::Setup(format!("cannot remove the resume state: {e}")))?;
    }
    Ok(())
}

/// Replace the recorded raw logs of a recorded executable with the logs of
/// its tests and this run's listings. A record of all tests replaces the
/// directory; a smaller one replaces their files.
fn copy_raw(state: &State, run_dir: &Path, pending: &Pending) -> Result<(), Failure> {
    let exe = &pending.exe;
    let dest = state.baseline_raw(exe);
    if pending.all {
        layout::fresh_dir(&dest, BASELINE_RAW_TREE)
    } else {
        layout::claim_inner(&dest, BASELINE_RAW_TREE)
    }
    .map_err(Failure::Setup)?;
    let mut copies: Vec<(PathBuf, PathBuf)> = Vec::new();
    for id in pending.results.keys() {
        let file = Path::new(&id.unit).join(layout::log_file(&id.key));
        if let Some(source) = find_log(state, run_dir, id.leg, exe, &file) {
            copies.push((source, dest.join(id.leg.as_str()).join(file)));
        }
    }
    for &leg in &pending.legs {
        let dir = run_dir.join(leg.as_str()).join(exe);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().ends_with(".list.log") {
                copies.push((entry.path(), dest.join(leg.as_str()).join(name)));
            }
        }
    }
    for (source, target) in copies {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| setup_io(parent, &e))?;
        }
        fs::copy(&source, &target).map_err(|e| setup_io(&source, &e))?;
    }
    Ok(())
}

/// The raw log `<leg>/<exe>/<file>` of this run, or of the latest earlier
/// run that has it, for a test taken from a resume file.
fn find_log(state: &State, run_dir: &Path, leg: PeDir, exe: &str, file: &Path) -> Option<PathBuf> {
    let rel = Path::new(leg.as_str()).join(exe).join(file);
    let here = run_dir.join(&rel);
    if here.is_file() {
        return Some(here);
    }
    let mut runs: Vec<PathBuf> = fs::read_dir(state.runs())
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    runs.sort();
    runs.into_iter()
        .rev()
        .map(|run| run.join(&rel))
        .find(|path| path.is_file())
}

/// A recorded executable whose baseline and raw logs are written once
/// every executable of the invocation is done.
struct Pending {
    exe: String,
    legs: Vec<PeDir>,
    /// Whether the record covers every test, so the raw logs are replaced
    /// as a whole.
    all: bool,
    path: PathBuf,
    merge: record::Merge,
    results: BTreeMap<TestId, Outcome>,
}

/// Append `text` to the raw log `path`.
fn append_log(path: &Path, text: &str) -> Result<(), Failure> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|e| setup_io(path, &e))
}

#[cfg(test)]
mod tests;
