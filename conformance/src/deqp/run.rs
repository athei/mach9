//! The `deqp-vk run` and `deqp-vk record` commands.
//!
//! Both select the cases of the groups from the mustpass ([`groups`]), check
//! with a probe that `deqp-vk` loads the driver of the named Vulkan stack
//! ([`Vulkan`]), and run every case once: in
//! batches, one `deqp-vk` process per batch, several at a time. A batch
//! whose process dies or stalls during a case gives that case
//! [`Status::Crash`] or [`Status::Stall`] and goes on with its remaining
//! cases in a new process. The cases known to crash or stall, and those
//! that crashed or stalled next to other processes, run alone afterwards,
//! since a lost device can take other processes' devices with it.
//!
//! - `run` compares each group with its expected results
//!   ([`expected::diff`]). The cases whose change gates run a second time;
//!   one whose second result is the expected one is unstable, not a
//!   regression.
//! - `record` runs again each case whose result differs from the old
//!   expected results, a missing file counting as all pass, and writes each
//!   group's file with the worse of two differing results
//!   ([`expected::record`]).
//!
//! The output of every process is kept in `runs/<run id>/` of the state
//! directory, which serves one invocation at a time. An interrupted
//! invocation keeps nothing for the next one, which starts over.
//!
//! [`groups`]: crate::deqp::groups

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    ffi::OsString,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Instant, SystemTime},
};

use crate::{
    deqp::{
        expected::{self, Expected, Status},
        groups::{Group, Groups, glob},
        log, probe,
        triage::{TRIAGE_FILE, Triage},
    },
    diff::Verdict,
    environment,
    exec::{self, Limits, Sampler, Spec},
    header::Header,
    layout::{self, DEQP_STATE_TREE, DEQP_WORK_TREE, State},
    output::Ending,
    procs,
    resume::Mode,
    runner::Conclusion,
    session::Failure,
};

/// macOS's sampler, which a stalled batch is sampled with.
const SAMPLE_TOOL: &str = "/usr/bin/sample";

/// The case the probe runs: it creates an instance and a device and logs
/// the device's properties.
pub const PROBE_CASE: &str = "dEQP-VK.info.device_properties";

/// The case that shows the device works: it draws a triangle and waits for
/// the submission.
pub const CANARY: &str = "dEQP-VK.api.smoke.triangle";

/// How many more times `record` runs each case the triage marks flaky.
const FLAKY_RUNS: usize = 3;

/// How many processes [`CANARY`] gets to pass in after a lost device.
const RECOVERY_TRIES: usize = 5;

/// The name of the Vulkan loader in either layout [`Vulkan::open`] takes.
const LOADER: &str = "libvulkan.1.dylib";

/// What an invocation of `deqp-vk run` or `deqp-vk record` was asked to
/// do.
pub struct Options {
    pub mode: Mode,
    /// The group names; every group for none.
    pub groups: Vec<String>,
    /// `run` only: run the cases of the groups one of these patterns
    /// matches, and compare only those; all cases for none.
    pub cases: Vec<String>,
    /// A bundle or a `MESA_DIST` ([`Vulkan::open`]).
    pub vulkan: PathBuf,
    /// The dist of `bin/build-vk-gl-cts.sh` ([`Deqp::open`]).
    pub deqp: PathBuf,
    pub state: PathBuf,
    /// The directory of the expected results and the triage file.
    pub expected: PathBuf,
    /// How many `deqp-vk` processes run at a time.
    pub jobs: usize,
    /// How many cases one process runs at most.
    pub batch: usize,
    /// The watchdog's limits for one process: no output for `idle` is a
    /// stalled case, since a case prints a line when it starts and when it
    /// ends.
    pub limits: Limits,
}

/// The Vulkan stack the cases run on: a loader and the manifest directory
/// of its driver, with the commits they were built from.
pub struct Vulkan {
    /// The bundle or `MESA_DIST`, which the driver must come from.
    pub dir: PathBuf,
    pub loader: PathBuf,
    pub icd_dir: PathBuf,
    /// `mesa` and `vulkan-loader`.
    pub components: BTreeMap<String, String>,
}

impl Vulkan {
    /// The stack in `dir`, made absolute: a full bundle
    /// (`lib/external/libvulkan.1.dylib`, `share/vulkan/icd.d`, the commits
    /// from `lib/wine/tests/COMPONENTS`) or a `MESA_DIST` of
    /// `bin/build-mesa.sh` (`lib/libvulkan.1.dylib`, `share/vulkan/icd.d`,
    /// the commits from `COMMIT` and `LOADER_COMMIT`).
    ///
    /// # Errors
    ///
    /// Returns a message when `dir` is neither, or a commit is missing.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let icd_dir = dir.join("share").join("vulkan").join("icd.d");
        let bundled = dir.join("lib").join("external").join(LOADER);
        let staged = dir.join("lib").join(LOADER);
        let mut components = BTreeMap::new();
        let loader = if bundled.is_file() {
            let tests = dir.join("lib").join("wine").join("tests");
            let header = Header::read(&tests).map_err(|e| {
                format!("{e} (a full bundle of bin/bundle-wine.sh names its Mesa there)")
            })?;
            for name in ["mesa", "vulkan-loader"] {
                let commit = header
                    .components
                    .get(name)
                    .ok_or_else(|| format!("{}/COMPONENTS has no {name} line", tests.display()))?;
                components.insert(name.to_owned(), commit.clone());
            }
            bundled
        } else if staged.is_file() {
            for (name, file) in [("mesa", "COMMIT"), ("vulkan-loader", "LOADER_COMMIT")] {
                components.insert(name.to_owned(), read_word(&dir.join(file))?);
            }
            staged
        } else {
            return Err(format!(
                "{} is neither a bundle nor a MESA_DIST: it has no lib/external/{LOADER} or \
                 lib/{LOADER}",
                dir.display()
            ));
        };
        if !icd_dir.is_dir() {
            return Err(format!("{} is missing", icd_dir.display()));
        }
        Ok(Self {
            dir,
            loader,
            icd_dir,
            components,
        })
    }
}

/// The `deqp-vk` dist of `bin/build-vk-gl-cts.sh`.
pub struct Deqp {
    pub dir: PathBuf,
    pub program: PathBuf,
    /// Its VK-GL-CTS commit, `COMMIT`.
    pub commit: String,
}

impl Deqp {
    /// The dist in `dir`, made absolute.
    ///
    /// # Errors
    ///
    /// Returns a message when `deqp-vk`, `mustpass.txt`, `vulkan/` or
    /// `COMMIT` is missing.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let program = dir.join("deqp-vk");
        for path in [&program, &dir.join("mustpass.txt")] {
            if !path.is_file() {
                return Err(format!(
                    "{} is missing; bin/build-vk-gl-cts.sh builds it",
                    path.display()
                ));
            }
        }
        if !dir.join("vulkan").is_dir() {
            return Err(format!("{}/vulkan is missing", dir.display()));
        }
        let commit = read_word(&dir.join("COMMIT"))?;
        Ok(Self {
            dir,
            program,
            commit,
        })
    }

    /// The case names of the mustpass, in its order.
    ///
    /// # Errors
    ///
    /// Returns a message when it cannot be read.
    pub fn mustpass(&self) -> Result<String, String> {
        let path = self.dir.join("mustpass.txt");
        fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The first word of the file at `path`.
fn read_word(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| format!("{} is empty", path.display()))
}

/// One group to run: its cases and its expected results.
struct Planned<'g> {
    group: &'g Group,
    /// The cases in scope, in mustpass order.
    cases: Vec<String>,
    /// Whether they are every case the group selects.
    whole: bool,
    old: Option<Expected>,
    path: PathBuf,
}

/// Run `options` over `groups`: check the inputs and the driver, run the
/// cases, and compare or record each group.
///
/// # Errors
///
/// Returns [`Failure::Setup`] for an unknown group, a pattern with
/// `record`, a Vulkan stack or `deqp-vk` dist the runner cannot use, a
/// missing expected file of `run`, a probe that does not show the named
/// driver, a state directory that is not the runner's or is in use, a batch
/// whose `deqp-vk` reports nothing, and an I/O error, and
/// [`Failure::Interrupted`] once `stop` is set.
pub fn execute(
    groups: &Groups,
    options: &Options,
    caller_env: &[(OsString, OsString)],
    stop: &AtomicBool,
) -> Result<Conclusion, Failure> {
    let selected = groups.select(&options.groups).map_err(Failure::Setup)?;
    if matches!(options.mode, Mode::Record) && !options.cases.is_empty() {
        return Err(Failure::Setup(String::from(
            "record writes whole groups; --cases is for run",
        )));
    }
    let vulkan = Vulkan::open(&options.vulkan).map_err(Failure::Setup)?;
    let deqp = Deqp::open(&options.deqp).map_err(Failure::Setup)?;
    let mut components = vulkan.components.clone();
    components.insert(String::from("vk-gl-cts"), deqp.commit.clone());
    let triage = Triage::load(&options.expected.join(TRIAGE_FILE)).map_err(Failure::Setup)?;
    let mustpass = deqp.mustpass().map_err(Failure::Setup)?;
    let mustpass: Vec<&str> = mustpass.lines().filter(|line| !line.is_empty()).collect();
    let mut plans = Vec::new();
    for group in selected {
        let mut cases = group.select(&mustpass);
        if cases.is_empty() {
            return Err(Failure::Setup(format!(
                "deqp-vk group {} selects no case of the mustpass",
                group.name
            )));
        }
        let whole = options.cases.is_empty();
        if !whole {
            cases.retain(|case| options.cases.iter().any(|pattern| glob(pattern, case)));
        }
        let path = options.expected.join(format!("{}.txt", group.name));
        let old = Expected::load(&path, &group.name).map_err(Failure::Setup)?;
        if matches!(options.mode, Mode::Run) && old.is_none() {
            return Err(Failure::Setup(format!(
                "no expected results {}: record them first",
                path.display()
            )));
        }
        plans.push(Planned {
            group,
            cases: cases.into_iter().map(str::to_owned).collect(),
            whole,
            old,
            path,
        });
    }
    if union(&plans).is_empty() {
        return Err(Failure::Setup(String::from(
            "--cases matches no case of the groups",
        )));
    }
    let state = State::claim_tree(&options.state, DEQP_STATE_TREE).map_err(Failure::Setup)?;
    refuse_running(&deqp.program)?;
    let work = state.work();
    layout::fresh_dir(&work, DEQP_WORK_TREE).map_err(Failure::Setup)?;
    let run_dir = state.runs().join(layout::run_id(SystemTime::now()));
    fs::create_dir_all(&run_dir).map_err(|e| setup_io(&run_dir, &e))?;
    println!("conformance: raw logs in {}", run_dir.display());
    let started = Instant::now();
    let mut total = 0;
    let mut conclusion: Option<Conclusion> = None;
    // The groups of one `kk_debug` setting run as a run of their own, so a
    // case of two groups with different settings runs once in each, and no
    // process mixes the settings.
    for setting in settings(&plans) {
        let plans: Vec<&Planned<'_>> = plans
            .iter()
            .filter(|plan| plan.group.kk_debug.as_deref() == setting)
            .collect();
        let dir = setting.map_or_else(
            || run_dir.clone(),
            |flags| run_dir.join(format!("kk_debug-{}", flags.replace(',', "+"))),
        );
        fs::create_dir_all(&dir).map_err(|e| setup_io(&dir, &e))?;
        let env =
            environment::deqp_env(caller_env.iter().cloned(), &vulkan.icd_dir, &work, setting);
        let runner = Runner {
            deqp: &deqp,
            vulkan: &vulkan,
            env: &env,
            kk_debug: setting,
            work: &work,
            run_dir: &dir,
            options,
            stop,
        };
        let part = runner.run_plans(&plans, &components, &triage)?;
        total += union_of(&plans).len();
        conclusion = Some(match conclusion {
            Some(before) => combine(before, part),
            None => part,
        });
    }
    println!(
        "conformance: deqp-vk: {total} cases in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    conclusion.ok_or_else(|| Failure::Setup(String::from("no deqp-vk group selected")))
}

/// The `kk_debug` settings of the plans that have a case, each once, unset
/// first. A setting whose groups `--cases` left without a case has nothing
/// to run, and so needs no probe.
fn settings<'p>(plans: &'p [Planned<'_>]) -> Vec<Option<&'p str>> {
    let mut settings: Vec<Option<&str>> = Vec::new();
    for plan in plans {
        let setting = plan.group.kk_debug.as_deref();
        if !plan.cases.is_empty() && !settings.contains(&setting) {
            settings.push(setting);
        }
    }
    settings.sort_by_key(Option::is_some);
    settings
}

/// The conclusion of two runs of one invocation: the worse verdict, or the
/// unstable cases of both records.
fn combine(a: Conclusion, b: Conclusion) -> Conclusion {
    match (a, b) {
        (Conclusion::Run(a), Conclusion::Run(b)) => Conclusion::Run(a.max(b)),
        (Conclusion::Record { unstable: a }, Conclusion::Record { unstable: b }) => {
            Conclusion::Record { unstable: a + b }
        }
        (a, _) => a,
    }
}

impl Runner<'_> {
    /// Probe the driver with this runner's environment, run the cases of
    /// `plans`, and compare or record each of them.
    fn run_plans(
        &self,
        plans: &[&Planned<'_>],
        components: &BTreeMap<String, String>,
        triage: &Triage,
    ) -> Result<Conclusion, Failure> {
        let options = self.options;
        let all = union_of(plans);
        self.probe()?;
        println!(
            "conformance: deqp-vk: {} cases of {} groups, {} at a time in batches of {}",
            all.len(),
            plans.len(),
            options.jobs,
            options.batch
        );
        let mut isolate = known_crashes(plans);
        let first = self.run_cases("first", &all, &isolate)?;
        // The second pass runs alone every case that crashed or stalled in
        // the first, so its own crashes cannot change the result of another
        // case.
        isolate.extend(
            first
                .results
                .iter()
                .filter(|(_, status)| status.is_crash_or_stall())
                .map(|(case, _)| case.clone())
                .chain(first.unstable.keys().cloned()),
        );
        match options.mode {
            Mode::Run => run_groups(self, plans, first, &isolate, components, triage),
            Mode::Record => record_groups(self, plans, &first, &isolate, components, triage),
        }
    }
}

/// Refuse to start while a process of `program` runs: one a run left
/// behind when a second signal ended it, or one started by hand. Either
/// shares the GPU with the run's processes and can lose its device next to
/// them; the runner cannot tell its own from another's, so it ends none.
fn refuse_running(program: &Path) -> Result<(), Failure> {
    let pids = procs::pids_of(program).map_err(Failure::Setup)?;
    if pids.is_empty() {
        return Ok(());
    }
    let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
    Err(Failure::Setup(format!(
        "{} runs already (pid {}); end it, for a run interrupted twice with `kill -9 {}`, and \
         start again",
        program.display(),
        pids.join(", "),
        pids.join(" ")
    )))
}

/// The cases of the plans whose expected results say crash or stall.
fn known_crashes(plans: &[&Planned<'_>]) -> HashSet<String> {
    plans
        .iter()
        .filter_map(|plan| plan.old.as_ref().map(|old| (plan, old)))
        .flat_map(|(plan, old)| {
            plan.cases
                .iter()
                .filter(|case| old.status(case).is_crash_or_stall())
        })
        .cloned()
        .collect()
}

/// The cases of every plan, each once, in the order they first appear.
fn union(plans: &[Planned<'_>]) -> Vec<String> {
    union_of(&plans.iter().collect::<Vec<_>>())
}

/// [`union`] of plans held by reference.
fn union_of(plans: &[&Planned<'_>]) -> Vec<String> {
    let mut seen = HashSet::new();
    plans
        .iter()
        .flat_map(|plan| plan.cases.iter())
        .filter(|case| seen.insert(case.as_str()))
        .cloned()
        .collect()
}

/// `run`: diff each group, run the gating cases again, and decide.
fn run_groups(
    runner: &Runner<'_>,
    plans: &[&Planned<'_>],
    first: Ran,
    isolate: &HashSet<String>,
    components: &BTreeMap<String, String>,
    triage: &Triage,
) -> Result<Conclusion, Failure> {
    let Ran {
        results: first,
        unstable: mut pass_unstable,
    } = first;
    let report = |results: &BTreeMap<String, Status>, plan: &Planned<'_>| {
        let cases: Vec<&str> = plan.cases.iter().map(String::as_str).collect();
        plan.old
            .as_ref()
            .map(|old| expected::diff(old, &cases, plan.whole, results, components, triage))
    };
    // Each case whose change gates, with the status its group expects.
    let mut wanted: BTreeMap<String, Status> = BTreeMap::new();
    for plan in plans {
        if let Some(report) = report(&first, plan) {
            for change in &report.changes {
                if change.run.is_some() && change.class().is_some() {
                    wanted.entry(change.case.clone()).or_insert(change.expected);
                }
            }
        }
    }
    let again: Vec<String> = wanted.keys().cloned().collect();
    let second = if again.is_empty() {
        BTreeMap::new()
    } else {
        println!(
            "conformance: deqp-vk: running {} changed cases again",
            again.len()
        );
        let ran = runner.run_cases("second", &again, isolate)?;
        pass_unstable.extend(ran.unstable);
        ran.results
    };
    let mut results = first;
    let mut unstable = Vec::new();
    for (case, &expected) in &wanted {
        if second.get(case) == Some(&expected)
            && let Some(first) = results.insert(case.clone(), expected)
        {
            unstable.push((case.clone(), first));
        }
    }
    let mut verdict = Verdict::Match;
    for plan in plans {
        if let Some(report) = report(&results, plan) {
            print!("{}", report.text());
            verdict = verdict.max(report.verdict());
        }
    }
    for (case, first) in &unstable {
        println!(
            "unstable deqp-vk {case}: the first run gave {first}, the second the expected result"
        );
    }
    pass_unstable.retain(|case, _| !triage.is_flaky(case));
    print_pass_unstable(&pass_unstable);
    if !unstable.is_empty() || !pass_unstable.is_empty() {
        verdict = verdict.max(Verdict::Unstable);
    }
    Ok(Conclusion::Run(verdict))
}

/// Print each case [`Runner::run_cases`] found unstable.
fn print_pass_unstable(unstable: &BTreeMap<String, (Status, Status)>) {
    for (case, (first, then)) in unstable {
        println!(
            "unstable deqp-vk {case}: {first} next to other processes, then {then}, and no case \
             lost the device alone"
        );
    }
}

/// `results` with the worse of the two statuses of each `unstable` case.
fn with_worse(
    mut results: BTreeMap<String, Status>,
    unstable: &BTreeMap<String, (Status, Status)>,
) -> BTreeMap<String, Status> {
    for (case, &(first, then)) in unstable {
        let worse = if then.rank() > first.rank() {
            then
        } else {
            first
        };
        results.insert(case.clone(), worse);
    }
    results
}

/// `record`: run the cases that differ from the old results again and
/// write each group's file.
fn record_groups(
    runner: &Runner<'_>,
    plans: &[&Planned<'_>],
    first: &Ran,
    isolate: &HashSet<String>,
    components: &BTreeMap<String, String>,
    triage: &Triage,
) -> Result<Conclusion, Failure> {
    // An unstable case of a pass is written with the worse of its results.
    let mut pass_unstable = first.unstable.clone();
    let first = &with_worse(first.results.clone(), &first.unstable);
    let mut again = Vec::new();
    let mut seen = HashSet::new();
    for plan in plans {
        for case in &plan.cases {
            let old = plan
                .old
                .as_ref()
                .map_or(Status::Pass, |old| old.status(case));
            if first.get(case) != Some(&old) && seen.insert(case.as_str()) {
                again.push(case.clone());
            }
        }
    }
    let mut second = if again.is_empty() {
        BTreeMap::new()
    } else {
        println!(
            "conformance: deqp-vk: running {} cases whose result changed again",
            again.len()
        );
        let ran = runner.run_cases("second", &again, isolate)?;
        let second = with_worse(ran.results, &ran.unstable);
        pass_unstable.extend(ran.unstable);
        second
    };
    // A case the triage marks flaky runs alone FLAKY_RUNS more times, and
    // the worst of its results is written, so its expected result is the
    // one that gates (a crash or stall) and its better results do not.
    let flaky: Vec<String> = union_of(plans)
        .into_iter()
        .filter(|case| triage.is_flaky(case))
        .collect();
    for run in 1..=FLAKY_RUNS {
        if flaky.is_empty() {
            break;
        }
        println!(
            "conformance: deqp-vk: running the {} flaky cases alone, {run} of {FLAKY_RUNS}",
            flaky.len()
        );
        let results = runner.run_batches(&format!("flaky-{run}"), &flaky, 1)?;
        for (case, status) in results {
            let worst = [
                Some(status),
                first.get(&case).copied(),
                second.get(&case).copied(),
            ]
            .into_iter()
            .flatten()
            .max_by_key(|status| status.rank())
            .unwrap_or(status);
            second.insert(case, worst);
        }
    }
    let mut written = Vec::new();
    let mut unstable = HashSet::new();
    for plan in plans {
        let cases: Vec<&str> = plan.cases.iter().map(String::as_str).collect();
        let (new, changed) = expected::record(&plan.group.name, &cases, first, &second, components);
        for (case, one, two) in changed {
            if triage.is_flaky(&case) {
                println!("flaky deqp-vk {case}: {one}, then {two}; recorded the worse");
                continue;
            }
            println!("unstable deqp-vk {case}: {one}, then {two}; recorded the worse");
            unstable.insert(case);
        }
        let mut counts = BTreeMap::new();
        for case in &cases {
            let status = new.status(case);
            if status != Status::Pass {
                *counts.entry(status).or_insert(0usize) += 1;
            }
        }
        let mut summary = format!("deqp-vk {}: {} cases", plan.group.name, new.cases);
        for (status, count) in counts {
            let _ = write!(summary, ", {count} {status}");
        }
        println!("{summary}");
        let staged = plan.path.with_extension("txt.new");
        fs::write(&staged, new.to_text()).map_err(|e| setup_io(&staged, &e))?;
        written.push((staged, &plan.path));
    }
    for (staged, path) in written {
        fs::rename(&staged, path).map_err(|e| setup_io(path, &e))?;
        println!("conformance: wrote {}", path.display());
    }
    pass_unstable.retain(|case, _| !triage.is_flaky(case));
    print_pass_unstable(&pass_unstable);
    unstable.extend(pass_unstable.into_keys());
    Ok(Conclusion::Record {
        unstable: unstable.len(),
    })
}

/// A setup failure for an I/O error on `path`.
fn setup_io(path: &Path, e: &std::io::Error) -> Failure {
    Failure::Setup(format!("{}: {e}", path.display()))
}

/// What every `deqp-vk` process of an invocation shares.
struct Runner<'a> {
    deqp: &'a Deqp,
    vulkan: &'a Vulkan,
    env: &'a [(OsString, OsString)],
    /// The `MESA_KK_DEBUG` of `env`, if any.
    kk_debug: Option<&'a str>,
    work: &'a Path,
    run_dir: &'a Path,
    options: &'a Options,
    stop: &'a AtomicBool,
}

/// How one process of a batch ended, for the batch's next step.
struct Attempt {
    results: Vec<(String, Status)>,
    unfinished: Option<String>,
    ending: Ending,
}

impl Runner<'_> {
    /// The arguments of a `deqp-vk` process that runs the cases of
    /// `caselist` and writes its test log to `qpa`.
    fn args(&self, caselist: &Path, qpa: &Path) -> Vec<OsString> {
        let mut args: Vec<OsString> = [
            "--deqp-log-images=disable",
            "--deqp-log-shader-sources=disable",
            "--deqp-shadercache=disable",
            "--deqp-log-flush=disable",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        for (option, path) in [
            ("--deqp-caselist-file=", caselist),
            ("--deqp-log-filename=", qpa),
            ("--deqp-archive-dir=", &self.deqp.dir),
            ("--deqp-vk-library-path=", &self.vulkan.loader),
        ] {
            let mut arg = OsString::from(option);
            arg.push(path);
            args.push(arg);
        }
        args
    }

    /// Run the probe: [`PROBE_CASE`] with the loader's driver messages on,
    /// which must pass and show that the loader found the driver in the
    /// named manifest directory and no other.
    fn probe(&self) -> Result<(), Failure> {
        let caselist = self.run_dir.join("probe.txt");
        fs::write(&caselist, format!("{PROBE_CASE}\n")).map_err(|e| setup_io(&caselist, &e))?;
        let qpa = self.run_dir.join("probe.qpa");
        let args = self.args(&caselist, &qpa);
        let mut env = self.env.to_vec();
        env.push((OsString::from("VK_LOADER_DEBUG"), OsString::from("driver")));
        let log = self.run_dir.join("probe.log");
        let finished = self.spawn(&args, &env, &log, "probe")?;
        let text = String::from_utf8_lossy(&finished.output);
        let see = format!("its output is in {}", log.display());
        let output = log::parse(&text);
        if output.results != [(PROBE_CASE.to_owned(), Status::Pass)] {
            return Err(Failure::Setup(format!(
                "deqp-vk probe: {PROBE_CASE} did not pass on {}; {see}",
                self.vulkan.icd_dir.display()
            )));
        }
        let manifests = probe::manifests(&text);
        let icd_dir = self.vulkan.icd_dir.display().to_string();
        if manifests.is_empty() || manifests.iter().any(|path| !path.starts_with(&icd_dir)) {
            return Err(Failure::Setup(format!(
                "deqp-vk probe: the loader did not take its drivers from {icd_dir} alone (it \
                 found {manifests:?}); {see}"
            )));
        }
        let drivers = probe::drivers(&text);
        let dir = self.vulkan.dir.display().to_string();
        let Some((device, driver)) = drivers.first() else {
            return Err(Failure::Setup(format!(
                "deqp-vk probe: the loader names no driver of a device; {see}"
            )));
        };
        if drivers
            .iter()
            .any(|(_, path)| !path.starts_with(&dir) || path != driver)
        {
            return Err(Failure::Setup(format!(
                "deqp-vk probe: the devices did not all run on the driver of {dir} (the loader \
                 used {drivers:?}); {see}"
            )));
        }
        let qpa_text = fs::read_to_string(&qpa).map_err(|e| setup_io(&qpa, &e))?;
        let version = probe::driver_version(&qpa_text).ok_or_else(|| {
            Failure::Setup(format!(
                "deqp-vk probe: {} has no driverVersion",
                qpa.display()
            ))
        })?;
        let debug = self
            .kk_debug
            .map_or_else(String::new, |flags| format!(", with MESA_KK_DEBUG={flags}"));
        println!(
            "conformance: deqp-vk runs on {device} with {driver}, driverVersion {version}, \
             through {} and {}{debug}",
            self.vulkan.loader.display(),
            manifests.join(", ")
        );
        Ok(())
    }

    /// Run `cases` and return the status of each; `pass` names the pass in
    /// the logs.
    ///
    /// A case that loses the device can take the devices of the other
    /// `deqp-vk` processes running at the time with it, which then fail or
    /// lose their device in cases that pass alone. So the cases of
    /// `isolate`, those known to crash or stall, run after the others, one
    /// process at a time, in `<pass>-alone/`; the others run on
    /// [`Options::jobs`] threads, and each of them that crashed or stalled
    /// there, a suspect, runs again alone too. When a suspect crashed, the
    /// others that failed run again next to each other without the
    /// suspects, in `<pass>-again/`.
    ///
    /// When a suspect crashes or stalls alone as well, it took the others
    /// with it, and the second result of each case counts. When none does,
    /// no case explains the first crash: every suspect and every case
    /// whose result changed in `<pass>-again/` is unstable, and is returned
    /// with its two results.
    fn run_cases(
        &self,
        pass: &str,
        cases: &[String],
        isolate: &HashSet<String>,
    ) -> Result<Ran, Failure> {
        let (mut alone, together): (Vec<String>, Vec<String>) = cases
            .iter()
            .cloned()
            .partition(|case| isolate.contains(case));
        let mut results = self.run_batches(pass, &together, self.options.jobs)?;
        let parallel = results.clone();
        // With one job the others ran alone already.
        let suspects: Vec<String> = together
            .iter()
            .filter(|case| {
                self.options.jobs > 1
                    && results
                        .get(*case)
                        .is_some_and(|status| status.is_crash_or_stall())
            })
            .cloned()
            .collect();
        if alone.is_empty() && suspects.is_empty() {
            return Ok(Ran {
                results,
                unstable: BTreeMap::new(),
            });
        }
        let mut suspects = suspects;
        let mut rerun = Vec::new();
        if !suspects.is_empty() {
            // A lost device can also make a case of another process fail, so
            // those run again next to each other, without the suspects.
            let crashed: HashSet<&String> = suspects.iter().collect();
            let failed: Vec<String> = together
                .iter()
                .filter(|case| {
                    !crashed.contains(case)
                        && results
                            .get(*case)
                            .is_some_and(|status| matches!(status, Status::Fail | Status::Warn))
                })
                .cloned()
                .collect();
            println!(
                "conformance: deqp-vk: running {} cases that did not pass again, without the {} \
                 that crashed or stalled",
                failed.len(),
                suspects.len()
            );
            let again = self.run_batches(&format!("{pass}-again"), &failed, self.options.jobs)?;
            suspects.extend(
                again
                    .iter()
                    .filter(|(_, status)| status.is_crash_or_stall())
                    .map(|(case, _)| case.clone()),
            );
            rerun = failed;
            results.extend(again);
        }
        println!(
            "conformance: deqp-vk: running alone {} cases known to crash or stall and {} that \
             crashed or stalled next to other processes",
            alone.len(),
            suspects.len()
        );
        alone.extend(suspects.iter().cloned());
        let alone_results = self.run_batches(&format!("{pass}-alone"), &alone, 1)?;
        results.extend(alone_results);
        let unstable = changed_without_cause(&suspects, &rerun, &parallel, &results);
        if unstable.is_empty() && !suspects.is_empty() {
            println!(
                "conformance: deqp-vk: a case lost the device alone too; the results alone and \
                 without it count"
            );
        } else if !unstable.is_empty() {
            println!(
                "conformance: deqp-vk: no case that crashed or stalled next to others did so \
                 alone; {} cases are unstable",
                unstable.len()
            );
        }
        Ok(Ran { results, unstable })
    }

    /// Run `cases`, in batches on `jobs` threads, and return the status of
    /// each. `pass` names the directory of their logs. With one job, each
    /// lost device is followed by [`Self::recover`].
    fn run_batches(
        &self,
        pass: &str,
        cases: &[String],
        jobs: usize,
    ) -> Result<BTreeMap<String, Status>, Failure> {
        if cases.is_empty() {
            return Ok(BTreeMap::new());
        }
        let dir = self.run_dir.join(pass);
        fs::create_dir_all(&dir).map_err(|e| setup_io(&dir, &e))?;
        let batches: Vec<&[String]> = cases.chunks(self.options.batch.max(1)).collect();
        let total = batches.len();
        let queue = Mutex::new(batches.into_iter().enumerate().collect::<VecDeque<_>>());
        let results = Mutex::new(BTreeMap::new());
        let failure: Mutex<Option<Failure>> = Mutex::new(None);
        let halt = AtomicBool::new(false);
        let done = Mutex::new(0usize);
        thread::scope(|scope| {
            for _ in 0..jobs.max(1) {
                scope.spawn(|| {
                    loop {
                        if halt.load(Ordering::SeqCst) || self.stop.load(Ordering::SeqCst) {
                            return;
                        }
                        let next = queue.lock().map_or(None, |mut queue| queue.pop_front());
                        let Some((index, batch)) = next else {
                            return;
                        };
                        let start = Instant::now();
                        match self.run_batch(&dir, index, batch, jobs <= 1) {
                            Ok(batch_results) => {
                                let summary = summary(&batch_results);
                                let finished = done.lock().map_or(0, |mut done| {
                                    *done += 1;
                                    *done
                                });
                                println!(
                                    "deqp-vk {pass} batch {index}: {finished}/{total} done, {} \
                                     cases{summary}, {:.1} s",
                                    batch.len(),
                                    start.elapsed().as_secs_f64()
                                );
                                if let Ok(mut results) = results.lock() {
                                    results.extend(batch_results);
                                }
                            }
                            Err(e) => {
                                halt.store(true, Ordering::SeqCst);
                                if let Ok(mut failure) = failure.lock() {
                                    failure.get_or_insert(e);
                                }
                                return;
                            }
                        }
                    }
                });
            }
        });
        if self.stop.load(Ordering::SeqCst) {
            return Err(Failure::Interrupted);
        }
        if let Some(failure) = failure.into_inner().ok().flatten() {
            return Err(failure);
        }
        let results = results
            .into_inner()
            .map_err(|_| Failure::Setup(String::from("a batch thread panicked")))?;
        if let Some(case) = cases.iter().find(|case| !results.contains_key(*case)) {
            return Err(Failure::Setup(format!("deqp-vk gave no result for {case}")));
        }
        Ok(results)
    }

    /// Run one batch to its end: a process for its cases, then one for the
    /// cases after the one it died or stalled in, until each has a status.
    /// With `recover`, a process that lost its device is followed by
    /// [`Self::recover`] before the next one.
    fn run_batch(
        &self,
        dir: &Path,
        index: usize,
        batch: &[String],
        recover: bool,
    ) -> Result<Vec<(String, Status)>, Failure> {
        let log = dir.join(format!("batch-{index}.log"));
        let mut remaining: Vec<&String> = batch.iter().collect();
        let mut results = Vec::new();
        let mut attempt = 0;
        while !remaining.is_empty() {
            attempt += 1;
            let caselist = dir.join(format!("batch-{index}.txt"));
            let mut list = String::new();
            for case in &remaining {
                let _ = writeln!(list, "{case}");
            }
            fs::write(&caselist, list).map_err(|e| setup_io(&caselist, &e))?;
            let args = self.args(&caselist, Path::new("/dev/null"));
            let what = format!("attempt {attempt}, {} cases", remaining.len());
            let finished = self.spawn(&args, self.env, &log, &what)?;
            let output = log::parse(&String::from_utf8_lossy(&finished.output));
            let step = next_step(
                &remaining,
                &Attempt {
                    results: output.results,
                    unfinished: output.unfinished,
                    ending: finished.ending,
                },
            )
            .map_err(|e| Failure::Setup(format!("{e}; its output is in {}", log.display())))?;
            let device_lost = step
                .results
                .iter()
                .any(|(_, status)| status.is_crash_or_stall());
            results.extend(step.results);
            remaining = step.remaining;
            if recover && device_lost {
                self.recover(dir, index, &log)?;
            }
        }
        Ok(results)
    }

    /// Wait until the device works again after a case lost it: run
    /// [`CANARY`] in a process of its own until it passes, at most
    /// [`RECOVERY_TRIES`] times. A process started right after another lost
    /// its device can lose its own at the first submission, in a case that
    /// passes otherwise.
    fn recover(&self, dir: &Path, index: usize, log: &Path) -> Result<(), Failure> {
        let caselist = dir.join(format!("batch-{index}.canary.txt"));
        fs::write(&caselist, format!("{CANARY}\n")).map_err(|e| setup_io(&caselist, &e))?;
        let args = self.args(&caselist, Path::new("/dev/null"));
        for attempt in 1..=RECOVERY_TRIES {
            let what = format!("recovery {attempt}, {CANARY}");
            let finished = self.spawn(&args, self.env, log, &what)?;
            let output = log::parse(&String::from_utf8_lossy(&finished.output));
            if output.results == [(CANARY.to_owned(), Status::Pass)] {
                return Ok(());
            }
        }
        Err(Failure::Setup(format!(
            "the device did not work again after a lost device: {CANARY} did not pass in \
             {RECOVERY_TRIES} processes; the output is in {}",
            log.display()
        )))
    }

    /// Run `deqp-vk` with `args` and `env` under the watchdog, and append
    /// its output and how it ended to `log`.
    fn spawn(
        &self,
        args: &[OsString],
        env: &[(OsString, OsString)],
        log: &Path,
        what: &str,
    ) -> Result<exec::Finished, Failure> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(Failure::Interrupted);
        }
        let spec = Spec {
            program: &self.deqp.program,
            args,
            env,
            cwd: self.work,
        };
        let sampler = Sampler {
            tool: PathBuf::from(SAMPLE_TOOL),
            dir: log.parent().unwrap_or(self.run_dir).to_path_buf(),
            stem: log
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned()),
            wineserver: None,
        };
        let limits = &self.options.limits;
        let finished =
            exec::run_tree(&spec, limits, Some(&sampler), self.stop).map_err(|e| match e {
                exec::Error::Interrupted => Failure::Interrupted,
                e => Failure::Setup(format!("{} {e}", self.deqp.program.display())),
            })?;
        let mut text = format!("[conformance] {what}: {}\n", self.deqp.program.display());
        text.push_str(&String::from_utf8_lossy(&finished.output));
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&finished.trailer(limits));
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .map_err(|e| setup_io(log, &e))?;
        Ok(finished)
    }
}

/// The non-pass counts of a batch's results, as `, <n> <status>` each.
fn summary(results: &[(String, Status)]) -> String {
    let mut text = String::new();
    for status in Status::ALL.into_iter().skip(1) {
        let count = results.iter().filter(|(_, s)| *s == status).count();
        if count > 0 {
            let _ = write!(text, ", {count} {status}");
        }
    }
    text
}

/// What a batch does after one process: the statuses it settled and the
/// cases still to run.
struct Step<'c> {
    results: Vec<(String, Status)>,
    remaining: Vec<&'c String>,
}

/// Settle the cases of `remaining` an `attempt` reported, give the case it
/// ended in a crash or stall, and keep the rest for the next process.
///
/// A result for a case outside `remaining` is ignored. A process that ends
/// without a case in progress and without reporting anything while cases
/// remain would loop forever, so that is an error: deqp-vk could not start
/// its run (no device, a case it does not know).
fn next_step<'c>(remaining: &[&'c String], attempt: &Attempt) -> Result<Step<'c>, String> {
    let mut settled: BTreeMap<&str, Status> = BTreeMap::new();
    for (case, status) in &attempt.results {
        if remaining.contains(&case) {
            settled.insert(case.as_str(), *status);
        }
    }
    let ended_in = attempt
        .unfinished
        .as_deref()
        .filter(|case| remaining.iter().any(|known| known.as_str() == *case));
    if let Some(case) = ended_in {
        let status = if attempt.ending == Ending::Stalled {
            Status::Stall
        } else {
            Status::Crash
        };
        settled.insert(case, status);
    } else if settled.is_empty() {
        return Err(format!(
            "deqp-vk ended ({}) without running any of its {} cases",
            ending_text(attempt.ending),
            remaining.len()
        ));
    }
    let results = remaining
        .iter()
        .filter_map(|case| {
            settled
                .get(case.as_str())
                .map(|status| ((*case).clone(), *status))
        })
        .collect();
    let left = remaining
        .iter()
        .filter(|case| !settled.contains_key(case.as_str()))
        .copied()
        .collect();
    Ok(Step {
        results,
        remaining: left,
    })
}

/// What [`Runner::run_cases`] gave.
struct Ran {
    /// The status that counts for each case.
    results: BTreeMap<String, Status>,
    /// The unstable cases, each with its first status and the one that
    /// counts.
    unstable: BTreeMap<String, (Status, Status)>,
}

/// The unstable cases of a pass: when none of the `suspects`, the cases
/// that crashed or stalled next to others in the `parallel` results, did so
/// again alone in the `last` results, each suspect, and each of the
/// `rerun` cases whose last result differs from its parallel one, with
/// both results. Nothing when a suspect crashed or stalled alone too, or
/// there is none.
fn changed_without_cause(
    suspects: &[String],
    rerun: &[String],
    parallel: &BTreeMap<String, Status>,
    last: &BTreeMap<String, Status>,
) -> BTreeMap<String, (Status, Status)> {
    let reproduced = suspects.iter().any(|case| {
        last.get(case)
            .is_some_and(|status| status.is_crash_or_stall())
    });
    if suspects.is_empty() || reproduced {
        return BTreeMap::new();
    }
    suspects
        .iter()
        .chain(rerun)
        .filter_map(|case| {
            let first = *parallel.get(case)?;
            let then = *last.get(case)?;
            (first != then).then(|| (case.clone(), (first, then)))
        })
        .collect()
}

fn ending_text(ending: Ending) -> String {
    match ending {
        Ending::Exited(code) => format!("exit code {code}"),
        Ending::Signaled(signal) => format!("signal {signal}"),
        Ending::Stalled => String::from("stalled"),
    }
}

#[cfg(test)]
mod tests;
