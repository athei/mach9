use super::*;

#[test]
fn each_case_takes_the_result_line_after_its_start() {
    let text = "Writing test log into /dev/null\n\nTest case 'dEQP-VK.a.one'..\n  Pass \
                (Rendering succeeded)\n\nTest case 'dEQP-VK.a.two'..\nMESA: warning: something \
                (else)\n  NotSupported (VK_EXT_foo not supported at vktTestCase.cpp:10)\n\nTest \
                case 'dEQP-VK.a.three'..\n  QualityWarning (Precision)\n\nTest case \
                'dEQP-VK.a.four'..\n  DeviceLost (lost)\n\nDONE!\n";
    let output = parse(text);
    assert_eq!(
        output.results,
        [
            (String::from("dEQP-VK.a.one"), Status::Pass),
            (String::from("dEQP-VK.a.two"), Status::NotSupported),
            (String::from("dEQP-VK.a.three"), Status::Warn),
            (String::from("dEQP-VK.a.four"), Status::Crash),
        ]
    );
    assert_eq!(output.unfinished, None);
}

#[test]
fn the_case_without_a_result_is_the_unfinished_one() {
    let text = "Test case 'dEQP-VK.a.one'..\n  Fail (bad)\n\nTest case 'dEQP-VK.a.two'..\n  \
                Unknown (not a result)\n";
    let output = parse(text);
    assert_eq!(
        output.results,
        [(String::from("dEQP-VK.a.one"), Status::Fail)]
    );
    assert_eq!(output.unfinished.as_deref(), Some("dEQP-VK.a.two"));
}

#[test]
fn a_result_line_without_a_case_is_ignored() {
    let output = parse("  Pass (stray)\n");
    assert_eq!(output.results.len(), 0);
    assert_eq!(output.unfinished, None);
}
