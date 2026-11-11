//! `deqp-vk record` and `run` against `stub-deqp-vk.sh`, a shell script that
//! prints what deqp-vk prints, with cases that lose the device and make the
//! cases other processes run at the time lose theirs or fail.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

use conformance::{
    deqp::{
        expected::{Expected, Status},
        groups::Groups,
        run::{self, Options},
    },
    diff::Verdict,
    exec::Limits,
    resume::Mode,
    runner::Conclusion,
    session::Failure,
};

const GROUPS: &str = "[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.g.*\"]\n";

/// What a test changes in how the stub runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Setup {
    /// A `MESA_DIST`, a 10 s idle limit, nothing else.
    Plain,
    /// The Vulkan stack is a bundle, not a `MESA_DIST`.
    Bundle,
    /// The probe names a driver outside the stack.
    ForeignDriver,
    /// The canary fails.
    CanaryFails,
    /// A 1 s idle limit.
    ShortIdle,
    /// The interrupt flag is set from the start.
    Stopped,
}

const PLAIN: Setup = Setup::Plain;

/// A scratch directory under the target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("conformance-test-deqp")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("expected")).unwrap();
    fs::create_dir_all(dir.join("state")).unwrap();
    dir
}

/// The Vulkan stack in `root`, a full bundle or a `MESA_DIST`, and the path
/// its driver has.
fn vulkan(root: &Path, bundle: bool) -> (PathBuf, PathBuf) {
    let dir = root.join(if bundle { "wine" } else { "mesa" });
    let lib = if bundle {
        dir.join("lib/external")
    } else {
        dir.join("lib")
    };
    fs::create_dir_all(&lib).unwrap();
    fs::create_dir_all(dir.join("share/vulkan/icd.d")).unwrap();
    fs::write(lib.join("libvulkan.1.dylib"), "").unwrap();
    if bundle {
        let tests = dir.join("lib/wine/tests");
        fs::create_dir_all(&tests).unwrap();
        fs::write(tests.join("VERSION"), "wine-11.21\n").unwrap();
        fs::write(
            tests.join("COMPONENTS"),
            "mesa=aaaaaaaaaaaa\nvulkan-loader=bbbbbbbbbbbb\ndxmt=dddddddddddd\n",
        )
        .unwrap();
    } else {
        fs::write(dir.join("COMMIT"), "aaaaaaaaaaaa\n").unwrap();
        fs::write(dir.join("LOADER_COMMIT"), "bbbbbbbbbbbb\n").unwrap();
    }
    let driver = std::path::absolute(lib.join("libvulkan_kosmickrisp.dylib")).unwrap();
    (dir, driver)
}

/// A `deqp-vk` dist in `root`, the stub as deqp-vk with `cases` as its
/// mustpass.
fn deqp(root: &Path, cases: &[&str]) -> PathBuf {
    let deqp = root.join("deqp");
    fs::create_dir_all(deqp.join("vulkan")).unwrap();
    let stub = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/stub-deqp-vk.sh");
    fs::copy(stub, deqp.join("deqp-vk")).unwrap();
    let mut mustpass = String::from("dEQP-VK.info.device_properties\n");
    for case in cases {
        mustpass.push_str(case);
        mustpass.push('\n');
    }
    fs::write(deqp.join("mustpass.txt"), mustpass).unwrap();
    fs::write(deqp.join("COMMIT"), "cccccccccccc\n").unwrap();
    deqp
}

fn try_execute(
    root: &Path,
    mode: Mode,
    cases: &[&str],
    setup: Setup,
) -> Result<Conclusion, Failure> {
    let (vulkan, driver) = vulkan(root, setup == Setup::Bundle);
    let options = Options {
        mode,
        groups: Vec::new(),
        cases: Vec::new(),
        vulkan,
        deqp: deqp(root, cases),
        state: root.join("deqp-state"),
        expected: root.join("expected"),
        jobs: 3,
        batch: 1,
        limits: Limits {
            idle: Duration::from_secs(if setup == Setup::ShortIdle { 1 } else { 10 }),
            total: Duration::from_secs(60),
        },
    };
    let driver = if setup == Setup::ForeignDriver {
        PathBuf::from("/opt/homebrew/lib/libMoltenVK.dylib")
    } else {
        driver
    };
    let mut env = vec![
        (OsString::from("PATH"), OsString::from("/bin:/usr/bin")),
        (
            OsString::from("STUB_STATE"),
            root.join("state").into_os_string(),
        ),
        (OsString::from("STUB_DRIVER"), driver.into_os_string()),
    ];
    if setup == Setup::CanaryFails {
        env.push((OsString::from("STUB_CANARY_FAILS"), OsString::from("1")));
    }
    let groups = Groups::parse(GROUPS).unwrap();
    let stop = AtomicBool::new(setup == Setup::Stopped);
    run::execute(&groups, &options, env, &stop)
}

fn execute(root: &Path, mode: Mode, cases: &[&str]) -> Conclusion {
    try_execute(root, mode, cases, PLAIN).unwrap()
}

fn statuses(root: &Path, cases: &[&str]) -> Vec<Status> {
    let expected = Expected::load(&root.join("expected/g.txt"), "g")
        .unwrap()
        .unwrap();
    assert_eq!(expected.cases, cases.len());
    cases.iter().map(|case| expected.status(case)).collect()
}

/// Write expected results in which every one of `cases` passes.
fn expect_all_pass(root: &Path, cases: &[&str]) {
    let expected = Expected {
        group: String::from("g"),
        components: [
            ("mesa", "aaaaaaaaaaaa"),
            ("vk-gl-cts", "cccccccccccc"),
            ("vulkan-loader", "bbbbbbbbbbbb"),
        ]
        .into_iter()
        .map(|(name, commit)| (name.to_owned(), commit.to_owned()))
        .collect(),
        cases: cases.len(),
        results: std::collections::BTreeMap::new(),
    };
    fs::write(root.join("expected/g.txt"), expected.to_text()).unwrap();
}

fn setup_error(result: Result<Conclusion, Failure>) -> String {
    match result {
        Err(Failure::Setup(message)) => message,
        other => panic!("expected a setup failure, got {other:?}"),
    }
}

#[test]
fn a_case_that_lost_the_device_next_to_another_counts_alone() {
    let root = scratch("isolated");
    let cases = [
        "dEQP-VK.g.a.lose",
        "dEQP-VK.g.b.victim",
        "dEQP-VK.g.c.hurt",
        "dEQP-VK.g.d.pass",
        "dEQP-VK.g.e.fail",
    ];
    let recorded = execute(&root, Mode::Record, &cases);
    assert!(
        matches!(recorded, Conclusion::Record { unstable: 0 }),
        "{recorded:?}"
    );
    assert!(root.join("state/victim.lost").exists(), "no victim lost");
    assert!(root.join("state/hurt.failed").exists(), "no case was hurt");
    assert_eq!(
        statuses(&root, &cases),
        [
            Status::Crash,
            Status::Pass,
            Status::Pass,
            Status::Pass,
            Status::Fail
        ]
    );
    let run = execute(&root, Mode::Run, &cases);
    assert!(matches!(run, Conclusion::Run(Verdict::Match)), "{run:?}");
}

#[test]
fn a_crash_next_to_others_that_does_not_come_back_alone_is_unstable() {
    let root = scratch("unstable");
    let cases = ["dEQP-VK.g.a.crowd", "dEQP-VK.g.b.pass"];
    expect_all_pass(&root, &cases);
    let run = execute(&root, Mode::Run, &cases);
    assert!(matches!(run, Conclusion::Run(Verdict::Unstable)), "{run:?}");
    fs::remove_file(root.join("state/crowd.count")).unwrap();
    let recorded = execute(&root, Mode::Record, &cases);
    assert!(
        matches!(recorded, Conclusion::Record { unstable: 1 }),
        "{recorded:?}"
    );
    assert_eq!(statuses(&root, &cases), [Status::Crash, Status::Pass]);
}

#[test]
fn a_flaky_case_is_recorded_with_its_worst_result_and_its_better_ones_do_not_gate() {
    let root = scratch("flaky");
    fs::write(
        root.join("expected/triage.txt"),
        "dEQP-VK.g.a.flip flaky  # pass, then crash\n",
    )
    .unwrap();
    let cases = ["dEQP-VK.g.a.flip", "dEQP-VK.g.b.pass"];
    let recorded = execute(&root, Mode::Record, &cases);
    assert!(
        matches!(recorded, Conclusion::Record { unstable: 0 }),
        "{recorded:?}"
    );
    assert_eq!(statuses(&root, &cases), [Status::Crash, Status::Pass]);
    let run = execute(&root, Mode::Run, &cases);
    assert!(matches!(run, Conclusion::Run(Verdict::Match)), "{run:?}");
}

#[test]
fn a_case_that_fails_now_is_a_regression() {
    let root = scratch("regression");
    let cases = ["dEQP-VK.g.a.later", "dEQP-VK.g.b.pass"];
    let recorded = execute(&root, Mode::Record, &cases);
    assert!(
        matches!(recorded, Conclusion::Record { unstable: 0 }),
        "{recorded:?}"
    );
    fs::write(root.join("state/later.fails"), "").unwrap();
    let run = execute(&root, Mode::Run, &cases);
    assert!(
        matches!(run, Conclusion::Run(Verdict::Regression)),
        "{run:?}"
    );
    assert_eq!(run.exit_code(), 1);
}

#[test]
fn a_case_that_prints_nothing_for_its_limit_is_a_stall() {
    let root = scratch("stall");
    let cases = ["dEQP-VK.g.a.hang", "dEQP-VK.g.b.pass"];
    let setup = Setup::ShortIdle;
    let recorded = try_execute(&root, Mode::Record, &cases, setup).unwrap();
    assert!(
        matches!(recorded, Conclusion::Record { unstable: 0 }),
        "{recorded:?}"
    );
    assert_eq!(statuses(&root, &cases), [Status::Stall, Status::Pass]);
}

#[test]
fn the_bundle_layout_runs_on_the_bundle_s_driver() {
    let root = scratch("bundle");
    let cases = ["dEQP-VK.g.a.pass", "dEQP-VK.g.b.fail"];
    let setup = Setup::Bundle;
    let recorded = try_execute(&root, Mode::Record, &cases, setup).unwrap();
    assert!(
        matches!(recorded, Conclusion::Record { unstable: 0 }),
        "{recorded:?}"
    );
    let text = fs::read_to_string(root.join("expected/g.txt")).unwrap();
    assert!(text.contains("# component: mesa=aaaaaaaaaaaa\n"), "{text}");
    assert!(!text.contains("dxmt"), "{text}");
    let run = try_execute(&root, Mode::Run, &cases, setup).unwrap();
    assert!(matches!(run, Conclusion::Run(Verdict::Match)), "{run:?}");
}

#[test]
fn the_probe_refuses_a_driver_outside_the_stack() {
    let root = scratch("foreign");
    let setup = Setup::ForeignDriver;
    let e = setup_error(try_execute(
        &root,
        Mode::Record,
        &["dEQP-VK.g.a.pass"],
        setup,
    ));
    assert!(e.contains("did not all run on the driver of"), "{e}");
}

#[test]
fn a_device_that_does_not_work_again_stops_the_run() {
    let root = scratch("recover");
    let setup = Setup::CanaryFails;
    let e = setup_error(try_execute(
        &root,
        Mode::Record,
        &["dEQP-VK.g.a.lose", "dEQP-VK.g.b.pass"],
        setup,
    ));
    assert!(e.contains("the device did not work again"), "{e}");
}

#[test]
fn a_process_that_runs_nothing_stops_the_run() {
    let root = scratch("silent");
    let e = setup_error(try_execute(
        &root,
        Mode::Record,
        &["dEQP-VK.g.a.silent"],
        PLAIN,
    ));
    assert!(e.contains("without running any of its 1 cases"), "{e}");
}

#[test]
fn an_interrupted_run_stops_with_the_interrupt() {
    let root = scratch("interrupted");
    let setup = Setup::Stopped;
    let result = try_execute(&root, Mode::Record, &["dEQP-VK.g.a.pass"], setup);
    assert!(matches!(result, Err(Failure::Interrupted)), "{result:?}");
    assert_eq!(Failure::Interrupted.exit_code(), 130);
}
