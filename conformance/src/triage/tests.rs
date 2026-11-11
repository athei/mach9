use super::*;
use crate::baseline;

const TEXT: &str = "\
# Notes.
d3d11/d3d11/test_swapchain_flip issue=12 flaky  # races the present thread
ddraw/visual/* issue=40

dxgi/dxgi/d3d12:test_colour_space_support flaky
d3d8/device/test_filter  # only a note
";

fn path(text: &str) -> TestPath {
    TestPath::parse(text).unwrap()
}

#[test]
fn notes_carry_an_issue_and_a_flaky_mark() {
    let triage = Triage::parse(TEXT, "triage.txt").unwrap();
    assert_eq!(triage.notes.len(), 4);
    assert_eq!(
        triage.notes[&path("d3d11/d3d11/test_swapchain_flip")],
        Note {
            issue: Some(12),
            flaky: true,
            line: 2
        }
    );
    assert_eq!(
        triage.notes[&path("ddraw/visual/*")],
        Note {
            issue: Some(40),
            flaky: false,
            line: 3
        }
    );
    assert_eq!(
        triage.notes[&path("d3d8/device/test_filter")],
        Note {
            issue: None,
            flaky: false,
            line: 6
        }
    );
    let flip = Key::Function(String::from("test_swapchain_flip"));
    assert!(triage.is_flaky("d3d11", "d3d11", &flip));
    assert!(!triage.is_flaky("d3d10core", "d3d11", &flip));
    assert!(!triage.is_flaky("ddraw", "visual", &Key::Whole));
    assert!(triage.is_flaky(
        "dxgi",
        "dxgi",
        &Key::Function(String::from("d3d12:test_colour_space_support"))
    ));
}

#[test]
fn each_mistake_is_an_error_with_its_line() {
    for (text, expected) in [
        (
            "d3d11/d3d11",
            "t.txt:1: \"d3d11/d3d11\" is not <exe>/<unit>/<key>",
        ),
        ("d3d11/d3d11/a/b", "is not <exe>/<unit>/<key>"),
        ("d3d 11/d3d11/a", "\"d3d\" is not <exe>/<unit>/<key>"),
        ("d3d.11/d3d11/a", "\"d3d.11\" is not an exe name"),
        ("d3d11/3d/a", "unit \"3d\" is not a C name"),
        ("d3d11/d3d11/a,b", "\"a,b\" is not a key"),
        (
            "\n\nd3d11/d3d11/a issue=0",
            "t.txt:3: \"0\" is not an issue number",
        ),
        ("d3d11/d3d11/a issue=12a", "\"12a\" is not an issue number"),
        ("d3d11/d3d11/a issue=#12", "\"\" is not an issue number"),
        ("d3d11/d3d11/a issue=", "\"\" is not an issue number"),
        ("d3d11/d3d11/a flaky flaky", "`flaky` twice"),
        ("d3d11/d3d11/a issue=1 issue=2", "two issues"),
        ("d3d11/d3d11/a Flaky", "unknown token \"Flaky\""),
        ("d3d11/d3d11/a 12", "unknown token \"12\""),
        (
            "d3d11/d3d11/a\nd3d11/d3d11/a flaky",
            "t.txt:2: d3d11/d3d11/a again, first on line 1",
        ),
    ] {
        let Err(error) = Triage::parse(text, "t.txt") else {
            panic!("{text:?} parsed");
        };
        assert!(error.contains(expected), "{text:?}: {error}");
    }
}

/// Every test the committed triage file names is in a committed baseline,
/// in some leg.
#[test]
fn every_triage_entry_names_a_test_of_a_committed_baseline() {
    let triage = Triage::load(&Triage::path()).unwrap();
    let baselines = baseline::load_dir(&baseline::dir()).unwrap();
    let missing: Vec<String> = triage
        .notes
        .iter()
        .filter(|(test, _)| {
            !baselines.iter().any(|baseline| {
                baseline.exe == test.exe
                    && baseline
                        .entries
                        .keys()
                        .any(|id| id.unit == test.unit && id.key == test.key)
            })
        })
        .map(|(test, note)| format!("triage.txt:{}: {test} is in no baseline", note.line))
        .collect();
    assert!(missing.is_empty(), "{missing:#?}");
}
