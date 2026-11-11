use super::*;

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
