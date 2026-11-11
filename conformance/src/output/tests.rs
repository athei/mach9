use super::*;

const PASS: &str = include_str!("../../tests/fixtures/output/pass.txt");
const FAILURES: &str = include_str!("../../tests/fixtures/output/failures.txt");
const QUEUED: &str = include_str!("../../tests/fixtures/output/queued.txt");
const REPEATED: &str = include_str!("../../tests/fixtures/output/repeated.txt");
const CRASH: &str = include_str!("../../tests/fixtures/output/crash.txt");
const SIGNAL: &str = include_str!("../../tests/fixtures/output/signal.txt");
const STALL: &str = include_str!("../../tests/fixtures/output/stall.txt");
const NOT_REACHED: &str = include_str!("../../tests/fixtures/output/not-reached.txt");
const CHILD_FAILURES: &str = include_str!("../../tests/fixtures/output/child-failures.txt");
const CHILD_CRASH: &str = include_str!("../../tests/fixtures/output/child-crash.txt");
const WHOLE: &str = include_str!("../../tests/fixtures/output/whole.txt");
const SILENCED: &str = include_str!("../../tests/fixtures/output/silenced.txt");
const LISTING: &str = include_str!("../../tests/fixtures/output/listing.txt");

fn key(name: &str) -> Key {
    Key::Function(name.to_owned())
}

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

/// `text` with every line ending in `\r\n`, as the CRT of a test writes
/// in text mode.
fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

/// Parse the run of `key` of `unit` in `output`, and check that the same
/// output with Windows line endings gives the same result.
fn parsed(output: &str, unit: &str, key: &Key, ending: Ending) -> Run {
    let run = parse_run(output, unit, key, ending);
    let windows = parse_run(&crlf(output), unit, key, ending);
    assert_eq!(windows.outcome, run.outcome);
    assert_eq!(windows.crash, run.crash);
    assert_eq!(windows.root, run.root);
    run
}

#[test]
fn a_clean_pass_counts_its_skips_and_ignores_todos_and_foreign_lines() {
    let run = parsed(PASS, "d3d11", &key("test_create_device"), Ending::Exited(0));
    assert_eq!(run.outcome, outcome(Status::Ok, 0, 0, 0, 2));
    assert_eq!(run.crash, None);
    assert_eq!(run.root, Some(0x24));
}

/// The failures fixture has `WINETEST_TIME` on, contexts, a message that
/// quotes `Test failed:`, a trace whose text holds it, both flaky kinds and
/// a todo success, which the summary counts as a failure.
#[test]
fn failures_on_several_sites_with_elapsed_times() {
    let run = parsed(FAILURES, "d3d11", &key("test_texture"), Ending::Exited(5));
    assert_eq!(run.outcome, outcome(Status::Fail, 4, 3, 1, 1));
    assert_eq!(run.crash, None);
}

#[test]
fn a_queued_test_runs_long_after_its_marker() {
    let run = parsed(
        QUEUED,
        "d3d11",
        &key("test_generate_mips"),
        Ending::Exited(3),
    );
    assert_eq!(run.outcome, outcome(Status::Fail, 3, 1, 0, 0));
}

#[test]
fn a_repeated_marker_counts_every_call_s_output() {
    let run = parsed(
        REPEATED,
        "opengl",
        &key("test_bitmap_rendering"),
        Ending::Exited(2),
    );
    assert_eq!(run.outcome, outcome(Status::Fail, 2, 1, 0, 0));
    assert_eq!(run.root, Some(0x60));
}

#[test]
fn a_root_without_summary_crashed_and_keeps_its_counts() {
    let run = parsed(CRASH, "d3d11", &key("test_texture"), Ending::Exited(5));
    assert_eq!(run.outcome, outcome(Status::Crash, 1, 1, 0, 0));
    assert_eq!(run.crash, Some(Crash::NoSummary));
}

#[test]
fn death_by_signal_is_a_crash_even_after_a_summary() {
    let run = parsed(SIGNAL, "d3d11", &key("test_fl9_draw"), Ending::Signaled(6));
    assert_eq!(run.outcome, outcome(Status::Crash, 1, 1, 0, 0));
    assert_eq!(run.crash, Some(Crash::Signal(6)));

    let summary = format!(
        "{PASS}0024:d3d11: 1 tests executed (0 marked as todo, 0 as flaky, 0 failures), 0 \
         skipped.\n"
    );
    let run = run_once(&summary, "test_create_device", Ending::Signaled(11));
    assert_eq!(run.outcome.status, Status::Crash);
    assert_eq!(run.crash, Some(Crash::Signal(11)));
}

fn run_once(output: &str, name: &str, ending: Ending) -> Run {
    parse_run(output, "d3d11", &key(name), ending)
}

#[test]
fn a_stall_is_a_stall_whatever_the_output() {
    let run = parsed(STALL, "dxgi", &key("test_swapchain_flip"), Ending::Stalled);
    assert_eq!(run.outcome, outcome(Status::Stall, 1, 1, 0, 0));
    assert_eq!(run.crash, None);
    let run = parsed(PASS, "d3d11", &key("test_create_device"), Ending::Stalled);
    assert_eq!(run.outcome.status, Status::Stall);
}

#[test]
fn a_key_without_its_marker_was_not_reached() {
    let run = parsed(
        NOT_REACHED,
        "d3d12",
        &key("test_create_device"),
        Ending::Exited(0),
    );
    assert_eq!(run.outcome, outcome(Status::NotReached, 0, 0, 0, 1));
    assert_eq!(run.crash, None);
    assert_eq!(run.root, Some(0x44));
}

#[test]
fn a_key_without_its_marker_or_summary_crashed() {
    let output = NOT_REACHED.replace("0044:d3d12: 0 tests", "0044:d3d12: (cut) 0 tests");
    let run = parsed(
        &output,
        "d3d12",
        &key("test_create_device"),
        Ending::Exited(5),
    );
    assert_eq!(run.outcome.status, Status::Crash);
    assert_eq!(run.crash, Some(Crash::NoSummary));
}

#[test]
fn another_key_s_marker_does_not_count() {
    let run = parsed(
        PASS,
        "d3d11",
        &key("test_device_interfaces"),
        Ending::Exited(0),
    );
    assert_eq!(run.outcome.status, Status::NotReached);
}

/// The parent's summary adds the child's two failures to its own one; the
/// failure lines count each once, and the skips are the child's.
#[test]
fn a_child_s_failures_are_not_counted_twice() {
    let run = parsed(
        CHILD_FAILURES,
        "vulkan",
        &key("test_external_memory"),
        Ending::Exited(3),
    );
    assert_eq!(run.outcome, outcome(Status::Fail, 3, 3, 0, 1));
    assert_eq!(run.root, Some(0x40));
    assert_eq!(run.crash, None);
}

#[test]
fn a_child_crash_makes_the_run_a_crash() {
    let run = parsed(
        CHILD_CRASH,
        "d3d11",
        &key("test_create_device"),
        Ending::Exited(1),
    );
    assert_eq!(run.outcome, outcome(Status::Crash, 0, 0, 0, 0));
    assert_eq!(
        run.crash,
        Some(Crash::Child(String::from(
            "d3d11.c:2344: unhandled exception c0000005 in child process 0050"
        )))
    );
}

#[test]
fn an_exit_code_other_than_the_failure_count_is_a_crash() {
    let run = parsed(PASS, "d3d11", &key("test_create_device"), Ending::Exited(5));
    assert_eq!(run.outcome.status, Status::Crash);
    assert_eq!(
        run.crash,
        Some(Crash::ExitCode {
            code: 5,
            failures: 0
        })
    );
    let run = parsed(FAILURES, "d3d11", &key("test_texture"), Ending::Exited(4));
    assert_eq!(
        run.crash,
        Some(Crash::ExitCode {
            code: 4,
            failures: 5
        })
    );
}

#[test]
fn the_exit_code_is_the_failure_count_capped_at_255() {
    let summary = "0024:wt-run:test_a\n0024:d3d11: 400 tests executed (0 marked as todo, 0 as \
                   flaky, 300 failures), 0 skipped.\n";
    assert_eq!(
        run_once(summary, "test_a", Ending::Exited(255)).outcome,
        outcome(Status::Fail, 0, 0, 0, 0)
    );
    assert_eq!(
        run_once(summary, "test_a", Ending::Exited(44)).crash,
        Some(Crash::ExitCode {
            code: 44,
            failures: 300
        })
    );
}

/// A failure the summary counts but no line shows, here a message without
/// its newline that hides the next check, still fails the run.
#[test]
fn summary_failures_no_line_explains_fail_the_run() {
    let output = "0024:wt-run:test_a\nd3d11.c:5: Test failed: no newlined3d11.c:6: Test failed: \
                  hidden\nd3d11.c:7: Test succeeded inside todo block: x\n0024:d3d11: 3 tests \
                  executed (0 marked as todo, 0 as flaky, 3 failures), 0 skipped.\n";
    let run = run_once(output, "test_a", Ending::Exited(3));
    assert_eq!(run.outcome, outcome(Status::Fail, 1, 1, 1, 0));
    let todo_only = "0024:wt-run:test_a\nd3d11.c:7: Test succeeded inside todo block: x\n0024:d3d11: \
                     1 tests executed (0 marked as todo, 0 as flaky, 1 failure), 0 skipped.\n";
    let run = run_once(todo_only, "test_a", Ending::Exited(1));
    assert_eq!(run.outcome, outcome(Status::Ok, 0, 0, 1, 0));
}

#[test]
fn a_whole_unit_s_root_is_its_last_summary() {
    let run = parsed(WHOLE, "d3d", &Key::Whole, Ending::Exited(1));
    assert_eq!(run.outcome, outcome(Status::Fail, 1, 1, 0, 1));
    assert_eq!(run.root, Some(0x70));
    let run = parsed(WHOLE, "visual", &Key::Whole, Ending::Exited(1));
    assert_eq!(run.crash, Some(Crash::NoSummary));
}

/// A whole unit whose child prints its summary first: the parent waits for
/// it, so the parent's summary comes last and is the root's.
#[test]
fn a_whole_unit_s_child_summary_before_the_parent_s_is_not_the_root() {
    let output = "\
0074:d3d: 3 tests executed (0 marked as todo, 0 as flaky, 2 failures), 0 skipped.
d3d.c:20: 2 failures in child process
0070:d3d: 9 tests executed (0 marked as todo, 0 as flaky, 2 failures), 1 skipped.
";
    let run = parsed(output, "d3d", &Key::Whole, Ending::Exited(2));
    assert_eq!(run.root, Some(0x70));
    assert_eq!(run.crash, None);
    assert_eq!(run.outcome, outcome(Status::Fail, 0, 0, 0, 1));
}

/// A child that printed its marker and no summary ended early, which its
/// parent's output does not show when no exception killed it.
#[test]
fn a_child_without_summary_is_a_crash() {
    let output = CHILD_FAILURES.replace(
        "0048:vulkan: 12 tests executed (0 marked as todo, 0 as flaky, 2 failures), 1 skipped.\n",
        "",
    );
    let run = parsed(
        &output,
        "vulkan",
        &key("test_external_memory"),
        Ending::Exited(3),
    );
    assert_eq!(run.outcome.status, Status::Crash);
    assert_eq!(run.crash, Some(Crash::ChildNoSummary(0x48)));
    assert_eq!(run.root, Some(0x40));
}

/// A child started for another unit prints that unit's summary, which
/// finishes it.
#[test]
fn a_child_of_another_unit_finishes_with_that_unit_s_summary() {
    let output = CHILD_FAILURES.replace("0048:vulkan: 12 tests", "0048:resource: 12 tests");
    assert_ne!(output, CHILD_FAILURES);
    let run = parsed(
        &output,
        "vulkan",
        &key("test_external_memory"),
        Ending::Exited(3),
    );
    assert_eq!(run.crash, None);
    assert_eq!(run.outcome.status, Status::Fail);
}

#[test]
fn silenced_skips_count_as_skips() {
    let run = parsed(
        SILENCED,
        "d3d11",
        &key("test_format_support"),
        Ending::Exited(0),
    );
    assert_eq!(run.outcome, outcome(Status::Ok, 0, 0, 0, 5));
}

#[test]
fn colour_codes_before_a_line_are_dropped() {
    let output = "0024:wt-run:test_a\n\x1b[1;91md3d11.c:5: Test failed: x\n\x1b[0m\x1b[1;91md3d11.c:6: \
                  Test failed: y\n\x1b[0m0024:d3d11: 2 tests executed (0 marked as todo, 0 as \
                  flaky, 2 failures), 0 skipped.\n";
    let run = run_once(output, "test_a", Ending::Exited(2));
    assert_eq!(run.outcome, outcome(Status::Fail, 2, 2, 0, 0));
}

#[test]
fn a_summary_of_one_failure_is_singular() {
    let output = "0024:wt-run:test_a\nd3d11.c:5: Test failed: x\n0024:d3d11: 1 tests executed (0 \
                  marked as todo, 0 as flaky, 1 failure), 0 skipped.\n";
    let run = run_once(output, "test_a", Ending::Exited(1));
    assert_eq!(run.outcome, outcome(Status::Fail, 1, 1, 0, 0));
}

/// Lines that look like checks or summaries but are not count nothing.
#[test]
fn near_misses_count_nothing() {
    let output = "0024:wt-run:test_a\n\
                  d3d11.c:x: Test failed: no line number\n\
                  d3d11.c:5:0.12 Test failed: two decimals\n\
                  d3d11.c:5:Test failed: no space\n\
                  my file.c:5: Test failed: a space in the file\n\
                  \x20d3d11.c:5: Test failed: indented\n\
                  0024:fixme:d3d11:d3d11_device_CreateTexture2D Test failed: in a fixme\n\
                  0024:0028:fixme:d3d:x stub\n\
                  024:d3d11: 2 tests executed (0 marked as todo, 0 as flaky, 2 failures), 9 skipped.\n\
                  0024:d3d11: 2 tests executed (0 marked as todo, 0 as flaky, 2 failures), 9 skipped\n\
                  0024:d3d11: 0 tests executed (0 marked as todo, 0 as flaky, 0 failures), 0 skipped.\n";
    let run = run_once(output, "test_a", Ending::Exited(0));
    assert_eq!(run.outcome, outcome(Status::Ok, 0, 0, 0, 0));
}

#[test]
fn a_listing_gives_distinct_keys_in_order() {
    let listing = parse_listing(LISTING, "opengl").unwrap();
    assert_eq!(
        listing.keys,
        [
            "test_bitmap_rendering",
            "test_dc",
            "test_choosepixelformat_flag_is_ignored_when_unset",
            "test_pbuffers",
        ]
    );
    assert!(listing.is_trustworthy());
    let windows = parse_listing(&crlf(LISTING), "opengl").unwrap();
    assert_eq!(windows.keys, listing.keys);
}

#[test]
fn a_listing_with_failures_is_not_trustworthy() {
    let output = LISTING.replace(
        "opengl.c:6160: Pixel format count 120.",
        "opengl.c:6160: Test failed: wglGetProcAddress failed.",
    );
    let listing = parse_listing(&output, "opengl").unwrap();
    assert_eq!(listing.keys.len(), 4);
    assert!(!listing.is_trustworthy());
    assert_eq!(
        listing.failures,
        ["opengl.c:6160: Test failed: wglGetProcAddress failed."]
    );
}

/// Only the listing process's lines count, and each must hold a key: a
/// line of another process glued to one is an error.
#[test]
fn a_listing_takes_valid_keys_of_its_own_process_only() {
    let other = LISTING.replace(
        "0060:wt-list:test_dc\n",
        "0060:wt-list:test_dc\n0064:wt-list:test_child_only\n",
    );
    assert_eq!(
        parse_listing(&other, "opengl").unwrap().keys,
        parse_listing(LISTING, "opengl").unwrap().keys
    );
    let glued = LISTING.replace(
        "0060:wt-list:test_dc\n0060:wt-list:test_bitmap_rendering\n0060:fixme:",
        "0060:wt-list:test_dc\n0060:wt-list:test_bitmap_rendering0060:fixme:",
    );
    assert_ne!(glued, LISTING);
    let error = listing_error(&glued, "opengl");
    assert!(
        matches!(&error.problem, ListingProblem::BadKey(key) if key.starts_with("test_bitmap_rendering0060:fixme:")),
        "{:?}",
        error.problem
    );
    let prefixed = LISTING.replace("0060:wt-list:test_dc\n", "0060:wt-list:d3d10:test_dc\n");
    assert_eq!(
        parse_listing(&prefixed, "opengl").unwrap().keys[1],
        "d3d10:test_dc"
    );
}

fn listing_error(output: &str, unit: &str) -> ListingError {
    match parse_listing(output, unit) {
        Ok(listing) => panic!("listed {:?}", listing.keys),
        Err(error) => error,
    }
}

/// A unit whose setup skips and returns lists nothing; the same output as
/// the run of a key that was not reached.
#[test]
fn a_listing_without_keys_carries_its_skips() {
    let error = listing_error(NOT_REACHED, "d3d12");
    assert_eq!(error.problem, ListingProblem::NoKeys);
    assert_eq!(
        error.skips,
        ["d3d12.c:1580: Tests skipped: Failed to create a D3D12 device."]
    );
}

#[test]
fn a_listing_without_summary_is_an_error() {
    let output = LISTING.replace("0060:opengl: 4 tests", "0060:opengl: 4 tests, then a crash");
    let output = format!("{output}opengl.c:12: Test failed: before the crash\n");
    let error = listing_error(&output, "opengl");
    assert_eq!(error.problem, ListingProblem::NoSummary);
    assert_eq!(
        error.failures,
        ["opengl.c:12: Test failed: before the crash"]
    );
    let error = listing_error(LISTING, "other");
    assert_eq!(error.problem, ListingProblem::NoSummary);
}

/// No input makes the parser panic: every prefix of every fixture, which
/// cuts lines, numbers and escape codes.
#[test]
fn no_input_panics() {
    for text in [
        PASS,
        FAILURES,
        CRASH,
        CHILD_FAILURES,
        SILENCED,
        LISTING,
        "\x1b[",
        "\x1b[1;",
        "0024:",
        "0024:d3d11:",
        "0024:d3d11:1.",
        "a.c:1:",
        "a.c:99999999999: Test failed: x",
    ] {
        for (cut, _) in text.char_indices() {
            let _ = parse_run(&text[..cut], "d3d11", &key("test_a"), Ending::Exited(0));
            let _ = parse_listing(&text[..cut], "d3d11");
        }
    }
}
