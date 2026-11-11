use super::*;

const COMPONENTS: &str = "mesa=1a2b3c4d5e6f\nvulkan-loader=0f1e2d3c4b5a\ndxmt=8c0b5f1aa\n\
                          vkd3d-proton=77aa88bb99cc\nfex=0123456789ab\n";

fn header() -> Header {
    Header::parse("wine-11.21-37-g53def89293a\n", COMPONENTS).unwrap()
}

#[test]
fn a_header_reads_the_version_and_the_components() {
    let header = header();
    assert_eq!(header.wine_version, "wine-11.21-37-g53def89293a");
    let names: Vec<&str> = header.components.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["dxmt", "fex", "mesa", "vkd3d-proton", "vulkan-loader"]
    );
    assert_eq!(header.components["dxmt"], "8c0b5f1aa");
    let windows = Header::parse(
        "wine-11.21-37-g53def89293a\r\n",
        &COMPONENTS.replace('\n', "\r\n"),
    )
    .unwrap();
    assert_eq!(windows, header);
}

#[test]
fn bad_headers_are_errors() {
    for (version, components, expected) in [
        ("", "", "VERSION: the first line \"\" is not a version"),
        ("wine 11", "", "is not a version"),
        (
            "wine-11.21",
            "mesa",
            "COMPONENTS:1: \"mesa\" is not name=commit",
        ),
        (
            "wine-11.21",
            "\nmesa=",
            "COMPONENTS:2: \"\" is not a commit",
        ),
        ("wine-11.21", "a b=1", "\"a b\" is not a component name"),
        (
            "wine-11.21",
            "mesa=1\nmesa=2",
            "component mesa is named twice",
        ),
    ] {
        let error = Header::parse(version, components).unwrap_err();
        assert!(
            error.contains(expected),
            "{version:?} {components:?}: {error}"
        );
    }
}

#[test]
fn a_bundle_without_components_cannot_be_read() {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("conformance-test-header");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("VERSION"), "wine-11.21-37-g53def89293a\n").unwrap();
    let error = Header::read(&dir).unwrap_err();
    assert!(error.contains("COMPONENTS"), "{error}");
    fs::write(dir.join("COMPONENTS"), COMPONENTS).unwrap();
    assert_eq!(Header::read(&dir).unwrap(), header());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn changes_list_the_version_and_each_component() {
    let base = header();
    assert_eq!(changes(&base, &base), []);
    let mut run = base.clone();
    run.wine_version = String::from("wine-11.22");
    run.components
        .insert(String::from("dxmt"), String::from("99887766"));
    run.components.remove("fex");
    run.components
        .insert(String::from("mtld3d"), String::from("abcdef0"));
    let changes: Vec<String> = changes(&base, &run)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        changes,
        [
            "Wine wine-11.21-37-g53def89293a -> wine-11.22",
            "dxmt 8c0b5f1aa -> 99887766",
            "fex 0123456789ab -> none",
            "mtld3d none -> abcdef0",
        ]
    );
}

#[test]
fn hashes_of_different_lengths_name_one_commit() {
    assert!(same_wine_version(
        "wine-11.21-37-g53def89293a",
        "wine-11.21-37-g53def89"
    ));
    assert!(!same_wine_version(
        "wine-11.21-37-g53def89293a",
        "wine-11.21-38-g53def89293a"
    ));
    assert!(!same_wine_version(
        "wine-11.21-37-g53def8",
        "wine-11.21-37-g53def89293a"
    ));
    assert!(!same_wine_version("wine-11.21", "wine-11.22"));
    assert!(same_wine_version(
        "wine-11.21-37-g53def89293a-dirty",
        "wine-11.21-37-g53def89-dirty"
    ));
    assert!(!same_wine_version(
        "wine-11.21-37-g53def89293a-dirty",
        "wine-11.21-37-g53def89"
    ));
    assert!(!same_wine_version(
        "wine-11.21-37-g53def89293a",
        "wine-11.21-37-g53def89-dirty"
    ));
    assert!(same_commit("8c0b5f1aa", "8c0b5f1"));
    assert!(!same_commit("8c0b5f1aa", "8c0b5f2"));
    assert!(!same_commit("main", "main2"));
}
