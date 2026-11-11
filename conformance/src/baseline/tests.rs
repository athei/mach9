use super::*;
use crate::outcome::Status;

/// A baseline as `record` writes it.
const TEXT: &str = "\
# Conformance baseline of d3d8_test.exe, one line per test.
# Written by `cargo run -p conformance -- record d3d8`: do not edit.
# wine: wine-11.21-37-g53def89293a
# component: dxmt=8c0b5f1
# component: fex=0123456789ab

[i386-windows device]
depth_blit_test crash failed=1 sites=1 todo_succeeded=0 skipped=0
test_filter fail failed=2 sites=1 todo_succeeded=0 skipped=0
test_fpu_setup ok failed=0 sites=0 todo_succeeded=0 skipped=1

[i386-windows stateblock]
* ok failed=0 sites=0 todo_succeeded=0 skipped=0

[x86_64-windows device]
test_filter not-reached failed=0 sites=0 todo_succeeded=0 skipped=0
";

fn parse(text: &str) -> Result<Baseline, String> {
    Baseline::parse(text, "d3d8", "d3d8.txt")
}

#[test]
fn a_baseline_round_trips() {
    let baseline = parse(TEXT).unwrap();
    assert_eq!(baseline.to_text(), TEXT);
    assert_eq!(parse(&baseline.to_text()).unwrap(), baseline);
    assert_eq!(baseline.header.wine_version, "wine-11.21-37-g53def89293a");
    assert_eq!(baseline.header.components.len(), 2);
    assert_eq!(baseline.entries.len(), 5);
    let whole = TestId {
        leg: PeDir::I386,
        unit: String::from("stateblock"),
        key: Key::Whole,
    };
    assert_eq!(baseline.entries[&whole].status, Status::Ok);
}

/// [`TEXT`] without its prose and in another order.
const SHUFFLED: &str = "\
# component: fex=0123456789ab
# wine: wine-11.21-37-g53def89293a
# component: dxmt=8c0b5f1
[x86_64-windows device]
test_filter not-reached failed=0 sites=0 todo_succeeded=0 skipped=0
[i386-windows stateblock]
* ok failed=0 sites=0 todo_succeeded=0 skipped=0
[i386-windows device]
test_fpu_setup ok failed=0 sites=0 todo_succeeded=0 skipped=1


test_filter fail failed=2 sites=1 todo_succeeded=0 skipped=0
depth_blit_test crash failed=1 sites=1 todo_succeeded=0 skipped=0
";

/// The writer sorts by leg, unit and key and writes its own prose, whatever
/// the text had.
#[test]
fn the_text_is_sorted() {
    assert_eq!(parse(SHUFFLED).unwrap().to_text(), TEXT);
}

#[test]
fn an_empty_baseline_has_only_its_header() {
    let baseline = Baseline {
        exe: String::from("d3d8"),
        header: Header::parse("wine-11.21\n", "").unwrap(),
        entries: BTreeMap::new(),
    };
    let text = baseline.to_text();
    assert!(text.ends_with("# wine: wine-11.21\n"), "{text}");
    assert_eq!(parse(&text).unwrap(), baseline);
}

#[test]
fn each_mistake_is_an_error_with_its_line() {
    let entry = "test_a ok failed=0 sites=0 todo_succeeded=0 skipped=0";
    let start = "# wine: w\n\n[i386-windows device]\n";
    for (text, expected) in [
        (
            format!("[i386-windows device]\n{entry}\n"),
            "d3d8.txt: no `# wine: <version>` line",
        ),
        (
            String::from("# wine: w\n# wine: v\n"),
            "d3d8.txt:2: a second `# wine:` line",
        ),
        (
            String::from("# wine: a b\n"),
            "d3d8.txt:1: \"a b\" is not a Wine version",
        ),
        (
            String::from("# wine: w\n# component: dxmt\n"),
            "d3d8.txt:2: \"dxmt\" is not name=commit",
        ),
        (
            String::from("# wine: w\n# component: a=1\n# component: a=2\n"),
            "d3d8.txt:3: component a is named twice",
        ),
        (
            format!("{start}{entry}\n# note\n"),
            "d3d8.txt:5: a comment after the first section",
        ),
        (
            format!("# wine: w\n{entry}\n"),
            "d3d8.txt:2: a key line before any section",
        ),
        (
            format!("# wine: w\n[aarch64-windows device]\n{entry}\n"),
            "d3d8.txt:2: unknown leg \"aarch64-windows\"",
        ),
        (
            format!("# wine: w\n[i386-windows 3d]\n{entry}\n"),
            "d3d8.txt:2: unit \"3d\" is not a C name",
        ),
        (
            String::from("# wine: w\n[i386-windows device\n"),
            "d3d8.txt:2: a section line must end in `]`",
        ),
        (
            String::from("# wine: w\n[i386-windows  device]\n"),
            "d3d8.txt:2: expected `[<leg> <unit>]`",
        ),
        (
            format!("{start}{entry}\n[i386-windows device]\n{entry}\n"),
            "d3d8.txt:5: section [i386-windows device] again, first on line 3",
        ),
        (
            format!("{start}[i386-windows visual]\n{entry}\n"),
            "d3d8.txt:4: the section of line 3 has no keys",
        ),
        (
            start.to_owned(),
            "d3d8.txt:3: the section of line 3 has no keys",
        ),
        (
            format!("{start}{entry}\n{entry}\n"),
            "d3d8.txt:5: key test_a again in this section",
        ),
        (
            format!("{start}{entry}\n* ok failed=0 sites=0 todo_succeeded=0 skipped=0\n"),
            "d3d8.txt:5: a `*` key must be the only key of its section",
        ),
        (
            format!("{start}* ok failed=0 sites=0 todo_succeeded=0 skipped=0\n{entry}\n"),
            "d3d8.txt:5: a `*` key must be the only key of its section",
        ),
        (
            format!("{start}test/a ok\n"),
            "d3d8.txt:4: \"test/a\" is not a key",
        ),
        (
            format!("{start} {entry}\n"),
            "d3d8.txt:4: \"\" is not a key",
        ),
        (
            format!("{start}test_a passed failed=0 sites=0 todo_succeeded=0 skipped=0\n"),
            "d3d8.txt:4: unknown status \"passed\"",
        ),
        (
            format!("{start}test_a ok failed=0 sites=0 todo_succeeded=0 skipped=0 x\n"),
            "d3d8.txt:4: unexpected \"x\" after the counts",
        ),
        (
            format!("{start}test_a ok failed=0 sites=0:3 todo_succeeded=0 skipped=0\n"),
            "d3d8.txt:4: sites \"0:3\" is not a count",
        ),
    ] {
        let error = parse(&text).unwrap_err();
        assert!(error.contains(expected), "{text:?}: {error}");
    }
}

#[test]
fn a_mistyped_header_field_is_an_error() {
    for (text, line) in [
        ("#wine: w\n", 1),
        ("# wine:w\n", 1),
        ("# wine: w\n# component:x=y\n", 2),
        ("# wine: w\n#  component: x=y\n", 2),
    ] {
        let error = parse(text).unwrap_err();
        assert!(
            error.starts_with(&format!(
                "d3d8.txt:{line}: a header field is `# wine: <version>`"
            )),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn prose_in_the_header_is_free() {
    let text = TEXT.replace(
        "# Written by",
        "# Anything at all, even # wine-ish words.\n# Written by",
    );
    assert_eq!(parse(&text).unwrap(), parse(TEXT).unwrap());
}

#[test]
fn load_reads_a_file_or_finds_none() {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("conformance-test-baselines");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    assert_eq!(Baseline::load(&dir.join("d3d8.txt"), "d3d8").unwrap(), None);
    fs::write(dir.join("d3d8.txt"), TEXT).unwrap();
    fs::write(dir.join(TRIAGE_FILE), "not a baseline\n").unwrap();
    fs::write(dir.join("notes.md"), "not a baseline\n").unwrap();
    let all = load_dir(&dir).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].exe, "d3d8");
    assert_eq!(all[0], parse(TEXT).unwrap());
    fs::write(dir.join("ddraw.txt"), "# no header\n").unwrap();
    let error = load_dir(&dir).unwrap_err();
    assert!(
        error.contains("ddraw.txt: no `# wine: <version>` line"),
        "{error}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_committed_baselines_parse() {
    load_dir(&dir()).unwrap();
}
