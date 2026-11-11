use super::*;

fn components() -> BTreeMap<String, String> {
    [("mesa", "aaaaaaa1"), ("vk-gl-cts", "bbbbbbb2")]
        .into_iter()
        .map(|(name, commit)| (name.to_owned(), commit.to_owned()))
        .collect()
}

fn results(pairs: &[(&str, Status)]) -> BTreeMap<String, Status> {
    pairs
        .iter()
        .map(|(case, status)| ((*case).to_owned(), *status))
        .collect()
}

fn expected(pairs: &[(&str, Status)], cases: usize) -> Expected {
    Expected {
        group: String::from("g"),
        components: components(),
        cases,
        results: results(pairs),
    }
}

#[test]
fn the_text_reads_back() {
    let one = expected(
        &[
            ("dEQP-VK.a.fails", Status::Fail),
            ("dEQP-VK.b.unsupported", Status::NotSupported),
        ],
        7,
    );
    let text = one.to_text();
    assert!(text.contains("# component: mesa=aaaaaaa1\n"), "{text}");
    assert!(
        text.contains("# cases: 7\n\ndEQP-VK.a.fails fail\n"),
        "{text}"
    );
    assert_eq!(Expected::parse(&text, "g", "g.txt").unwrap(), one);
    let empty = expected(&[], 3);
    assert_eq!(
        Expected::parse(&empty.to_text(), "g", "g.txt").unwrap(),
        empty
    );
}

#[test]
fn bad_files_are_refused() {
    for (text, error) in [
        ("dEQP-VK.a fail\n", "no `# cases: <n>` line"),
        ("# cases: 1\n# cases: 2\n", "a second `# cases:` line"),
        ("# cases: x\n", "not a number of cases"),
        (
            "# cases: 1\ndEQP-VK.a pass\n",
            "not a status of a case that does not pass",
        ),
        ("# cases: 1\ndEQP-VK.a broken\n", "not a status"),
        ("# cases: 1\nVK.a fail\n", "not a deqp-vk case"),
        (
            "# cases: 2\ndEQP-VK.b fail\ndEQP-VK.a fail\n",
            "out of order",
        ),
        (
            "# cases: 2\ndEQP-VK.a fail\ndEQP-VK.a crash\n",
            "listed twice",
        ),
        (
            "# cases: 1\ndEQP-VK.a fail\n# late\n",
            "a comment after the first case",
        ),
        ("# component: mesa=1 2\n# cases: 1\n", "g.txt:1:"),
        ("# cases: 1\ndEQP-VK.a\n", "is not `<case> <status>`"),
    ] {
        let e = Expected::parse(text, "g", "g.txt").err().unwrap();
        assert!(e.contains(error), "{text:?}: {e}");
    }
}

#[test]
fn statuses_rank_and_classify_their_changes() {
    for status in Status::ALL {
        assert_eq!(Status::parse(status.as_str()), Some(status));
        assert_eq!(status.change_to(status), None);
    }
    assert_eq!(
        Status::Pass.change_to(Status::Fail),
        Some(Class::Regression)
    );
    assert_eq!(Status::Fail.change_to(Status::Pass), Some(Class::Stale));
    assert_eq!(
        Status::Pass.change_to(Status::NotSupported),
        Some(Class::Regression)
    );
    assert_eq!(
        Status::Fail.change_to(Status::NotSupported),
        Some(Class::Regression)
    );
    assert_eq!(
        Status::Crash.change_to(Status::Stall),
        Some(Class::Regression)
    );
    assert_eq!(
        Status::Stall.change_to(Status::Crash),
        Some(Class::Regression)
    );
    assert_eq!(Status::Crash.change_to(Status::Fail), Some(Class::Stale));
    assert_eq!(Status::from_deqp("Timeout"), Some(Status::Stall));
    assert_eq!(Status::from_deqp("Waiver"), Some(Status::NotSupported));
    assert_eq!(Status::from_deqp("InternalError"), Some(Status::Fail));
    assert_eq!(Status::from_deqp("Running"), None);
}

#[test]
fn a_run_is_diffed_case_by_case() {
    let old = expected(
        &[
            ("dEQP-VK.a.fixed", Status::Fail),
            ("dEQP-VK.a.still", Status::Fail),
            ("dEQP-VK.a.gone", Status::Fail),
            ("dEQP-VK.a.flaky", Status::Fail),
        ],
        6,
    );
    let selected = [
        "dEQP-VK.a.fixed",
        "dEQP-VK.a.still",
        "dEQP-VK.a.broke",
        "dEQP-VK.a.flaky",
        "dEQP-VK.a.flaky_crash",
        "dEQP-VK.a.ok",
    ];
    let run = results(&[
        ("dEQP-VK.a.fixed", Status::Pass),
        ("dEQP-VK.a.still", Status::Fail),
        ("dEQP-VK.a.broke", Status::Fail),
        ("dEQP-VK.a.flaky", Status::Pass),
        ("dEQP-VK.a.flaky_crash", Status::Crash),
        ("dEQP-VK.a.ok", Status::Pass),
    ]);
    let triage = Triage::parse("dEQP-VK.a.flaky* flaky\n", "triage.txt").unwrap();
    let mut changed = components();
    changed.insert(String::from("mesa"), String::from("ccccccc3"));
    let report = diff(&old, &selected, true, &run, &changed, &triage);
    let lines: Vec<String> = report
        .changes
        .iter()
        .map(|change| format!("{} {:?}", change.case, change.class()))
        .collect();
    assert_eq!(
        lines,
        [
            "dEQP-VK.a.broke Some(Regression)",
            "dEQP-VK.a.flaky_crash Some(Regression)",
            "dEQP-VK.a.fixed Some(Stale)",
            "dEQP-VK.a.gone Some(Stale)",
            "dEQP-VK.a.flaky None",
        ]
    );
    assert_eq!(report.matched, 2);
    assert_eq!(report.verdict(), Verdict::Regression);
    let text = report.text();
    assert!(
        text.contains("deqp-vk g: header: mesa aaaaaaa1 -> ccccccc3\n"),
        "{text}"
    );
    assert!(
        text.contains("REGRESSION deqp-vk g dEQP-VK.a.broke: pass -> fail\n"),
        "{text}"
    );
    assert!(
        text.contains("stale deqp-vk g dEQP-VK.a.gone: fail -> not selected\n"),
        "{text}"
    );
    assert!(
        text.contains("deqp-vk g: 2 regressions, 2 stale, 1 flaky, 2 match\n"),
        "{text}"
    );
}

#[test]
fn a_partial_run_drops_no_listed_case_and_a_missing_result_is_a_crash() {
    let old = expected(&[("dEQP-VK.a.other", Status::Fail)], 2);
    let report = diff(
        &old,
        &["dEQP-VK.a.one"],
        false,
        &BTreeMap::new(),
        &components(),
        &Triage::default(),
    );
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].run, Some(Status::Crash));
    assert_eq!(report.header.len(), 0);
}

#[test]
fn another_number_of_cases_under_a_node_line_is_stale_in_a_whole_run() {
    let old = expected(&[("dEQP-VK.a.*", Status::NotSupported)], 3);
    let run = results(&[
        ("dEQP-VK.a.1", Status::NotSupported),
        ("dEQP-VK.a.2", Status::NotSupported),
    ]);
    let selected = ["dEQP-VK.a.1", "dEQP-VK.a.2"];
    let report = diff(
        &old,
        &selected,
        true,
        &run,
        &components(),
        &Triage::default(),
    );
    assert_eq!(report.changes.len(), 0);
    assert_eq!(report.count, Some((3, 2)));
    assert_eq!(report.verdict(), Verdict::Stale);
    let text = report.text();
    assert!(
        text.contains(
            "stale deqp-vk g: the group selects 2 cases, its expected results were recorded for 3"
        ),
        "{text}"
    );
    let partial = diff(
        &old,
        &selected,
        false,
        &run,
        &components(),
        &Triage::default(),
    );
    assert_eq!(partial.count, None);
    assert_eq!(partial.verdict(), Verdict::Match);
}

#[test]
fn a_match_has_the_match_verdict() {
    let old = expected(&[("dEQP-VK.a.f", Status::Fail)], 2);
    let run = results(&[("dEQP-VK.a.f", Status::Fail), ("dEQP-VK.a.p", Status::Pass)]);
    let report = diff(
        &old,
        &["dEQP-VK.a.f", "dEQP-VK.a.p"],
        true,
        &run,
        &components(),
        &Triage::default(),
    );
    assert_eq!(report.verdict(), Verdict::Match);
    assert_eq!(report.matched, 2);
}

#[test]
fn record_writes_the_cases_that_do_not_pass_with_the_worse_of_two_runs() {
    let selected = ["dEQP-VK.a.1", "dEQP-VK.a.2", "dEQP-VK.a.3", "dEQP-VK.a.4"];
    let first = results(&[
        ("dEQP-VK.a.1", Status::Pass),
        ("dEQP-VK.a.2", Status::Fail),
        ("dEQP-VK.a.3", Status::Fail),
        ("dEQP-VK.a.4", Status::Pass),
    ]);
    let second = results(&[
        ("dEQP-VK.a.2", Status::Fail),
        ("dEQP-VK.a.4", Status::Crash),
    ]);
    let (new, unstable) = record("g", &selected, &first, &second, &components());
    assert_eq!(new.cases, 4);
    assert_eq!(
        new.results,
        results(&[
            ("dEQP-VK.a.2", Status::Fail),
            ("dEQP-VK.a.3", Status::Fail),
            ("dEQP-VK.a.4", Status::Crash),
        ])
    );
    assert_eq!(
        unstable,
        [(String::from("dEQP-VK.a.4"), Status::Pass, Status::Crash)]
    );
}

#[test]
fn record_writes_one_line_for_a_node_whose_cases_share_a_status() {
    let selected = [
        "dEQP-VK.a.b.1",
        "dEQP-VK.a.b.2",
        "dEQP-VK.a.c.1",
        "dEQP-VK.a.c.2",
        "dEQP-VK.a.d.only",
        "dEQP-VK.a.e",
        "dEQP-VK.f.1",
        "dEQP-VK.f.2",
    ];
    let first = results(&[
        ("dEQP-VK.a.b.1", Status::NotSupported),
        ("dEQP-VK.a.b.2", Status::NotSupported),
        ("dEQP-VK.a.c.1", Status::Pass),
        ("dEQP-VK.a.c.2", Status::Fail),
        ("dEQP-VK.a.d.only", Status::NotSupported),
        ("dEQP-VK.a.e", Status::Pass),
        ("dEQP-VK.f.1", Status::Pass),
        ("dEQP-VK.f.2", Status::Pass),
    ]);
    let (new, unstable) = record("g", &selected, &first, &BTreeMap::new(), &components());
    assert_eq!(unstable.len(), 0);
    assert_eq!(
        new.results,
        results(&[
            ("dEQP-VK.a.b.*", Status::NotSupported),
            ("dEQP-VK.a.c.2", Status::Fail),
            ("dEQP-VK.a.d.only", Status::NotSupported),
        ])
    );
    for &case in &selected {
        assert_eq!(new.status(case), first[case], "{case}");
    }
    let text = new.to_text();
    assert_eq!(Expected::parse(&text, "g", "g.txt").unwrap(), new);
    let report = diff(
        &new,
        &selected,
        true,
        &first,
        &components(),
        &Triage::default(),
    );
    assert_eq!(report.verdict(), Verdict::Match);
    assert_eq!(report.matched, selected.len());
    let report = diff(
        &new,
        &["dEQP-VK.a.c.1", "dEQP-VK.a.c.2", "dEQP-VK.a.d.only"],
        true,
        &first,
        &components(),
        &Triage::default(),
    );
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].case, "dEQP-VK.a.b.*");
    assert_eq!(report.changes[0].class(), Some(Class::Stale));
}

#[test]
fn a_node_line_covers_only_what_is_under_it() {
    let text = "# cases: 3\ndEQP-VK.a.b.* not-supported\ndEQP-VK.a.bc fail\n";
    let one = Expected::parse(text, "g", "g.txt").unwrap();
    assert_eq!(one.status("dEQP-VK.a.b.x.y"), Status::NotSupported);
    assert_eq!(one.status("dEQP-VK.a.bc"), Status::Fail);
    assert_eq!(one.status("dEQP-VK.a.bd"), Status::Pass);
    for (text, error) in [
        (
            "# cases: 2\ndEQP-VK.a.* fail\ndEQP-VK.a.b fail\n",
            "is under dEQP-VK.a.*",
        ),
        (
            "# cases: 1\ndEQP-VK.a* fail\n",
            "not a deqp-vk case or `<node>.*`",
        ),
        (
            "# cases: 1\ndEQP-VK.*.b fail\n",
            "not a deqp-vk case or `<node>.*`",
        ),
    ] {
        let e = Expected::parse(text, "g", "g.txt").err().unwrap();
        assert!(e.contains(error), "{text:?}: {e}");
    }
}

#[test]
fn the_committed_expected_results_parse() {
    let groups = crate::deqp::groups::Groups::builtin().unwrap();
    for group in &groups.groups {
        let path = dir().join(format!("{}.txt", group.name));
        Expected::load(&path, &group.name).unwrap();
    }
}
