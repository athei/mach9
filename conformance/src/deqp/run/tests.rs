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
        Conclusion::Record { .. } => panic!("a record"),
    };
    assert_eq!(run(Verdict::Match, Verdict::Match), Verdict::Match);
    assert_eq!(run(Verdict::Match, Verdict::Unstable), Verdict::Unstable);
    assert_eq!(run(Verdict::Unstable, Verdict::Match), Verdict::Unstable);
    match combine(
        Conclusion::Record { unstable: 1 },
        Conclusion::Record { unstable: 2 },
    ) {
        Conclusion::Record { unstable } => assert_eq!(unstable, 3),
        Conclusion::Run(_) => panic!("a run"),
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
