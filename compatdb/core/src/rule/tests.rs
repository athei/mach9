use super::*;
use crate::Table;

#[test]
fn a_fingerprint_constraint_is_an_optional_extra_filter() {
    let table = Table {
        rules: vec![Rule {
            exe: "Launcher.exe".into(),
            company: Some("Rockstar Games".into()),
            product: Some("Rockstar Games Launcher".into()),
            dxgi: Some(Dxgi::Wined3d),
            ..Rule::default()
        }],
    };
    let rockstar = VersionInfo {
        company: "Rockstar Games".into(),
        product: "Rockstar Games Launcher".into(),
        original_filename: "Launcher.exe".into(),
    };
    // Matches by basename + fingerprint regardless of where it is installed.
    assert_eq!(
        table.resolve("Launcher.exe", &rockstar).dxgi,
        Some(Dxgi::Wined3d)
    );
    // A different program that happens to be called Launcher.exe: same
    // basename, no version resource, so the fingerprint fails and it is
    // left alone.
    assert!(
        table
            .resolve("launcher.exe", &VersionInfo::default())
            .is_empty()
    );
}

#[test]
fn a_bare_basename_rule_needs_no_fingerprint() {
    let table = Table {
        rules: vec![Rule {
            exe: "steamwebhelper.exe".into(),
            arguments: vec!["--in-process-gpu".into()],
            ..Rule::default()
        }],
    };
    let r = table.resolve("steamwebhelper.exe", &VersionInfo::default());
    assert_eq!(r.arguments, vec!["--in-process-gpu".to_string()]);
}

#[test]
fn specificity_orders_wildcard_then_exe_then_fingerprint() {
    let wildcard = Rule {
        exe: ANY_EXE.into(),
        ..Rule::default()
    };
    let wildcard_pinned = Rule {
        exe: ANY_EXE.into(),
        company: Some("Rockstar Games".into()),
        ..Rule::default()
    };
    let named = Rule {
        exe: "a.exe".into(),
        ..Rule::default()
    };
    let named_pinned = Rule {
        exe: "a.exe".into(),
        product: Some("A".into()),
        ..Rule::default()
    };
    assert!(wildcard.specificity() < wildcard_pinned.specificity());
    assert!(wildcard_pinned.specificity() < named.specificity());
    assert!(named.specificity() < named_pinned.specificity());
    assert!(named.is_unfingerprinted());
    assert!(!named_pinned.is_unfingerprinted());
    assert!(!wildcard.is_unfingerprinted());
}
