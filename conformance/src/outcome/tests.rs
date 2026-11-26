use super::*;

#[test]
fn an_outcome_reads_back_what_it_writes() {
    for status in Status::ALL {
        for unexplained in [0, 2] {
            let outcome = Outcome {
                status,
                failed: 12,
                sites: 3,
                todo_succeeded: 1,
                skipped: 4_000_000_000,
                unexplained,
            };
            let text = outcome.to_string();
            let tokens: Vec<&str> = text.split(' ').collect();
            assert_eq!(Outcome::parse(&tokens), Ok(outcome), "{text}");
        }
    }
}

#[test]
fn unexplained_failures_are_written_only_when_there_are_any() {
    let outcome = Outcome {
        status: Status::Fail,
        failed: 1,
        sites: 1,
        todo_succeeded: 0,
        skipped: 1,
        unexplained: 0,
    };
    assert_eq!(
        outcome.to_string(),
        "fail failed=1 sites=1 todo_succeeded=0 skipped=1"
    );
    let outcome = Outcome {
        unexplained: 1,
        ..outcome
    };
    assert_eq!(
        outcome.to_string(),
        "fail failed=1 sites=1 todo_succeeded=0 skipped=1 unexplained=1"
    );
}

#[test]
fn a_bad_outcome_is_an_error() {
    for (text, expected) in [
        ("", "unknown status \"\""),
        (
            "passed failed=0 sites=0 todo_succeeded=0 skipped=0",
            "unknown status",
        ),
        (
            "ok failed=0 sites=0 todo_succeeded=0",
            "missing skipped=<n>",
        ),
        (
            "ok sites=0 failed=0 todo_succeeded=0 skipped=0",
            "expected failed=<n>",
        ),
        (
            "ok failed=-1 sites=0 todo_succeeded=0 skipped=0",
            "is not a count",
        ),
        (
            "ok failed=01 sites=0 todo_succeeded=0 skipped=0",
            "is not a count",
        ),
        (
            "ok failed= sites=0 todo_succeeded=0 skipped=0",
            "is not a count",
        ),
        (
            "ok failed=4294967296 sites=0 todo_succeeded=0 skipped=0",
            "is not a count",
        ),
        (
            "ok failed=0 sites=0 todo_succeeded=0 skipped=0 flaky",
            "unexpected \"flaky\"",
        ),
        (
            "ok failed=0 sites=0 todo_succeeded=0 skipped=0 ",
            "unexpected \"\"",
        ),
        (
            "fail failed=0 sites=0 todo_succeeded=0 skipped=0 unexplained=0",
            "has no unexplained=<n>",
        ),
        (
            "fail failed=0 sites=0 todo_succeeded=0 skipped=0 unexplained=01",
            "is not a count",
        ),
        (
            "fail failed=0 sites=0 todo_succeeded=0 skipped=0 unexplained=1 flaky",
            "unexpected \"flaky\"",
        ),
        (
            "fail failed=0 sites=0 unexplained=1 todo_succeeded=0 skipped=0",
            "expected todo_succeeded=<n>",
        ),
    ] {
        let tokens: Vec<&str> = text.split(' ').collect();
        let error = Outcome::parse(&tokens).unwrap_err();
        assert!(error.contains(expected), "{text}: {error}");
    }
}

#[test]
fn keys_are_function_names_with_prefixes_or_a_star() {
    assert_eq!(Key::parse("*"), Some(Key::Whole));
    assert_eq!(
        Key::parse("d3d12:test_colour_space_support"),
        Some(Key::Function(String::from(
            "d3d12:test_colour_space_support"
        )))
    );
    for bad in [
        "",
        "a b",
        "a/b",
        "**",
        "a#b",
        "test_b0060:fixme:x",
        "d3d10:",
        ":test_a",
        "a-b",
        "a.b",
        "1test",
    ] {
        assert_eq!(Key::parse(bad), None, "{bad}");
    }
}

#[test]
fn statuses_rank_from_ok_to_crash_and_stall() {
    let ranks: Vec<u8> = Status::ALL.iter().map(|status| status.rank()).collect();
    assert_eq!(ranks, [0, 1, 2, 3, 3]);
    for status in Status::ALL {
        assert_eq!(Status::parse(status.as_str()), Some(status));
    }
}
