use super::*;
use crate::{
    model::PeDir,
    outcome::{Key, Status},
};

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

fn id(leg: PeDir, key: &str) -> TestId {
    TestId {
        leg,
        unit: String::from("d3d11"),
        key: Key::Function(key.to_owned()),
    }
}

fn header(wine: &str) -> Header {
    Header::parse(wine, "dxmt=8c0b5f1\n").unwrap()
}

fn map(entries: &[(&TestId, Outcome)]) -> BTreeMap<TestId, Outcome> {
    entries
        .iter()
        .map(|(id, outcome)| ((*id).clone(), *outcome))
        .collect()
}

/// The old baseline: `test_a` and `test_b` on both legs, all passing.
fn old() -> Baseline {
    let entries = [PeDir::I386, PeDir::X86_64]
        .into_iter()
        .flat_map(|leg| [(id(leg, "test_a"), OK), (id(leg, "test_b"), OK)])
        .collect();
    Baseline {
        exe: String::from("d3d11"),
        header: header("wine-11.21"),
        entries,
    }
}

#[test]
fn a_full_record_replaces_everything_and_takes_the_new_header() {
    let a32 = id(PeDir::I386, "test_a");
    let c32 = id(PeDir::I386, "test_c");
    let results = map(&[(&a32, FAIL), (&c32, OK)]);
    let merge = merge(
        "d3d11",
        Some(&old()),
        &header("wine-11.22"),
        &results,
        &Scope::All,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(merge.baseline.entries, results);
    assert_eq!(merge.baseline.header, header("wine-11.22"));
    assert_eq!(merge.added, [(c32, OK)]);
    assert_eq!(merge.changed, [(a32, OK, FAIL)]);
    let dropped: Vec<&TestId> = merge.dropped.iter().map(|(id, _)| id).collect();
    assert_eq!(
        dropped,
        [
            &id(PeDir::I386, "test_b"),
            &id(PeDir::X86_64, "test_a"),
            &id(PeDir::X86_64, "test_b")
        ]
    );
    assert_eq!(merge.unstable, []);
}

#[test]
fn a_record_of_one_leg_keeps_the_other() {
    let a64 = id(PeDir::X86_64, "test_a");
    let results = map(&[(&a64, FAIL)]);
    let merge = merge(
        "d3d11",
        Some(&old()),
        &header("wine-11.21"),
        &results,
        &Scope::Leg(PeDir::X86_64),
        &BTreeMap::new(),
    )
    .unwrap();
    let mut expected = old().entries;
    expected.insert(a64.clone(), FAIL);
    expected.remove(&id(PeDir::X86_64, "test_b"));
    assert_eq!(merge.baseline.entries, expected);
    assert_eq!(merge.changed, [(a64, OK, FAIL)]);
    assert_eq!(merge.dropped, [(id(PeDir::X86_64, "test_b"), OK)]);
    assert_eq!(merge.added, []);
}

#[test]
fn a_record_of_selected_keys_touches_only_them() {
    let b32 = id(PeDir::I386, "test_b");
    let a64 = id(PeDir::X86_64, "test_a");
    let scope = Scope::Tests(BTreeSet::from([b32.clone()]));
    let results = map(&[(&b32, FAIL), (&a64, FAIL)]);
    let merge = merge(
        "d3d11",
        Some(&old()),
        &header("wine-11.21"),
        &results,
        &scope,
        &BTreeMap::new(),
    )
    .unwrap();
    let mut expected = old().entries;
    expected.insert(b32.clone(), FAIL);
    assert_eq!(merge.baseline.entries, expected);
    assert_eq!(merge.changed, [(b32, OK, FAIL)]);
    assert_eq!(merge.dropped, []);
}

#[test]
fn a_partial_record_on_another_build_is_refused() {
    for scope in [
        Scope::Leg(PeDir::I386),
        Scope::Tests(BTreeSet::from([id(PeDir::I386, "test_a")])),
    ] {
        let Err(RecordError::HeaderDiffers(changes)) = merge(
            "d3d11",
            Some(&old()),
            &header("wine-11.22"),
            &BTreeMap::new(),
            &scope,
            &BTreeMap::new(),
        ) else {
            panic!("a partial record on another build was merged");
        };
        assert_eq!(
            changes,
            [HeaderChange::Wine {
                from: String::from("wine-11.21"),
                to: String::from("wine-11.22")
            }]
        );
    }
    let merge = merge(
        "d3d11",
        None,
        &header("wine-11.22"),
        &BTreeMap::new(),
        &Scope::Leg(PeDir::I386),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(merge.baseline.header, header("wine-11.22"));
}

/// A scoped record keeps the old header text when the new one names the
/// same commit with a longer hash, so the file does not change for it.
#[test]
fn a_partial_record_keeps_an_equal_header() {
    let mut old = old();
    old.header = header("wine-11.21-37-g53def89");
    let merge = merge(
        "d3d11",
        Some(&old),
        &header("wine-11.21-37-g53def89293a"),
        &BTreeMap::new(),
        &Scope::Leg(PeDir::I386),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(merge.baseline.header.wine_version, "wine-11.21-37-g53def89");
}

#[test]
fn the_confirm_pass_reruns_changed_and_new_keys() {
    let a32 = id(PeDir::I386, "test_a");
    let b32 = id(PeDir::I386, "test_b");
    let c32 = id(PeDir::I386, "test_c");
    let first = map(&[(&a32, OK), (&b32, FAIL), (&c32, OK)]);
    let confirm = to_confirm(Some(&old()), &first);
    assert_eq!(confirm, BTreeSet::from([b32.clone(), c32.clone()]));
    assert_eq!(to_confirm(None, &first).len(), 3);

    let second = map(&[(&b32, OK), (&c32, OK)]);
    let unstable = unstable(&confirm, &first, &second);
    assert_eq!(unstable, map(&[(&b32, FAIL)]));
    let missing = map(&[(&b32, FAIL)]);
    assert_eq!(
        super::unstable(&confirm, &first, &missing),
        map(&[(&c32, OK)])
    );
    assert_eq!(
        super::unstable(&confirm, &BTreeMap::new(), &BTreeMap::new()),
        BTreeMap::new()
    );

    let merge = merge(
        "d3d11",
        Some(&old()),
        &header("wine-11.21"),
        &first,
        &Scope::Leg(PeDir::I386),
        &unstable,
    )
    .unwrap();
    assert_eq!(merge.baseline.entries[&b32], FAIL);
    assert_eq!(merge.unstable, [(b32.clone(), FAIL)]);
    assert_eq!(merge.added, [(c32, OK)]);
    assert_eq!(merge.changed, [(b32, OK, FAIL)]);
}

#[test]
fn the_worse_outcome_takes_the_worse_status_and_each_larger_count() {
    let crash = outcome(Status::Crash, 0);
    let crash_with_fail_counts = Outcome {
        failed: 2,
        sites: 2,
        ..crash
    };
    assert_eq!(worse(FAIL, crash), crash_with_fail_counts);
    assert_eq!(worse(crash, FAIL), crash_with_fail_counts);
    assert_eq!(worse(OK, OK), OK);
    let skipped = Outcome { skipped: 1, ..OK };
    assert_eq!(worse(OK, skipped), skipped);
    let stall = outcome(Status::Stall, 0);
    assert_eq!(worse(crash, stall), crash);
    assert_eq!(worse(stall, crash), stall);

    let first = Outcome {
        status: Status::Fail,
        failed: 5,
        sites: 1,
        todo_succeeded: 0,
        skipped: 0,
    };
    let second = Outcome {
        status: Status::Fail,
        failed: 3,
        sites: 3,
        todo_succeeded: 1,
        skipped: 2,
    };
    let written = Outcome {
        status: Status::Fail,
        failed: 5,
        sites: 3,
        todo_succeeded: 1,
        skipped: 2,
    };
    assert_eq!(worse(first, second), written);
    assert_eq!(worse(second, first), written);
}

/// A later run that gives either of the two outcomes is no regression
/// against what an unstable key was written with.
#[test]
fn either_outcome_of_an_unstable_key_passes_against_its_entry() {
    use crate::{
        diff::{Class, diff},
        triage::Triage,
    };
    let a32 = id(PeDir::I386, "test_a");
    let first = Outcome {
        status: Status::Fail,
        failed: 5,
        sites: 1,
        todo_succeeded: 0,
        skipped: 0,
    };
    let second = Outcome {
        status: Status::Fail,
        failed: 3,
        sites: 3,
        todo_succeeded: 0,
        skipped: 2,
    };
    let unstable = unstable(
        &BTreeSet::from([a32.clone()]),
        &map(&[(&a32, first)]),
        &map(&[(&a32, second)]),
    );
    let merged = merge(
        "d3d11",
        None,
        &header("wine-11.21"),
        &map(&[(&a32, first)]),
        &Scope::All,
        &unstable,
    )
    .unwrap();
    let triage = Triage::parse("", "triage.txt").unwrap();
    for later in [first, second] {
        let report = diff(
            "d3d11",
            Some(&merged.baseline),
            &header("wine-11.21"),
            &map(&[(&a32, later)]),
            &Scope::All,
            &triage,
        );
        assert!(
            report
                .changes
                .iter()
                .all(|change| change.class() != Some(Class::Regression)),
            "{later}: {}",
            report.text()
        );
    }
}

/// A full record on a new build writes an unstable key with this build's
/// worse outcome, not the old build's entry, under the new header.
#[test]
fn an_unstable_key_of_a_full_record_takes_this_build_s_outcome() {
    let a32 = id(PeDir::I386, "test_a");
    let results = map(&[(&a32, OK)]);
    let merge = merge(
        "d3d11",
        Some(&old()),
        &header("wine-11.22"),
        &results,
        &Scope::All,
        &map(&[(&a32, FAIL)]),
    )
    .unwrap();
    assert_eq!(merge.baseline.header, header("wine-11.22"));
    assert_eq!(merge.baseline.entries, map(&[(&a32, FAIL)]));
    assert_eq!(merge.unstable, [(a32, FAIL)]);
}

/// An unstable key the old baseline lacks is written, and one `results`
/// lacks is still listed.
#[test]
fn an_unstable_new_key_is_written_and_listed() {
    let c32 = id(PeDir::I386, "test_c");
    let d32 = id(PeDir::I386, "test_d");
    let merge = merge(
        "d3d11",
        Some(&old()),
        &header("wine-11.21"),
        &map(&[(&c32, OK)]),
        &Scope::Tests(BTreeSet::from([c32.clone(), d32.clone()])),
        &map(&[(&c32, FAIL), (&d32, FAIL)]),
    )
    .unwrap();
    assert_eq!(merge.baseline.entries[&c32], FAIL);
    assert_eq!(merge.baseline.entries[&d32], FAIL);
    assert_eq!(merge.unstable, [(c32.clone(), FAIL), (d32.clone(), FAIL)]);
    assert_eq!(merge.added, [(c32, FAIL), (d32, FAIL)]);
    let text = merge.text();
    assert!(
        text.ends_with("d3d11: 2 added, 0 dropped, 0 changed, 2 unstable\n"),
        "{text}"
    );
}
