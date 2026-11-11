//! The spawn, drain and watchdog code of `conformance::exec` against
//! `stub-wine.sh`, a shell script that prints what a test executable prints.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use conformance::{
    exec::{self, Error, Finished, Limits, Sampler, Spec, Stall},
    outcome::{Key, Status},
    output::{Crash, Ending, parse_listing, parse_run},
};

const LIMITS: Limits = Limits {
    idle: Duration::from_secs(1),
    total: Duration::from_secs(2),
};

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// A scratch directory under the target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("conformance-test-exec")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The stub's environment: `PATH` for `sleep` and `perl`, and the
/// selection variable.
fn env(selection: &str, value: &str) -> Vec<(OsString, OsString)> {
    vec![
        (
            OsString::from("PATH"),
            OsString::from("/bin:/usr/bin:/usr/sbin"),
        ),
        (OsString::from(selection), OsString::from(value)),
    ]
}

fn run_stub(selection: &str, value: &str, sampler: Option<&Sampler>) -> Finished {
    let stop = AtomicBool::new(false);
    run_stopped(selection, value, sampler, &stop).unwrap()
}

fn run_stopped(
    selection: &str,
    value: &str,
    sampler: Option<&Sampler>,
    stop: &AtomicBool,
) -> Result<Finished, Error> {
    run_with(selection, value, None, sampler, stop)
}

/// Run the stub with `ready` as its `STUB_READY`, when given.
fn run_with(
    selection: &str,
    value: &str,
    ready: Option<&Path>,
    sampler: Option<&Sampler>,
    stop: &AtomicBool,
) -> Result<Finished, Error> {
    let program = tests_dir().join("stub-wine.sh");
    let args = [OsString::from("d3d12_test.exe"), OsString::from("d3d12")];
    let mut env = env(selection, value);
    if let Some(ready) = ready {
        env.push((OsString::from("STUB_READY"), ready.as_os_str().to_owned()));
    }
    let spec = Spec {
        program: &program,
        args: &args,
        env: &env,
        cwd: &tests_dir(),
    };
    exec::run_tree(&spec, &LIMITS, sampler, stop)
}

fn text(finished: &Finished) -> String {
    String::from_utf8_lossy(&finished.output).into_owned()
}

fn status(finished: &Finished, key: &str) -> Status {
    let key = Key::Function(key.to_owned());
    parse_run(&text(finished), "d3d12", &key, finished.ending)
        .outcome
        .status
}

#[test]
fn a_listing_gives_its_keys() {
    let finished = run_stub("WT_LIST", "1", None);
    assert_eq!(finished.ending, Ending::Exited(0));
    let listing = parse_listing(&text(&finished), "d3d12").unwrap();
    assert_eq!(listing.keys, ["test_pass", "test_fail"]);
    assert!(finished.pipe_closed);
}

/// stdout and stderr arrive in one stream in the order they were written.
#[test]
fn a_passing_run_merges_stdout_and_stderr_in_order() {
    let finished = run_stub("WT_ONLY", "test_pass", None);
    assert_eq!(finished.ending, Ending::Exited(0));
    assert_eq!(status(&finished, "test_pass"), Status::Ok);
    let output = text(&finished);
    let stderr = output.find("on stderr").unwrap();
    let stdout = output.find("on stdout").unwrap();
    assert!(stderr < stdout, "{output}");
    assert!(output.starts_with("0024:wt-run:test_pass\n"), "{output}");
    assert!(
        finished
            .trailer(&LIMITS)
            .starts_with("[conformance] exited with code 0 after")
    );
}

#[test]
fn a_failing_run_passes_its_exit_code_through() {
    let finished = run_stub("WT_ONLY", "test_fail", None);
    assert_eq!(finished.ending, Ending::Exited(1));
    assert_eq!(status(&finished, "test_fail"), Status::Fail);
}

#[test]
fn a_run_without_summary_is_a_crash() {
    let finished = run_stub("WT_ONLY", "test_crash", None);
    assert_eq!(finished.ending, Ending::Exited(5));
    let key = Key::Function(String::from("test_crash"));
    let run = parse_run(&text(&finished), "d3d12", &key, finished.ending);
    assert_eq!(run.outcome.status, Status::Crash);
    assert_eq!(run.crash, Some(Crash::NoSummary));
}

/// No output for the idle limit is a stall; the stalled process is sampled
/// before its group is killed.
#[test]
fn silence_past_the_idle_limit_is_a_stall_and_is_sampled() {
    let dir = scratch("idle");
    let sampler = Sampler {
        tool: tests_dir().join("stub-sample.sh"),
        dir: dir.clone(),
        stem: String::from("test_silent"),
        wineserver: Some((PathBuf::from("/nonexistent/wineserver"), dir.clone())),
    };
    let start = Instant::now();
    let finished = run_stub("WT_ONLY", "test_silent", Some(&sampler));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(finished.ending, Ending::Stalled);
    assert_eq!(finished.stall, Some(Stall::Idle));
    assert_eq!(status(&finished, "test_silent"), Status::Stall);
    let sample = fs::read_to_string(dir.join("test_silent.sample.txt")).unwrap();
    assert!(sample.starts_with("stub sample of "), "{sample}");
    assert!(sample.ends_with(" 2 -mayDie\n"), "{sample}");
    let server = fs::read_to_string(dir.join("test_silent.wineserver-sample.txt")).unwrap();
    assert!(
        server.starts_with("no /nonexistent/wineserver serves"),
        "{server}"
    );
    let trailer = finished.trailer(&LIMITS);
    assert!(trailer.contains("stall: no output for 1 s"), "{trailer}");
    assert!(trailer.contains("test_silent.sample.txt"), "{trailer}");
}

/// Output that never stops is a stall at the total limit, and the sampler
/// skips quietly when its tool does not exist.
#[test]
fn a_run_past_the_total_limit_is_a_stall() {
    let dir = scratch("total");
    let sampler = Sampler {
        tool: PathBuf::from("/nonexistent/sample"),
        dir: dir.clone(),
        stem: String::from("test_chatty"),
        wineserver: None,
    };
    let finished = run_stub("WT_ONLY", "test_chatty", Some(&sampler));
    assert_eq!(finished.stall, Some(Stall::Total));
    assert!(finished.elapsed >= LIMITS.total);
    assert!(text(&finished).contains("still busy"));
    assert_eq!(finished.samples, Vec::<PathBuf>::new());
    assert!(fs::read_dir(&dir).unwrap().next().is_none());
}

/// A child left in the test's process group is killed with it, so the pipe
/// ends at once.
#[test]
fn a_child_in_the_group_does_not_hold_the_run() {
    let start = Instant::now();
    let finished = run_stub("WT_ONLY", "test_child", None);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(finished.ending, Ending::Exited(0));
    assert!(finished.pipe_closed);
    assert_eq!(status(&finished, "test_child"), Status::Ok);
}

/// Kills the holder whose pid the stub wrote to a file, also when the test
/// fails.
struct Holder(PathBuf);

impl Holder {
    fn pid(&self) -> Option<String> {
        fs::read_to_string(&self.0)
            .ok()
            .map(|text| text.trim().to_owned())
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if let Some(pid) = self.pid() {
            let _ = std::process::Command::new("/bin/kill")
                .args(["-KILL", &pid])
                .status();
        }
    }
}

/// A process in a group of its own that holds the pipe survives the group
/// kill and does not keep the run waiting past the drain.
#[test]
fn a_holder_outside_the_group_is_drained_for_a_bounded_time() {
    let holder = Holder(scratch("holder").join("ready"));
    let stop = AtomicBool::new(false);
    let start = Instant::now();
    let finished = run_with("WT_ONLY", "test_holder", Some(&holder.0), None, &stop).unwrap();
    assert!(
        start.elapsed() < exec::DRAIN_CAP + Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    let pid = holder.pid().unwrap();
    let alive = std::process::Command::new("/bin/kill")
        .args(["-0", &pid])
        .status()
        .unwrap();
    assert!(
        alive.success(),
        "the holder {pid} did not survive the group kill"
    );
    assert_eq!(finished.ending, Ending::Exited(0));
    assert!(!finished.pipe_closed);
    assert_eq!(status(&finished, "test_holder"), Status::Ok);
    assert!(
        finished
            .trailer(&LIMITS)
            .contains("still held the output pipe")
    );
}

/// A burst right before the exit is read to its last line.
#[test]
fn a_burst_before_the_exit_is_read_whole() {
    let finished = run_stub("WT_ONLY", "test_burst", None);
    assert_eq!(finished.ending, Ending::Exited(0));
    let output = text(&finished);
    assert!(output.contains("stub.c:14: burst line 200000\n"));
    assert_eq!(output.matches("burst line").count(), 200_000);
    assert_eq!(status(&finished, "test_burst"), Status::Ok);
}

#[test]
fn the_interrupt_flag_kills_the_tree() {
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(300));
            stop.store(true, Ordering::SeqCst);
        });
        let start = Instant::now();
        let result = run_stopped("WT_ONLY", "test_silent", None, &stop);
        assert!(matches!(result, Err(Error::Interrupted)));
        assert!(start.elapsed() < LIMITS.idle);
    });
}

#[test]
fn a_step_runs_into_its_log_and_has_a_limit() {
    let dir = scratch("step");
    let log_path = dir.join("session.log");
    let log = fs::File::create(&log_path).unwrap();
    let program = tests_dir().join("stub-wine.sh");
    let args = [OsString::from("x.exe"), OsString::from("unit")];
    let stop = AtomicBool::new(false);
    let failing = env("WT_ONLY", "test_fail");
    let spec = Spec {
        program: &program,
        args: &args,
        env: &failing,
        cwd: &dir,
    };
    let status = exec::run_step(&spec, &log, Duration::from_secs(5), &stop).unwrap();
    assert_eq!(status.code(), Some(1));
    let logged = fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("Test failed: expected 1, got 2"),
        "{logged}"
    );

    let silent = env("WT_ONLY", "test_silent");
    let spec = Spec {
        env: &silent,
        ..spec
    };
    let result = exec::run_step(&spec, &log, Duration::from_millis(500), &stop);
    assert!(matches!(result, Err(Error::TimedOut(_))));
}

#[test]
fn a_missing_program_is_a_spawn_error() {
    let stop = AtomicBool::new(false);
    let program = PathBuf::from("/nonexistent/wine");
    let spec = Spec {
        program: &program,
        args: &[],
        env: &[],
        cwd: &tests_dir(),
    };
    assert!(matches!(
        exec::run_tree(&spec, &LIMITS, None, &stop),
        Err(Error::Spawn(_))
    ));
}
