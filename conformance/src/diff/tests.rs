use Status::{Crash, Fail, NotReached, Stall};

use super::*;
use crate::outcome::Key;

const fn outcome(
    status: Status,
    failed: u32,
    sites: u32,
    todo_succeeded: u32,
    skipped: u32,
) -> Outcome {
    Outcome {
        status,
        failed,
        sites,
        todo_succeeded,
        skipped,
    }
}

const OK: Outcome = outcome(Status::Ok, 0, 0, 0, 1);
const FAIL: Outcome = outcome(Fail, 3, 2, 0, 1);

fn id(leg: PeDir, key: &str) -> TestId {
    TestId {
        leg,
        unit: String::from("d3d11"),
        key: Key::Function(key.to_owned()),
    }
}

fn header(wine: &str) -> Header {
    Header::parse(wine, "dxmt=8c0b5f1\nfex=0123456\n").unwrap()
}

fn baseline(entries: &[(TestId, Outcome)]) -> Baseline {
    Baseline {
        exe: String::from("d3d11"),
        header: header("wine-11.21"),
        entries: entries.iter().cloned().collect(),
    }
}

fn no_triage() -> Triage {
    Triage::parse("", "triage.txt").unwrap()
}

fn flaky_triage() -> Triage {
    Triage::parse("d3d11/d3d11/test_a flaky\n", "triage.txt").unwrap()
}

/// One row of a rule table: the key's baseline outcome, its run's outcome,
/// whether the triage marks it flaky, the deltas and the verdict.
struct Case {
    before: Option<Outcome>,
    after: Option<Outcome>,
    flaky: bool,
    deltas: Vec<Delta>,
    verdict: Verdict,
}

fn case(
    before: Option<Outcome>,
    after: Option<Outcome>,
    deltas: Vec<Delta>,
    verdict: Verdict,
) -> Case {
    Case {
        before,
        after,
        flaky: false,
        deltas,
        verdict,
    }
}

/// Diff each case's one key and check its deltas and verdict.
fn check(cases: Vec<Case>) {
    let key = id(PeDir::I386, "test_a");
    for case in cases {
        let recorded: Vec<(TestId, Outcome)> =
            case.before.map(|o| (key.clone(), o)).into_iter().collect();
        let results: BTreeMap<TestId, Outcome> =
            case.after.map(|o| (key.clone(), o)).into_iter().collect();
        let triage = if case.flaky {
            flaky_triage()
        } else {
            no_triage()
        };
        let report = diff(
            "d3d11",
            Some(&baseline(&recorded)),
            &header("wine-11.21"),
            &results,
            &Scope::All,
            &triage,
        );
        let label = format!(
            "{:?} -> {:?}, flaky {}",
            case.before, case.after, case.flaky
        );
        assert_eq!(report.verdict(), case.verdict, "{label}");
        match report.changes.as_slice() {
            [] => {
                assert_eq!(case.deltas, [], "{label}");
                assert_eq!(report.matched, 1, "{label}");
            }
            [change] => {
                assert_eq!(change.deltas, case.deltas, "{label}");
                assert_eq!(change.flaky, case.flaky, "{label}");
                assert_eq!(report.matched, 0, "{label}");
            }
            _ => panic!("{label}: more than one change"),
        }
    }
}

#[test]
fn equal_outcomes_match() {
    check(vec![
        case(Some(OK), Some(OK), vec![], Verdict::Match),
        case(Some(FAIL), Some(FAIL), vec![], Verdict::Match),
    ]);
}

/// A worse status is a regression, a better one stale; a crash's counts are
/// not compared with those of a full run.
#[test]
fn status_changes() {
    check(vec![
        case(
            Some(OK),
            Some(outcome(Fail, 1, 1, 0, 1)),
            vec![
                Delta::Status {
                    from: Status::Ok,
                    to: Fail,
                },
                Delta::Failed { from: 0, to: 1 },
                Delta::Sites { from: 0, to: 1 },
            ],
            Verdict::Regression,
        ),
        case(
            Some(FAIL),
            Some(outcome(NotReached, 0, 0, 0, 1)),
            vec![
                Delta::Status {
                    from: Fail,
                    to: NotReached,
                },
                Delta::Failed { from: 3, to: 0 },
                Delta::Sites { from: 2, to: 0 },
            ],
            Verdict::Regression,
        ),
        case(
            Some(FAIL),
            Some(outcome(Crash, 1, 1, 0, 0)),
            vec![Delta::Status {
                from: Fail,
                to: Crash,
            }],
            Verdict::Regression,
        ),
        case(
            Some(OK),
            Some(outcome(Stall, 0, 0, 0, 0)),
            vec![Delta::Status {
                from: Status::Ok,
                to: Stall,
            }],
            Verdict::Regression,
        ),
        case(
            Some(outcome(Crash, 1, 1, 0, 0)),
            Some(outcome(Stall, 1, 1, 0, 0)),
            vec![Delta::Status {
                from: Crash,
                to: Stall,
            }],
            Verdict::Regression,
        ),
        case(
            Some(outcome(Crash, 1, 1, 0, 0)),
            Some(FAIL),
            vec![Delta::Status {
                from: Crash,
                to: Fail,
            }],
            Verdict::Stale,
        ),
        case(
            Some(FAIL),
            Some(OK),
            vec![
                Delta::Status {
                    from: Fail,
                    to: Status::Ok,
                },
                Delta::Failed { from: 3, to: 0 },
                Delta::Sites { from: 2, to: 0 },
            ],
            Verdict::Stale,
        ),
        case(
            Some(outcome(NotReached, 0, 0, 0, 1)),
            Some(OK),
            vec![Delta::Status {
                from: NotReached,
                to: Status::Ok,
            }],
            Verdict::Stale,
        ),
    ]);
}

#[test]
fn failure_and_site_changes() {
    check(vec![
        case(
            Some(FAIL),
            Some(outcome(Fail, 4, 2, 0, 1)),
            vec![Delta::Failed { from: 3, to: 4 }],
            Verdict::Regression,
        ),
        case(
            Some(FAIL),
            Some(outcome(Fail, 3, 3, 0, 1)),
            vec![Delta::Sites { from: 2, to: 3 }],
            Verdict::Regression,
        ),
        case(
            Some(FAIL),
            Some(outcome(Fail, 2, 2, 0, 1)),
            vec![Delta::Failed { from: 3, to: 2 }],
            Verdict::Stale,
        ),
        case(
            Some(FAIL),
            Some(outcome(Fail, 2, 3, 0, 1)),
            vec![
                Delta::Failed { from: 3, to: 2 },
                Delta::Sites { from: 2, to: 3 },
            ],
            Verdict::Regression,
        ),
    ]);
}

/// The counts of two truncated outcomes are listed and never gate, in
/// either direction; a swap of crash and stall still does.
#[test]
fn counts_of_two_truncated_outcomes_gate_only_when_more_failed() {
    check(vec![
        case(
            Some(outcome(Crash, 1, 1, 0, 0)),
            Some(outcome(Crash, 2, 2, 0, 0)),
            vec![
                Delta::Failed { from: 1, to: 2 },
                Delta::Sites { from: 1, to: 2 },
            ],
            Verdict::Regression,
        ),
        case(
            Some(outcome(Crash, 0, 0, 0, 0)),
            Some(outcome(Crash, 50, 1, 0, 0)),
            vec![
                Delta::Failed { from: 0, to: 50 },
                Delta::Sites { from: 0, to: 1 },
            ],
            Verdict::Regression,
        ),
        case(
            Some(outcome(Stall, 4, 2, 0, 0)),
            Some(outcome(Stall, 3, 3, 0, 0)),
            vec![
                Delta::Failed { from: 4, to: 3 },
                Delta::Sites { from: 2, to: 3 },
            ],
            Verdict::Regression,
        ),
        case(
            Some(outcome(Crash, 0, 0, 0, 0)),
            Some(outcome(Crash, 0, 0, 0, 3)),
            vec![Delta::Skipped { from: 0, to: 3 }],
            Verdict::Match,
        ),
        case(
            Some(outcome(Crash, 3, 2, 0, 2)),
            Some(outcome(Crash, 0, 0, 0, 0)),
            vec![
                Delta::Failed { from: 3, to: 0 },
                Delta::Sites { from: 2, to: 0 },
                Delta::Skipped { from: 2, to: 0 },
            ],
            Verdict::Match,
        ),
        case(
            Some(outcome(Stall, 0, 0, 1, 0)),
            Some(outcome(Stall, 0, 0, 0, 0)),
            vec![Delta::TodoSucceeded { from: 1, to: 0 }],
            Verdict::Match,
        ),
        case(
            Some(outcome(Crash, 3, 2, 0, 0)),
            Some(outcome(Stall, 0, 0, 0, 0)),
            vec![
                Delta::Status {
                    from: Crash,
                    to: Stall,
                },
                Delta::Failed { from: 3, to: 0 },
                Delta::Sites { from: 2, to: 0 },
            ],
            Verdict::Regression,
        ),
    ]);
    let key = id(PeDir::I386, "test_a");
    let report = diff(
        "d3d11",
        Some(&baseline(&[(key.clone(), outcome(Crash, 3, 2, 0, 0))])),
        &header("wine-11.21"),
        &BTreeMap::from([(key, outcome(Crash, 0, 0, 0, 0))]),
        &Scope::All,
        &no_triage(),
    );
    let text = report.text();
    assert!(
        text.starts_with("info d3d11 i386-windows d3d11/test_a: failed 3 -> 0, sites 2 -> 0\n"),
        "{text}"
    );
    assert!(text.ends_with("0 flaky, 1 info, 0 match\n"), "{text}");
}

/// More skips is lost coverage, even when a fail turned ok; fewer is
/// stale. A todo that starts or stops succeeding is stale.
#[test]
fn skip_and_todo_changes() {
    check(vec![
        case(
            Some(OK),
            Some(outcome(Status::Ok, 0, 0, 0, 2)),
            vec![Delta::Skipped { from: 1, to: 2 }],
            Verdict::Regression,
        ),
        case(
            Some(OK),
            Some(outcome(Status::Ok, 0, 0, 0, 0)),
            vec![Delta::Skipped { from: 1, to: 0 }],
            Verdict::Stale,
        ),
        case(
            Some(FAIL),
            Some(outcome(Status::Ok, 0, 0, 0, 3)),
            vec![
                Delta::Status {
                    from: Fail,
                    to: Status::Ok,
                },
                Delta::Failed { from: 3, to: 0 },
                Delta::Sites { from: 2, to: 0 },
                Delta::Skipped { from: 1, to: 3 },
            ],
            Verdict::Regression,
        ),
        case(
            Some(OK),
            Some(outcome(Status::Ok, 0, 0, 1, 1)),
            vec![Delta::TodoSucceeded { from: 0, to: 1 }],
            Verdict::Stale,
        ),
        case(
            Some(outcome(Status::Ok, 0, 0, 2, 1)),
            Some(OK),
            vec![Delta::TodoSucceeded { from: 2, to: 0 }],
            Verdict::Stale,
        ),
    ]);
}

#[test]
fn dropped_and_new_keys() {
    check(vec![
        case(Some(OK), None, vec![Delta::Dropped], Verdict::Regression),
        case(None, Some(OK), vec![Delta::New(Status::Ok)], Verdict::Stale),
        case(None, Some(FAIL), vec![Delta::New(Fail)], Verdict::Stale),
        case(
            None,
            Some(outcome(NotReached, 0, 0, 0, 0)),
            vec![Delta::New(NotReached)],
            Verdict::Stale,
        ),
        case(
            None,
            Some(outcome(Crash, 0, 0, 0, 0)),
            vec![Delta::New(Crash)],
            Verdict::Regression,
        ),
        case(
            None,
            Some(outcome(Stall, 0, 0, 0, 0)),
            vec![Delta::New(Stall)],
            Verdict::Regression,
        ),
    ]);
}

#[test]
fn a_flaky_key_gates_only_on_a_new_crash_or_stall() {
    let flaky = |before, after, deltas, verdict| Case {
        flaky: true,
        ..case(before, after, deltas, verdict)
    };
    check(vec![
        flaky(
            Some(OK),
            Some(outcome(Fail, 1, 1, 0, 1)),
            vec![
                Delta::Status {
                    from: Status::Ok,
                    to: Fail,
                },
                Delta::Failed { from: 0, to: 1 },
                Delta::Sites { from: 0, to: 1 },
            ],
            Verdict::Match,
        ),
        flaky(
            Some(FAIL),
            Some(OK),
            vec![
                Delta::Status {
                    from: Fail,
                    to: Status::Ok,
                },
                Delta::Failed { from: 3, to: 0 },
                Delta::Sites { from: 2, to: 0 },
            ],
            Verdict::Match,
        ),
        flaky(Some(OK), None, vec![Delta::Dropped], Verdict::Match),
        flaky(None, Some(OK), vec![Delta::New(Status::Ok)], Verdict::Match),
        flaky(
            None,
            Some(outcome(Crash, 0, 0, 0, 0)),
            vec![Delta::New(Crash)],
            Verdict::Regression,
        ),
        flaky(
            Some(OK),
            Some(outcome(Crash, 0, 0, 0, 0)),
            vec![Delta::Status {
                from: Status::Ok,
                to: Crash,
            }],
            Verdict::Regression,
        ),
        flaky(
            Some(FAIL),
            Some(outcome(Stall, 0, 0, 0, 0)),
            vec![Delta::Status {
                from: Fail,
                to: Stall,
            }],
            Verdict::Regression,
        ),
        flaky(
            Some(outcome(Crash, 0, 0, 0, 0)),
            Some(outcome(Crash, 1, 1, 0, 0)),
            vec![
                Delta::Failed { from: 0, to: 1 },
                Delta::Sites { from: 0, to: 1 },
            ],
            Verdict::Match,
        ),
    ]);
}

#[test]
fn a_smaller_scope_drops_nothing_outside_it() {
    let a32 = id(PeDir::I386, "test_a");
    let b32 = id(PeDir::I386, "test_b");
    let a64 = id(PeDir::X86_64, "test_a");
    let recorded = baseline(&[(a32.clone(), OK), (b32.clone(), OK), (a64.clone(), OK)]);
    let run = |scope: &Scope, results: &[(TestId, Outcome)]| {
        diff(
            "d3d11",
            Some(&recorded),
            &header("wine-11.21"),
            &results.iter().cloned().collect(),
            scope,
            &no_triage(),
        )
    };

    let report = run(&Scope::Leg(PeDir::X86_64), &[(a64.clone(), OK)]);
    assert_eq!(report.verdict(), Verdict::Match);
    assert_eq!(report.matched, 1);

    let only = Scope::Tests(BTreeSet::from([a32.clone()]));
    let report = run(&only, &[(a32.clone(), FAIL)]);
    assert_eq!(report.verdict(), Verdict::Regression);
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].id, a32);

    let report = run(&only, &[(b32.clone(), FAIL)]);
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].deltas, [Delta::Dropped]);

    let report = run(&Scope::All, &[(a32, OK), (b32, OK)]);
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].id, a64);
    assert_eq!(report.changes[0].deltas, [Delta::Dropped]);
}

#[test]
fn header_changes_are_listed_and_do_not_gate() {
    let a = id(PeDir::I386, "test_a");
    let recorded = baseline(&[(a.clone(), OK)]);
    let mut run_header = header("wine-11.22");
    run_header
        .components
        .insert(String::from("dxmt"), String::from("99aa88b"));
    let report = diff(
        "d3d11",
        Some(&recorded),
        &run_header,
        &BTreeMap::from([(a, OK)]),
        &Scope::All,
        &no_triage(),
    );
    assert_eq!(report.verdict(), Verdict::Match);
    let text = report.text();
    assert!(
        text.contains("d3d11: header: Wine wine-11.21 -> wine-11.22\n"),
        "{text}"
    );
    assert!(
        text.contains("d3d11: header: dxmt 8c0b5f1 -> 99aa88b\n"),
        "{text}"
    );
}

#[test]
fn without_a_baseline_every_key_is_new() {
    let report = diff(
        "d3d11",
        None,
        &header("wine-11.21"),
        &BTreeMap::from([(id(PeDir::I386, "test_a"), OK)]),
        &Scope::All,
        &no_triage(),
    );
    assert_eq!(report.verdict(), Verdict::Stale);
    assert_eq!(report.header, []);
}

#[test]
fn the_report_lists_regressions_then_stale_then_flaky_keys() {
    let ids: Vec<TestId> = ["test_a", "test_b", "test_c", "test_d"]
        .iter()
        .map(|key| id(PeDir::I386, key))
        .collect();
    let recorded = baseline(&[
        (ids[0].clone(), FAIL),
        (ids[1].clone(), OK),
        (ids[2].clone(), FAIL),
        (ids[3].clone(), OK),
    ]);
    let results = BTreeMap::from([
        (ids[0].clone(), outcome(Fail, 5, 2, 0, 1)),
        (ids[1].clone(), OK),
        (ids[2].clone(), OK),
        (ids[3].clone(), outcome(Crash, 0, 0, 0, 0)),
    ]);
    let report = diff(
        "d3d11",
        Some(&recorded),
        &header("wine-11.21"),
        &results,
        &Scope::All,
        &flaky_triage(),
    );
    assert_eq!(
        report.text(),
        "\
REGRESSION d3d11 i386-windows d3d11/test_d: status ok -> crash
  baseline ok failed=0 sites=0 todo_succeeded=0 skipped=1
  run      crash failed=0 sites=0 todo_succeeded=0 skipped=0
stale d3d11 i386-windows d3d11/test_c: status fail -> ok, failed 3 -> 0, sites 2 -> 0
  baseline fail failed=3 sites=2 todo_succeeded=0 skipped=1
  run      ok failed=0 sites=0 todo_succeeded=0 skipped=1
flaky d3d11 i386-windows d3d11/test_a: failed 3 -> 5
  baseline fail failed=3 sites=2 todo_succeeded=0 skipped=1
  run      fail failed=5 sites=2 todo_succeeded=0 skipped=1
d3d11: 1 regressions, 1 stale, 1 flaky, 0 info, 1 match
"
    );
    assert_eq!(report.verdict(), Verdict::Regression);
}

#[test]
fn verdicts_order_from_match_to_regression() {
    assert!(Verdict::Match < Verdict::Stale);
    assert!(Verdict::Stale < Verdict::Unstable);
    assert!(Verdict::Unstable < Verdict::Regression);
    assert_eq!(Verdict::Stale.max(Verdict::Unstable), Verdict::Unstable);
}
