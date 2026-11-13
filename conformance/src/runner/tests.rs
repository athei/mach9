use compatdb_core::{Dxgi, OpenGl};

use super::*;

const PASS: &str = include_str!("../../tests/fixtures/output/pass.txt");

fn config() -> Config {
    Config::builtin().unwrap()
}

fn suites(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn function(name: &str) -> Key {
    Key::Function(name.to_owned())
}

#[test]
fn a_plan_takes_every_leg_an_exe_is_built_for() {
    let config = config();
    let plans = plan(&config, &suites(&["d3d12", "d3d8"]), None, None).unwrap();
    let got: Vec<(&str, Vec<PeDir>, bool)> = plans
        .iter()
        .map(|plan| {
            (
                plan.exe.name.as_str(),
                plan.legs.clone(),
                matches!(plan.scope, Scope::All),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("d3d12", vec![PeDir::I386, PeDir::X86_64], true),
            ("d3d8", vec![PeDir::I386], true),
        ]
    );
    assert_eq!(
        plans[0].scope_text(),
        "legs=i386-windows,x86_64-windows only=all"
    );
}

#[test]
fn a_leg_narrows_the_scope_unless_it_is_the_only_one() {
    let config = config();
    let plans = plan(&config, &suites(&["all"]), Some(PeDir::X86_64), None).unwrap();
    assert!(plans.iter().all(|plan| plan.exe.name != "d3d8"));
    assert!(plans.iter().all(|plan| plan.legs == [PeDir::X86_64]));
    assert!(matches!(plans[0].scope, Scope::Leg(PeDir::X86_64)));
    let d3d8 = plan(&config, &suites(&["d3d8"]), Some(PeDir::I386), None).unwrap();
    assert!(matches!(d3d8[0].scope, Scope::All));
    let all = plan(&config, &suites(&["all", "d3d11"]), None, None).unwrap();
    assert_eq!(all.len(), config.exes.len());
}

#[test]
fn only_selects_keys_of_one_exe_on_each_leg() {
    let config = config();
    let plans = plan(
        &config,
        &suites(&["d3d12"]),
        None,
        Some("d3d12/test_ordinals,d3d12/test_interfaces"),
    )
    .unwrap();
    let Scope::Tests(ids) = &plans[0].scope else {
        panic!("not a scope of tests");
    };
    assert_eq!(ids.len(), 4);
    assert!(ids.contains(&TestId {
        leg: PeDir::I386,
        unit: String::from("d3d12"),
        key: function("test_ordinals"),
    }));
    assert_eq!(
        plans[0].scope_text(),
        "legs=i386-windows,x86_64-windows only=d3d12/test_interfaces,d3d12/test_ordinals"
    );
    let ddraw = plan(&config, &suites(&["ddraw"]), None, Some("d3d/*")).unwrap();
    assert!(ddraw[0].runs_unit("d3d"));
    assert!(!ddraw[0].runs_unit("ddraw7"));
}

#[test]
fn bad_selections_are_errors() {
    let config = config();
    for (names, leg, only, expected) in [
        (&[][..], None, None, "no suite given"),
        (
            &["d3d13"][..],
            None,
            None,
            "unknown suite \"d3d13\"; the suites are ddraw",
        ),
        (
            &["d3d8"][..],
            Some(PeDir::X86_64),
            None,
            "d3d8 is not built for x86_64-windows",
        ),
        (
            &["d3d11", "dxgi"][..],
            None,
            Some("d3d11/test_a"),
            "--only needs exactly one suite",
        ),
        (
            &["all"][..],
            None,
            Some("d3d11/test_a"),
            "--only needs exactly one suite",
        ),
        (
            &["d3d12"][..],
            None,
            Some("test_ordinals"),
            "is not <unit>/<key>",
        ),
        (
            &["d3d12"][..],
            None,
            Some("d3d12/test_a,"),
            "\"\" is not <unit>/<key>",
        ),
        (
            &["d3d12"][..],
            None,
            Some("d3d11/test_a"),
            "d3d12 has no unit \"d3d11\"",
        ),
        (
            &["d3d12"][..],
            None,
            Some("d3d12/test a"),
            "\"test a\" is not a key",
        ),
        (
            &["d3d12"][..],
            None,
            Some("d3d12/*"),
            "d3d12 has keys of its own",
        ),
        (
            &["ddraw"][..],
            None,
            Some("d3d/test_a"),
            "select it as d3d/*",
        ),
    ] {
        let error = plan(&config, &suites(names), leg, only)
            .err()
            .unwrap_or_default();
        assert!(error.contains(expected), "{names:?} {only:?}: {error}");
    }
}

fn expect(dxgi: Option<Dxgi>, opengl: Option<OpenGl>, wined3d: Option<Wined3d>) -> Expect {
    Expect {
        dxgi,
        opengl,
        wined3d,
    }
}

/// The pass fixture's d3d11 process has DXMT, Mesa's EGL and an unset
/// wined3d renderer, which is GL.
#[test]
fn the_expected_implementations_are_checked() {
    let exe = "d3d11_test.exe";
    check_expect(&expect(None, None, None), PASS, exe).unwrap();
    check_expect(
        &expect(Some(Dxgi::Dxmt), Some(OpenGl::Egl), Some(Wined3d::Gl)),
        PASS,
        exe,
    )
    .unwrap();
    for (expected, error) in [
        (
            expect(Some(Dxgi::Wined3d), None, None),
            "d3d11_test.exe runs with dxgi = dxmt, not wined3d",
        ),
        (
            expect(None, Some(OpenGl::Cgl), None),
            "d3d11_test.exe runs with opengl = egl, not cgl",
        ),
        (
            expect(None, None, Some(Wined3d::Vulkan)),
            "d3d11_test.exe runs with wined3d = gl, not vulkan",
        ),
    ] {
        assert_eq!(
            check_expect(&expected, PASS, exe).err().unwrap_or_default(),
            error
        );
    }
    let error = check_expect(
        &expect(Some(Dxgi::Dxmt), None, None),
        PASS,
        "d3d12_test.exe",
    )
    .err()
    .unwrap_or_default();
    assert_eq!(
        error,
        "cannot tell the dxgi of d3d12_test.exe: no compatdb block for the process"
    );
    let vulkan = PASS.replace(
        "wined3d = unset (upstream:",
        "wined3d = vulkan, WINE_COMPATDB line 1 (was",
    );
    let error = check_expect(&expect(None, None, Some(Wined3d::Gl)), &vulkan, exe)
        .err()
        .unwrap_or_default();
    assert_eq!(error, "d3d11_test.exe runs with wined3d = vulkan, not gl");
}

#[test]
fn exit_codes_follow_the_conclusion() {
    for (conclusion, code, line) in [
        (Conclusion::Run(Verdict::Match), 0, "conformance: match"),
        (
            Conclusion::Run(Verdict::Stale),
            1,
            "conformance: stale baseline",
        ),
        (
            Conclusion::Run(Verdict::Unstable),
            1,
            "conformance: unstable keys",
        ),
        (
            Conclusion::Run(Verdict::Regression),
            1,
            "conformance: regression",
        ),
        (
            Conclusion::Record { unstable: 0 },
            0,
            "conformance: recorded a stable baseline",
        ),
        (
            Conclusion::Record { unstable: 2 },
            1,
            "conformance: unstable keys: 2 were written",
        ),
    ] {
        assert_eq!(conclusion.exit_code(), code, "{conclusion:?}");
        assert!(conclusion.last_line().starts_with(line), "{conclusion:?}");
    }
    assert_eq!(Failure::Setup(String::from("x")).exit_code(), 2);
    assert_eq!(Failure::Interrupted.exit_code(), 130);
}

const fn outcome(status: Status, failed: u32) -> Outcome {
    Outcome {
        status,
        failed,
        sites: failed,
        todo_succeeded: 0,
        skipped: 0,
    }
}

const OK: Outcome = outcome(Status::Ok, 0);
const FAIL: Outcome = outcome(Status::Fail, 2);
const WORSE: Outcome = outcome(Status::Fail, 4);

fn id(leg: PeDir, key: &str) -> TestId {
    TestId {
        leg,
        unit: String::from("d3d12"),
        key: function(key),
    }
}

fn header() -> Header {
    Header::parse("wine-11.21", "dxmt=8c0b5f1\n").unwrap()
}

fn baseline(entries: &[(&TestId, Outcome)]) -> Baseline {
    Baseline {
        exe: String::from("d3d12"),
        header: header(),
        entries: map(entries),
    }
}

fn map(entries: &[(&TestId, Outcome)]) -> BTreeMap<TestId, Outcome> {
    entries
        .iter()
        .map(|(id, outcome)| ((*id).clone(), *outcome))
        .collect()
}

fn triage(text: &str) -> Triage {
    Triage::parse(text, "triage.txt").unwrap()
}

fn decide(
    old: &Baseline,
    first: &[(&TestId, Outcome)],
    second: &[(&TestId, Outcome)],
    triage_text: &str,
) -> RunDecision {
    decide_run(
        "d3d12",
        old,
        &header(),
        map(first),
        &map(second),
        &Scope::All,
        &triage(triage_text),
    )
}

/// `run` runs again what gates and the run listed: a regression and a
/// stale key, not a match, a flaky change, a new key or a dropped key.
#[test]
fn run_runs_again_the_listed_keys_whose_change_gates() {
    let [worse, better, same, flaky, new, dropped] =
        ["test_a", "test_b", "test_c", "test_d", "test_e", "test_f"]
            .map(|key| id(PeDir::I386, key));
    let old = baseline(&[
        (&worse, OK),
        (&better, FAIL),
        (&same, OK),
        (&flaky, OK),
        (&dropped, OK),
    ]);
    let first = map(&[
        (&worse, FAIL),
        (&better, OK),
        (&same, OK),
        (&flaky, FAIL),
        (&new, OK),
    ]);
    let report = diff::diff(
        "d3d12",
        Some(&old),
        &header(),
        &first,
        &Scope::All,
        &triage("d3d12/d3d12/test_d flaky\n"),
    );
    assert_eq!(run_again(&report), [(worse, OK), (better, FAIL)]);
}

/// A key whose second run gives the baseline's outcome is unstable and no
/// regression; one whose second run gives its first outcome again stays a
/// regression.
#[test]
fn run_takes_a_second_run_at_the_baseline_s_outcome_as_unstable() {
    let a = id(PeDir::I386, "test_a");
    let b = id(PeDir::X86_64, "test_a");
    let old = baseline(&[(&a, OK), (&b, OK)]);

    let decided = decide(&old, &[(&a, FAIL), (&b, OK)], &[(&a, OK)], "");
    assert_eq!(decided.unstable, [(a.clone(), FAIL)]);
    assert_eq!(decided.verdict, Verdict::Unstable);
    assert_eq!(decided.report.changes.len(), 0);
    assert_eq!(Conclusion::Run(decided.verdict).exit_code(), 1);

    let decided = decide(&old, &[(&a, FAIL), (&b, OK)], &[(&a, FAIL)], "");
    assert_eq!(decided.unstable, []);
    assert_eq!(decided.verdict, Verdict::Regression);

    let decided = decide(&old, &[(&a, FAIL), (&b, OK)], &[(&a, WORSE)], "");
    assert_eq!(decided.unstable, []);
    assert_eq!(decided.verdict, Verdict::Regression);
    assert_eq!(decided.report.changes[0].run, Some(FAIL));

    let decided = decide(&old, &[(&a, OK), (&b, OK)], &[(&a, FAIL)], "");
    assert_eq!(decided.verdict, Verdict::Match);
    assert_eq!(Conclusion::Run(decided.verdict).exit_code(), 0);
}

/// Without a second outcome, as when the second run of a stale key is
/// still missing, the first stands; a stale key that comes back to the
/// baseline is unstable, and a regression elsewhere still decides.
#[test]
fn run_combines_unstable_and_other_changes() {
    let a = id(PeDir::I386, "test_a");
    let b = id(PeDir::I386, "test_b");
    let old = baseline(&[(&a, FAIL), (&b, OK)]);
    let decided = decide(&old, &[(&a, OK), (&b, OK)], &[], "");
    assert_eq!(decided.verdict, Verdict::Stale);
    let decided = decide(&old, &[(&a, OK), (&b, FAIL)], &[(&a, FAIL), (&b, FAIL)], "");
    assert_eq!(decided.unstable, [(a, OK)]);
    assert_eq!(decided.verdict, Verdict::Regression);
}

/// A state directory with a resume file holding one record, a baselines
/// directory with an old `d3d12.txt`, and a pending record of a new one.
struct Finish {
    dir: PathBuf,
    state: State,
    baselines: PathBuf,
    pending: Pending,
    resume_file: PathBuf,
}

impl Finish {
    fn new(name: &str) -> Self {
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("conformance-test-finalize")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let state = State::claim(&dir.join("state")).unwrap();
        let baselines = dir.join("baselines");
        fs::create_dir_all(&baselines).unwrap();
        let a = id(PeDir::I386, "test_a");
        let old = baseline(&[(&a, OK)]);
        fs::write(baselines.join("d3d12.txt"), old.to_text()).unwrap();
        let first = map(&[(&a, FAIL)]);
        let merge = decide_record(
            "d3d12",
            Some(&old),
            &header(),
            &first,
            &BTreeMap::new(),
            &Scope::All,
        )
        .unwrap();
        let pending = Pending {
            exe: String::from("d3d12"),
            legs: vec![PeDir::I386],
            all: true,
            path: baselines.join("d3d12.txt"),
            merge,
            results: first,
        };
        let resume_file = state.resume().join("record-d3d12-i386-windows.txt");
        Self {
            dir,
            state,
            baselines,
            pending,
            resume_file,
        }
    }

    /// Open the resume file and append the first-pass record.
    fn resume(&self) -> Resume {
        let identity = Identity {
            mode: Mode::Record,
            exe: String::from("d3d12"),
            leg: PeDir::I386,
            digest: String::from("fnv1a64:0:0"),
            scope: String::from("legs=i386-windows only=all"),
            header: header(),
        };
        let mut opened = Resume::open(&self.state.resume(), &identity).unwrap();
        append(
            &mut opened.resume,
            Pass::First,
            "d3d12",
            &function("test_a"),
            FAIL,
        )
        .unwrap();
        opened.resume
    }

    fn baseline_text(&self) -> String {
        fs::read_to_string(self.baselines.join("d3d12.txt")).unwrap()
    }
}

impl Drop for Finish {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A `baseline-raw/<exe>` that is a symbolic link fails the end of a
/// record before anything is discarded: the old baseline and the resume
/// file stay, and no temporary baseline is left.
#[test]
fn a_failing_end_of_a_record_keeps_the_old_baseline_and_the_resume_file() {
    let finish = Finish::new("fails");
    let old = finish.baseline_text();
    let resume = finish.resume();
    let target = finish.dir.join("elsewhere");
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(finish.state.dir.join("baseline-raw")).unwrap();
    std::os::unix::fs::symlink(&target, finish.state.baseline_raw("d3d12")).unwrap();
    let run_dir = finish.state.runs().join("run");
    let result = finalize(
        &finish.state,
        &run_dir,
        std::slice::from_ref(&finish.pending),
        vec![resume],
    );
    let Err(Failure::Setup(message)) = result else {
        panic!("the end of the record did not fail");
    };
    assert!(message.contains("is a symbolic link"), "{message}");
    assert_eq!(finish.baseline_text(), old);
    assert!(!finish.baselines.join("d3d12.txt.new").exists());
    assert!(finish.resume_file.is_file());
}

/// Without a failure the new baseline is in place, the raw logs are
/// copied and the resume file is gone.
#[test]
fn the_end_of_a_record_writes_the_baseline_and_then_removes_the_resume_file() {
    let finish = Finish::new("writes");
    let resume = finish.resume();
    let run_dir = finish.state.runs().join("run");
    let log = run_dir.join("i386-windows/d3d12/d3d12/test_a.log");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, "a log\n").unwrap();
    finalize(
        &finish.state,
        &run_dir,
        std::slice::from_ref(&finish.pending),
        vec![resume],
    )
    .unwrap();
    assert_eq!(
        finish.baseline_text(),
        finish.pending.merge.baseline.to_text()
    );
    let copied = finish
        .state
        .baseline_raw("d3d12")
        .join("i386-windows/d3d12/test_a.log");
    assert_eq!(fs::read_to_string(copied).unwrap(), "a log\n");
    assert!(!finish.resume_file.exists());
}

/// The records of an interrupted leg split by pass: a second-pass record
/// is no first outcome, and a later record of a pass replaces an earlier.
#[test]
fn resume_records_split_by_pass() {
    let record = |pass, key: &str, outcome| Record {
        pass,
        unit: String::from("d3d12"),
        key: function(key),
        outcome,
    };
    let (first, confirm) = split_records(vec![
        record(Pass::First, "test_a", FAIL),
        record(Pass::Confirm, "test_a", OK),
        record(Pass::First, "test_b", OK),
        record(Pass::Confirm, "test_c", FAIL),
        record(Pass::First, "test_b", WORSE),
    ]);
    let slot = |key: &str| (String::from("d3d12"), function(key));
    assert_eq!(
        first,
        BTreeMap::from([(slot("test_a"), FAIL), (slot("test_b"), WORSE)])
    );
    assert_eq!(
        confirm,
        BTreeMap::from([(slot("test_a"), OK), (slot("test_c"), FAIL)])
    );
}

/// `record` decides from the same old baseline whether the second pass
/// came from this invocation or from the resume file of an interrupted
/// one, and a stray second outcome of a key it does not confirm changes
/// nothing.
#[test]
fn record_decides_the_same_with_resumed_passes() {
    let a = id(PeDir::I386, "test_a");
    let b = id(PeDir::I386, "test_b");
    let c = id(PeDir::I386, "test_c");
    let old = baseline(&[(&a, OK), (&b, OK)]);
    let first = map(&[(&a, FAIL), (&b, OK), (&c, OK)]);
    let fresh = decide_record(
        "d3d12",
        Some(&old),
        &header(),
        &first,
        &map(&[(&a, WORSE), (&c, OK)]),
        &Scope::All,
    )
    .unwrap();
    assert_eq!(fresh.unstable, [(a, WORSE)]);
    assert_eq!(fresh.baseline.entries[&c], OK);

    let records = vec![
        Record {
            pass: Pass::First,
            unit: String::from("d3d12"),
            key: function("test_a"),
            outcome: FAIL,
        },
        Record {
            pass: Pass::Confirm,
            unit: String::from("d3d12"),
            key: function("test_a"),
            outcome: WORSE,
        },
        Record {
            pass: Pass::Confirm,
            unit: String::from("d3d12"),
            key: function("test_b"),
            outcome: FAIL,
        },
    ];
    let (_, confirm) = split_records(records);
    let mut second: BTreeMap<TestId, Outcome> = confirm
        .into_iter()
        .map(|((unit, key), outcome)| {
            (
                TestId {
                    leg: PeDir::I386,
                    unit,
                    key,
                },
                outcome,
            )
        })
        .collect();
    second.insert(c, OK);
    let resumed =
        decide_record("d3d12", Some(&old), &header(), &first, &second, &Scope::All).unwrap();
    assert_eq!(resumed.baseline, fresh.baseline);
    assert_eq!(resumed.unstable, fresh.unstable);
    assert_eq!(resumed.baseline.entries[&b], OK);
    assert_eq!(Conclusion::Record { unstable: 1 }.exit_code(), 1);
}

#[test]
fn only_run_takes_debug_channels() {
    assert_eq!(check_winedebug(Mode::Run, None), Ok(()));
    assert_eq!(check_winedebug(Mode::Record, None), Ok(()));
    assert_eq!(check_winedebug(Mode::Run, Some("+win,+event")), Ok(()));
    assert_eq!(
        check_winedebug(Mode::Record, Some("+win,+event")),
        Err(String::from(
            "record takes no --winedebug: a baseline is the result of the untraced tests"
        ))
    );
    assert_eq!(
        check_winedebug(Mode::Run, Some("")),
        Err(String::from("--winedebug names no channel"))
    );
}

#[test]
fn a_traced_scope_differs_from_the_untraced_one() {
    let config = config();
    let plans = plan(&config, &suites(&["d3d8"]), None, None).unwrap();
    let scope = plans[0].scope_text();
    assert_eq!(traced_scope(scope.clone(), None), scope);
    assert_eq!(
        traced_scope(scope, Some("+win,+event")),
        "legs=i386-windows only=all winedebug=+win,+event"
    );
}
