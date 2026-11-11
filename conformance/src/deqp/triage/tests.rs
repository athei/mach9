use super::*;

#[test]
fn patterns_mark_their_cases() {
    let triage = Triage::parse(
        "# notes\n\ndEQP-VK.sync.a issue=63 flaky  # races\ndEQP-VK.draw.layer.* issue=61\n",
        "triage.txt",
    )
    .unwrap();
    assert!(triage.is_flaky("dEQP-VK.sync.a"));
    assert!(!triage.is_flaky("dEQP-VK.sync.ab"));
    assert!(!triage.is_flaky("dEQP-VK.draw.layer.x"));
    assert_eq!(triage.notes[1].issue, Some(61));
}

#[test]
fn bad_lines_are_refused() {
    for (text, error) in [
        ("draw.a flaky\n", "not a deqp-vk case pattern"),
        ("dEQP-VK.a flaky flaky\n", "`flaky` twice"),
        ("dEQP-VK.a issue=1 issue=2\n", "two issues"),
        ("dEQP-VK.a issue=x\n", "not an issue number"),
        ("dEQP-VK.a slow\n", "unknown token"),
        (
            "dEQP-VK.a\ndEQP-VK.a flaky\n",
            "triage.txt:2: dEQP-VK.a again",
        ),
    ] {
        let e = Triage::parse(text, "triage.txt").err().unwrap();
        assert!(e.contains(error), "{text:?}: {e}");
    }
}

#[test]
fn the_committed_triage_parses() {
    Triage::load(&crate::deqp::expected::dir().join(TRIAGE_FILE)).unwrap();
}
