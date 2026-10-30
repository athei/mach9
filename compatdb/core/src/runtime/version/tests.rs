use super::*;
use crate::{Exe, Fingerprint, Rule};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// One `VS_VERSIONINFO` node as a resource compiler lays it out: the three
/// header words, the key, padding, the value, then, when there are any,
/// padding and the children. `text` sets `wType`, which decides whether
/// `wValueLength` counts characters or bytes. `wLength` leaves out the
/// padding after the node, as real resources do.
fn version_node(key: &str, value: &[u16], text: bool, children: &[u16]) -> Vec<u16> {
    let mut node = vec![0, 0, u16::from(text)];
    node.extend(key.encode_utf16());
    node.push(0);
    node.resize(node.len().next_multiple_of(2), 0);
    node.extend(value);
    if !children.is_empty() {
        node.resize(node.len().next_multiple_of(2), 0);
        node.extend(children);
    }
    // The fixtures stay far below 64 KB. Were one to grow past it, the
    // saturated length would make the tests that parse it fail.
    let byte_len = |units: usize| u16::try_from(units.saturating_mul(2)).unwrap_or(u16::MAX);
    let length = byte_len(node.len());
    let value_length = if text {
        u16::try_from(value.len()).unwrap_or(u16::MAX)
    } else {
        byte_len(value.len())
    };
    if let [w_length, w_value_length, ..] = node.as_mut_slice() {
        *w_length = length;
        *w_value_length = value_length;
    }
    node
}

/// Sibling nodes, each starting on a 32-bit boundary.
fn siblings(nodes: &[Vec<u16>]) -> Vec<u16> {
    let mut out = Vec::new();
    for node in nodes {
        out.resize(out.len().next_multiple_of(2), 0);
        out.extend(node);
    }
    out
}

/// A `String` entry; an empty value is the NUL alone.
fn string_entry(key: &str, value: &str) -> Vec<u16> {
    let mut text: Vec<u16> = value.encode_utf16().collect();
    text.push(0);
    version_node(key, &text, true, &[])
}

/// A whole `VS_VERSIONINFO`: the root with a zeroed `VS_FIXEDFILEINFO`, a
/// `StringFileInfo` holding one string table per entry of `tables`, and a
/// `VarFileInfo` after it.
fn version_resource(tables: &[(&str, &[(&str, &str)])]) -> Vec<u16> {
    let tables: Vec<Vec<u16>> = tables
        .iter()
        .map(|(language, entries)| {
            let strings: Vec<Vec<u16>> = entries
                .iter()
                .map(|(key, value)| string_entry(key, value))
                .collect();
            version_node(language, &[], false, &siblings(&strings))
        })
        .collect();
    let string_info = version_node("StringFileInfo", &[], false, &siblings(&tables));
    let translation = version_node("Translation", &[0x0409, 0x04b0], false, &[]);
    let var_info = version_node("VarFileInfo", &[], false, &translation);
    version_node(
        "VS_VERSION_INFO",
        &[0; 26],
        false,
        &siblings(&[string_info, var_info]),
    )
}

#[test]
fn version_info_reads_the_standard_fields_from_a_resource() {
    let blob = version_resource(&[(
        "040904b0",
        &[
            ("CompanyName", "Rockstar Games"),
            ("FileVersion", "1.0.0.1"),
            ("OriginalFilename", "Launcher.exe"),
            ("ProductName", "Rockstar Games Launcher"),
        ],
    )]);
    let vi = version_info(&blob);
    assert_eq!(vi.company, "Rockstar Games");
    assert_eq!(vi.product, "Rockstar Games Launcher");
    assert_eq!(vi.original_filename, "Launcher.exe");
}

#[test]
fn an_empty_value_followed_by_another_entry_is_empty() {
    // The entry after the empty one must not be read as its value.
    let blob = version_resource(&[(
        "040904b0",
        &[
            ("CompanyName", ""),
            ("FileDescription", "Game"),
            ("OriginalFilename", ""),
            ("ProductName", ""),
            ("ProductVersion", "1.0.0.0"),
        ],
    )]);
    assert_eq!(version_info(&blob), VersionInfo::default());
}

#[test]
fn an_empty_value_at_the_end_of_a_table_is_empty() {
    // Nothing of the next table or of VarFileInfo leaks into it either.
    let blob = version_resource(&[
        (
            "040904b0",
            &[("CompanyName", "Acme"), ("OriginalFilename", "")],
        ),
        ("040704b0", &[("ProductName", "Rocket Sled")]),
    ]);
    let vi = version_info(&blob);
    assert_eq!(vi.company, "Acme");
    assert_eq!(vi.original_filename, "");
    // A field the first table lacks comes from the next one.
    assert_eq!(vi.product, "Rocket Sled");
    let last = version_resource(&[("040904b0", &[("ProductName", "")])]);
    assert_eq!(version_info(&last), VersionInfo::default());
}

#[test]
fn the_nostale_client_has_only_a_company() {
    // The string table of NosTale's NostaleClientX.exe (block 041203B5),
    // whose only non-empty names are the company and the versions.
    // Reading the values as NUL-separated runs takes the next entry's
    // header and key for the value of each empty one, so the product read
    // as "4\u{8}\u{1}ProductVersion".
    let blob = version_resource(&[(
        "041203B5",
        &[
            ("CompanyName", "Entwell"),
            ("FileDescription", ""),
            ("FileVersion", "0.9.3.3259"),
            ("InternalName", ""),
            ("LegalCopyright", ""),
            ("LegalTrademarks", ""),
            ("OriginalFilename", ""),
            ("ProductName", ""),
            ("ProductVersion", "1.0.0.0"),
            ("Comments", ""),
            ("PrivateBuild", ""),
            ("SpecialBuild", ""),
        ],
    )]);
    // Two entries as the exe stores them, which pins the fixture's layout
    // to the real one.
    let entry = |header: [u16; 3], text: &str| {
        let mut units = header.to_vec();
        units.extend(text.encode_utf16());
        units
    };
    assert_eq!(
        string_entry("CompanyName", "Entwell"),
        entry([0x30, 8, 1], "CompanyName\0\0Entwell\0")
    );
    assert_eq!(
        string_entry("ProductName", ""),
        entry([0x22, 1, 1], "ProductName\0\0\0")
    );
    let vi = version_info(&blob);
    assert_eq!(
        vi,
        VersionInfo {
            company: "Entwell".into(),
            ..VersionInfo::default()
        }
    );
    // A rule that pins a field the exe leaves empty does not match it.
    let pinned = |product: Option<&str>, original_filename: Option<&str>| Rule {
        exe: Exe::literal("NostaleClientX.exe"),
        company: Some(Fingerprint::literal("Entwell")),
        product: product.map(Fingerprint::literal),
        original_filename: original_filename.map(Fingerprint::literal),
        ..Rule::default()
    };
    assert!(pinned(None, None).matches("NostaleClientX.exe", &vi, None));
    assert!(!pinned(Some("ProductVersion"), None).matches("NostaleClientX.exe", &vi, None));
    assert!(!pinned(None, Some("ProductName")).matches("NostaleClientX.exe", &vi, None));
}

#[test]
fn keys_match_case_insensitively_and_the_first_entry_wins() {
    let blob = version_resource(&[
        ("040904b0", &[("companyname", "First")]),
        ("040704b0", &[("CompanyName", "Second")]),
    ]);
    assert_eq!(version_info(&blob).company, "First");
}

#[test]
fn a_malformed_resource_yields_empty_fields() -> TestResult {
    let blob = version_resource(&[(
        "040904b0",
        &[("CompanyName", "Acme"), ("ProductName", "Rocket Sled")],
    )]);
    // Cut short anywhere: never a panic, and never a value that is not
    // the field's own.
    for len in 0..blob.len() {
        let vi = version_info(blob.get(..len).ok_or("cut")?);
        assert!("Acme".starts_with(&vi.company), "{len}: {vi:?}");
        assert!("Rocket Sled".starts_with(&vi.product), "{len}: {vi:?}");
    }
    // A root that is not VS_VERSION_INFO, and a zero length.
    let mut foreign = blob.clone();
    *foreign.get_mut(3).ok_or("no key")? = u16::from(b'X');
    assert_eq!(version_info(&foreign), VersionInfo::default());
    let mut zero = blob;
    *zero.first_mut().ok_or("no wLength")? = 0;
    assert_eq!(version_info(&zero), VersionInfo::default());
    // A bare key and value with no tree around them.
    let mut runs: Vec<u16> = vec![0x40, 0x0f, 0x01];
    runs.extend("CompanyName\0\0Acme\0".encode_utf16());
    assert_eq!(version_info(&runs), VersionInfo::default());
    Ok(())
}

#[test]
fn a_broken_entry_ends_its_table_without_taking_a_neighbours_value() -> TestResult {
    // The wLength of the entry with this key, three words before the key.
    fn w_length<'a>(blob: &'a mut [u16], key: &str) -> Result<&'a mut u16, String> {
        let units = u16s(key);
        blob.windows(units.len())
            .position(|w| w == units)
            .and_then(|at| at.checked_sub(3))
            .and_then(|at| blob.get_mut(at))
            .ok_or_else(|| format!("no entry {key}"))
    }

    let blob = version_resource(&[(
        "040904b0",
        &[
            ("CompanyName", "Acme"),
            ("FileDescription", "Game"),
            ("ProductName", "Rocket Sled"),
        ],
    )]);
    let acme_only = VersionInfo {
        company: "Acme".into(),
        ..VersionInfo::default()
    };
    // A zero or too-small wLength in the middle of the table: the walk
    // stops there, so the entries before it still count.
    for length in [0, 2, 6] {
        let mut broken = blob.clone();
        *w_length(&mut broken, "FileDescription")? = length;
        assert_eq!(version_info(&broken), acme_only, "wLength {length}");
    }
    // An entry claiming more than its table holds is cut off at the
    // table's end, and nothing after it is read as a sibling.
    let mut oversized = blob;
    *w_length(&mut oversized, "CompanyName")? = 0xfffe;
    assert_eq!(version_info(&oversized), acme_only);
    // A value with wValueLength 0 and no NUL before the next entry ends
    // where its entry does.
    let mut unterminated = version_node("CompanyName", &u16s("Acme"), true, &[]);
    *unterminated.get_mut(1).ok_or("no wValueLength")? = 0;
    assert!(
        unterminated.len().is_multiple_of(2),
        "no padding before the next entry"
    );
    let table = version_node(
        "040904b0",
        &[],
        false,
        &siblings(&[unterminated, string_entry("ProductName", "Rocket Sled")]),
    );
    let info = version_node("StringFileInfo", &[], false, &table);
    let unterminated = version_node("VS_VERSION_INFO", &[0; 26], false, &info);
    assert_eq!(
        version_info(&unterminated),
        VersionInfo {
            company: "Acme".into(),
            product: "Rocket Sled".into(),
            ..VersionInfo::default()
        }
    );
    Ok(())
}
