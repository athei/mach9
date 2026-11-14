use std::path::PathBuf;

use super::*;

const OUTPUT: &str = "\
[Vulkan Loader] INFO | LAYER:   Insert instance layer \"VK_LAYER_KHRONOS_validation\" (/l/lib/libVkLayer_khronos_validation.dylib)
[Vulkan Loader] LAYER:          vkCreateInstance layer callstack setup to:
Validation Warning: [ Undefined-Value-ShaderOutputNotConsumed-DynamicRendering ] | MessageID = 0x1d2a3b4c
vkCreateGraphicsPipelines(): pCreateInfos[0] Inside the fragment shader, it writes to output Location 1 ...
Objects: 1
    [0] VkPipeline 0x0

Test case 'dEQP-VK.query_pool.statistics_query.a'..
Validation Error: [ VUID-vkCmdBeginQuery-queryPool-01922 ] | MessageID = 0x8c4a2b9c
vkCmdBeginQuery(): queryPool (VkQueryPool 0x1) ... (https://docs.vulkan.org/spec#VUID-vkCmdBeginQuery-queryPool-01922)
Objects: 2
    [0] VkCommandBuffer 0x2
    [1] VkQueryPool 0x1

  Pass (Pass)
Test case 'dEQP-VK.query_pool.statistics_query.b'..
a line of some other process that names VUID-vkCmdDraw-None-08600.
  Pass (Pass)
";

#[test]
fn a_scan_counts_the_instances_and_attributes_each_message_to_its_case() {
    let scan = scan(OUTPUT);
    assert_eq!((scan.instances, scan.with_layer), (1, 1));
    assert_eq!(scan.without_layer(), 0);
    let got: Vec<(Severity, &str, Option<&str>)> = scan
        .messages
        .iter()
        .map(|m| (m.severity, m.id.as_str(), m.case.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            (
                Severity::Warning,
                "Undefined-Value-ShaderOutputNotConsumed-DynamicRendering",
                None
            ),
            (
                Severity::Error,
                "VUID-vkCmdBeginQuery-queryPool-01922",
                Some("dEQP-VK.query_pool.statistics_query.a")
            ),
            (
                Severity::Error,
                "VUID-vkCmdDraw-None-08600",
                Some("dEQP-VK.query_pool.statistics_query.b")
            ),
        ]
    );
    assert!(
        scan.messages[1].text.starts_with("vkCmdBeginQuery(): "),
        "{:?}",
        scan.messages[1]
    );
}

/// A message whose heading came in the middle of a test's check line, the
/// rest of which follows the message, is still read.
#[test]
fn a_heading_inside_a_test_line_is_read() {
    let text = "\
ddraw7.c:123:0.512 Validation Warning: [ Undefined-Value-ShaderOutputNotConsumed-DynamicRendering ] | MessageID = 0x46877e3e
vkCmdDraw(): Inside [VK_SHADER_STAGE_FRAGMENT_BIT] it writes to [Output variable, Location 1]
Objects: 1
    [0] VkCommandBuffer 0x1[zink cmdbuf]

Test failed: Got unexpected color 0x00ff0000.
";
    let scan = scan(text);
    assert_eq!(scan.messages.len(), 1, "{:?}", scan.messages);
    assert_eq!(scan.messages[0].severity, Severity::Warning);
    assert_eq!(
        scan.messages[0].id,
        "Undefined-Value-ShaderOutputNotConsumed-DynamicRendering"
    );
    assert!(scan.messages[0].text.starts_with("vkCmdDraw(): "));
}

#[test]
fn an_instance_without_the_layer_is_counted() {
    let text = "\
[Vulkan Loader] INFO | LAYER:   Insert instance layer \"VK_LAYER_KHRONOS_validation\" (/l)
[Vulkan Loader] LAYER:          vkCreateInstance layer callstack setup to:
[Vulkan Loader] INFO | LAYER:   Insert instance layer \"VK_LAYER_OTHER\" (/o)
[Vulkan Loader] LAYER:          vkCreateInstance layer callstack setup to:
";
    let scan = scan(text);
    assert_eq!((scan.instances, scan.with_layer), (2, 1));
    assert_eq!(scan.without_layer(), 1);
}

#[test]
fn the_pin_is_read_from_redist_env() {
    let redist = "# the layers\nVULKAN_VALIDATION_LAYERS_GIT_TAG=t\n\
                  VULKAN_VALIDATION_LAYERS_GIT_COMMIT=15c64e7\n";
    assert_eq!(pinned_commit(redist), Some("15c64e7"));
    assert_eq!(
        pinned_commit("VULKAN_VALIDATION_LAYERS_GIT_COMMIT=\n"),
        None
    );
    assert!(pin().is_ok());
}

#[test]
fn the_known_list_parses_patterns_and_issues() {
    let known = Known::parse(
        "# header\n\nopengl32/*/* Undefined-Value-* # Zink's blits\n\
         dEQP-VK.query_pool.* VUID-vkCmdBeginQuery-queryPool-01922 issue=82\n",
        "validation.txt",
    )
    .unwrap();
    assert_eq!(known.entries.len(), 2);
    assert_eq!(known.entries[1].issue, Some(82));
    assert_eq!(known.entries[1].line, 4);
    assert_eq!(
        known.find(
            "opengl32/opengl/test_fbo",
            "Undefined-Value-ShaderOutputNotConsumed-DynamicRendering"
        ),
        Some(0)
    );
    assert_eq!(known.find("d3d12/d3d12/test_x", "Undefined-Value-X"), None);
}

#[test]
fn the_known_list_refuses_bad_lines() {
    for (text, error) in [
        ("opengl32/*/*\n", "has no message ID pattern"),
        ("a/b/c X\na/b/c X\n", "again, first on line 1"),
        ("a/b/c X flaky\n", "unknown token"),
        ("a/b/c X issue=0\n", "is not an issue number"),
        ("a/b/c X issue=1 issue=2\n", "two issues"),
    ] {
        let e = Known::parse(text, "v.txt").err().unwrap();
        assert!(e.contains(error), "{text:?}: {e}");
    }
}

#[test]
fn the_committed_lists_parse() {
    Known::load(&crate::baseline::dir().join(KNOWN_FILE)).unwrap();
    Known::load(&crate::deqp::expected::dir().join(KNOWN_FILE)).unwrap();
}

fn occurrence(test: &str, id: &str) -> Occurrence {
    Occurrence {
        test: test.to_owned(),
        label: format!("{test} (label)"),
        log: PathBuf::from("/runs/1/a.log"),
        message: Message {
            severity: Severity::Error,
            id: id.to_owned(),
            text: String::from("vkCmdDraw(): something"),
            case: None,
        },
    }
}

#[test]
fn a_report_counts_the_pairs_no_entry_matches_and_lists_entries_not_seen() {
    let known = Known::parse(
        "a/u/* VUID-known issue=7\na/u/k VUID-gone\nb/u/* VUID-other\n",
        "v.txt",
    )
    .unwrap();
    let occurrences = [
        occurrence("a/u/k", "VUID-known"),
        occurrence("a/u/k", "VUID-known"),
        occurrence("a/u/l", "VUID-known"),
        occurrence("a/u/k", "VUID-new"),
    ];
    let ran = [String::from("a/u/k"), String::from("a/u/l")];
    let report = report(&occurrences, &known, &ran);
    assert_eq!(report.new, 1);
    assert!(
        report
            .summary
            .contains("validation: new error VUID-new: 1 tests, 1 times"),
        "{}",
        report.summary
    );
    assert!(
        report
            .summary
            .contains("validation: known issue=7 error VUID-known: 2 tests, 3 times"),
        "{}",
        report.summary
    );
    assert!(
        report
            .summary
            .contains("validation: known a/u/k VUID-gone (line 2) was not seen"),
        "{}",
        report.summary
    );
    assert!(!report.summary.contains("VUID-other"), "{}", report.summary);
    assert_eq!(report.detail.lines().count(), 3, "{}", report.detail);
}
