use super::*;

fn plans_of(groups: &Groups) -> Vec<Planned<'_>> {
    groups
        .groups
        .iter()
        .map(|group| Planned {
            group,
            cases: vec![String::from("dEQP-VK.a")],
            whole: true,
            old: None,
            path: PathBuf::from("/e/g.txt"),
        })
        .collect()
}

#[test]
fn each_kk_debug_setting_runs_once_unset_first() {
    let groups = Groups::parse(
        "[[group]]\nname = \"f\"\ncases = [\"dEQP-VK.*\"]\nkk_debug = \"force_icb\"\n\
         [[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\n\
         [[group]]\nname = \"h\"\ncases = [\"dEQP-VK.*\"]\nkk_debug = \"force_icb\"\n\
         [[group]]\nname = \"i\"\ncases = [\"dEQP-VK.*\"]\nkk_debug = \"msl\"\n",
    )
    .unwrap();
    let plans = plans_of(&groups);
    assert_eq!(settings(&plans), [None, Some("force_icb"), Some("msl")]);
    let groups = Groups::parse("[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\n").unwrap();
    assert_eq!(settings(&plans_of(&groups)), [None]);
}

#[test]
fn a_setting_whose_groups_have_no_case_left_does_not_run() {
    let groups = Groups::parse(
        "[[group]]\nname = \"f\"\ncases = [\"dEQP-VK.*\"]\nkk_debug = \"force_icb\"\n\
         [[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\n\
         [[group]]\nname = \"h\"\ncases = [\"dEQP-VK.*\"]\nkk_debug = \"force_icb\"\n",
    )
    .unwrap();
    // `--cases` left the two force_icb groups without a case.
    let mut plans = plans_of(&groups);
    plans[0].cases.clear();
    plans[2].cases.clear();
    assert_eq!(settings(&plans), [None]);
    // One force_icb group kept its cases, the group without a setting none.
    let mut plans = plans_of(&groups);
    plans[0].cases.clear();
    plans[1].cases.clear();
    assert_eq!(settings(&plans), [Some("force_icb")]);
}

#[test]
fn the_runs_of_the_settings_combine_to_the_worse() {
    let run = |a, b| match combine(Conclusion::Run(a), Conclusion::Run(b)) {
        Conclusion::Run(verdict) => verdict,
        other => panic!("not a run: {other:?}"),
    };
    assert_eq!(run(Verdict::Match, Verdict::Match), Verdict::Match);
    assert_eq!(run(Verdict::Match, Verdict::Unstable), Verdict::Unstable);
    assert_eq!(run(Verdict::Unstable, Verdict::Match), Verdict::Unstable);
    match combine(
        Conclusion::Record { unstable: 1 },
        Conclusion::Record { unstable: 2 },
    ) {
        Conclusion::Record { unstable } => assert_eq!(unstable, 3),
        other => panic!("not a record: {other:?}"),
    }
}

fn statuses(pairs: &[(&str, Status)]) -> BTreeMap<String, Status> {
    pairs
        .iter()
        .map(|(case, status)| ((*case).to_owned(), *status))
        .collect()
}

#[test]
fn crashes_no_case_explains_alone_are_unstable() {
    let suspects = names(&["dEQP-VK.a"]);
    let rerun = names(&["dEQP-VK.b", "dEQP-VK.c"]);
    let parallel = statuses(&[
        ("dEQP-VK.a", Status::Crash),
        ("dEQP-VK.b", Status::Fail),
        ("dEQP-VK.c", Status::Fail),
    ]);
    let last = statuses(&[
        ("dEQP-VK.a", Status::Pass),
        ("dEQP-VK.b", Status::Pass),
        ("dEQP-VK.c", Status::Fail),
    ]);
    let unstable = changed_without_cause(&suspects, &rerun, &parallel, &last);
    assert_eq!(
        unstable.into_iter().collect::<Vec<_>>(),
        [
            (String::from("dEQP-VK.a"), (Status::Crash, Status::Pass)),
            (String::from("dEQP-VK.b"), (Status::Fail, Status::Pass)),
        ]
    );
    let reproduced = statuses(&[
        ("dEQP-VK.a", Status::Crash),
        ("dEQP-VK.b", Status::Pass),
        ("dEQP-VK.c", Status::Fail),
    ]);
    assert_eq!(
        changed_without_cause(&suspects, &rerun, &parallel, &reproduced).len(),
        0
    );
    assert_eq!(
        changed_without_cause(&[], &rerun, &parallel, &last).len(),
        0
    );
}

fn names(cases: &[&str]) -> Vec<String> {
    cases.iter().map(|case| (*case).to_owned()).collect()
}

fn attempt(results: &[(&str, Status)], unfinished: Option<&str>, ending: Ending) -> Attempt {
    Attempt {
        results: results
            .iter()
            .map(|(case, status)| ((*case).to_owned(), *status))
            .collect(),
        unfinished: unfinished.map(str::to_owned),
        ending,
    }
}

#[test]
fn a_clean_process_settles_its_cases() {
    let cases = names(&["dEQP-VK.a", "dEQP-VK.b"]);
    let remaining: Vec<&String> = cases.iter().collect();
    let step = next_step(
        &remaining,
        &attempt(
            &[("dEQP-VK.a", Status::Pass), ("dEQP-VK.b", Status::Fail)],
            None,
            Ending::Exited(0),
        ),
    )
    .unwrap();
    assert_eq!(step.results.len(), 2);
    assert_eq!(step.remaining.len(), 0);
}

#[test]
fn a_crash_settles_the_unfinished_case_and_keeps_the_rest() {
    let cases = names(&["dEQP-VK.a", "dEQP-VK.b", "dEQP-VK.c", "dEQP-VK.d"]);
    let remaining: Vec<&String> = cases.iter().collect();
    let step = next_step(
        &remaining,
        &attempt(
            &[("dEQP-VK.a", Status::Pass), ("dEQP-VK.x", Status::Pass)],
            Some("dEQP-VK.b"),
            Ending::Signaled(11),
        ),
    )
    .unwrap();
    assert_eq!(
        step.results,
        [
            (String::from("dEQP-VK.a"), Status::Pass),
            (String::from("dEQP-VK.b"), Status::Crash),
        ]
    );
    assert_eq!(step.remaining, [&cases[2], &cases[3]]);
    let step = next_step(
        &step.remaining,
        &attempt(&[], Some("dEQP-VK.c"), Ending::Stalled),
    )
    .unwrap();
    assert_eq!(step.results, [(String::from("dEQP-VK.c"), Status::Stall)]);
    assert_eq!(step.remaining, [&cases[3]]);
}

#[test]
fn a_process_that_ran_nothing_is_an_error() {
    let cases = names(&["dEQP-VK.a"]);
    let remaining: Vec<&String> = cases.iter().collect();
    let e = next_step(&remaining, &attempt(&[], None, Ending::Exited(1)))
        .err()
        .unwrap();
    assert!(e.contains("exit code 1"), "{e}");
    let e = next_step(
        &remaining,
        &attempt(&[], Some("dEQP-VK.other"), Ending::Signaled(6)),
    )
    .err()
    .unwrap();
    assert!(e.contains("without running any of its 1 cases"), "{e}");
}

#[test]
fn a_process_that_stopped_between_cases_leaves_the_rest_for_the_next() {
    let cases = names(&["dEQP-VK.a", "dEQP-VK.b"]);
    let remaining: Vec<&String> = cases.iter().collect();
    let step = next_step(
        &remaining,
        &attempt(&[("dEQP-VK.a", Status::Pass)], None, Ending::Signaled(6)),
    )
    .unwrap();
    assert_eq!(step.results.len(), 1);
    assert_eq!(step.remaining, [&cases[1]]);
}

/// Groups `f` and `g`, both over every case.
fn two_groups() -> Groups {
    Groups::parse(
        "[[group]]\nname = \"f\"\ncases = [\"dEQP-VK.*\"]\n\
         [[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\n",
    )
    .unwrap()
}

/// A plan of `group` over `cases`, with the expected results of `old`, the
/// lines after the header, or none.
fn planned<'g>(group: &'g Group, cases: &[&str], old: Option<&str>) -> Planned<'g> {
    Planned {
        group,
        cases: names(cases),
        whole: true,
        old: old.map(|lines| {
            let text = format!("# cases: {}\n\n{lines}", cases.len());
            Expected::parse(&text, &group.name, "g.txt").unwrap()
        }),
        path: PathBuf::from("/e/g.txt"),
    }
}

fn results(pairs: &[(&str, Status)]) -> Vec<(String, Status)> {
    pairs
        .iter()
        .map(|(case, status)| ((*case).to_owned(), *status))
        .collect()
}

#[test]
fn each_case_is_expected_to_end_as_all_its_groups_say() {
    let groups = two_groups();
    let f = planned(
        &groups.groups[0],
        &["dEQP-VK.a", "dEQP-VK.b", "dEQP-VK.c"],
        Some("dEQP-VK.b fail\ndEQP-VK.c not-supported\n"),
    );
    let g = planned(
        &groups.groups[1],
        &["dEQP-VK.b", "dEQP-VK.c", "dEQP-VK.d"],
        None,
    );
    let expected = expectations(&[&f, &g]);
    assert_eq!(expected.get("dEQP-VK.a"), Some(&Some(Status::Pass)));
    assert_eq!(expected.get("dEQP-VK.d"), Some(&Some(Status::Pass)));
    // `g` has no expected results, so it expects b and c to pass.
    assert_eq!(expected.get("dEQP-VK.b"), Some(&None));
    assert_eq!(expected.get("dEQP-VK.c"), Some(&None));
    let expected = expectations(&[&f]);
    assert_eq!(expected.get("dEQP-VK.b"), Some(&Some(Status::Fail)));
    assert_eq!(expected.get("dEQP-VK.c"), Some(&Some(Status::NotSupported)));
}

fn expected_of(pairs: &[(&'static str, Option<Status>)]) -> HashMap<&'static str, Option<Status>> {
    pairs.iter().copied().collect()
}

#[test]
fn a_first_pass_process_whose_cases_pass_as_expected_keeps_no_log() {
    let expected = expected_of(&[
        ("dEQP-VK.a", Some(Status::Pass)),
        ("dEQP-VK.b", Some(Status::NotSupported)),
    ]);
    let ran = results(&[
        ("dEQP-VK.a", Status::Pass),
        ("dEQP-VK.b", Status::NotSupported),
    ]);
    assert!(!keeps_log(false, &ran, &expected));
    assert!(!keeps_log(false, &[], &expected));
}

#[test]
fn a_first_pass_process_keeps_its_log_for_a_case_that_did_not_pass() {
    let expected = expected_of(&[
        ("dEQP-VK.a", Some(Status::Pass)),
        ("dEQP-VK.b", Some(Status::Fail)),
        ("dEQP-VK.c", Some(Status::Crash)),
    ]);
    for status in [Status::Warn, Status::Fail, Status::Crash, Status::Stall] {
        let ran = results(&[("dEQP-VK.a", Status::Pass), ("dEQP-VK.b", status)]);
        assert!(keeps_log(false, &ran, &expected), "{status}");
    }
    // Expected or not, a failing or crashing case keeps the log.
    assert!(keeps_log(
        false,
        &results(&[("dEQP-VK.b", Status::Fail)]),
        &expected
    ));
    assert!(keeps_log(
        false,
        &results(&[("dEQP-VK.c", Status::Crash)]),
        &expected
    ));
}

#[test]
fn a_first_pass_process_keeps_its_log_for_a_status_its_groups_do_not_expect() {
    let expected = expected_of(&[
        ("dEQP-VK.a", Some(Status::Pass)),
        ("dEQP-VK.b", Some(Status::NotSupported)),
        ("dEQP-VK.c", None),
    ]);
    for ran in [
        [("dEQP-VK.a", Status::NotSupported)],
        [("dEQP-VK.b", Status::Pass)],
        [("dEQP-VK.c", Status::Pass)],
        [("dEQP-VK.unknown", Status::Pass)],
    ] {
        assert!(keeps_log(false, &results(&ran), &expected), "{ran:?}");
    }
}

#[test]
fn a_process_of_a_later_pass_keeps_its_log() {
    let expected = expected_of(&[("dEQP-VK.a", Some(Status::Pass))]);
    assert!(keeps_log(
        true,
        &results(&[("dEQP-VK.a", Status::Pass)]),
        &expected
    ));
}

#[test]
fn a_canary_keeps_its_log_only_when_it_does_not_pass() {
    assert!(canary_passed(&results(&[(CANARY, Status::Pass)])));
    for ran in [
        results(&[(CANARY, Status::Fail)]),
        results(&[(CANARY, Status::Crash)]),
        results(&[("dEQP-VK.a", Status::Pass)]),
        results(&[]),
    ] {
        assert!(!canary_passed(&ran), "{ran:?}");
    }
}

#[test]
fn removing_a_test_log_the_process_did_not_write_succeeds() {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("conformance-test-deqp-logs");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let qpa = dir.join("batch-0.1.qpa");
    fs::write(&qpa, "#beginSession\n").unwrap();
    remove_log(&qpa).unwrap();
    assert!(!qpa.exists());
    remove_log(&qpa).unwrap();
    fs::remove_dir_all(&dir).unwrap();
}
