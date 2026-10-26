use super::*;
use crate::{Setting, Table};

#[test]
fn a_fingerprint_constraint_is_an_optional_extra_filter() {
    let table = Table {
        rules: vec![Rule {
            exe: "Launcher.exe".into(),
            company: Some("Rockstar Games".into()),
            product: Some("Rockstar Games Launcher".into()),
            dxgi: Setting::Set(Dxgi::Wined3d),
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
        table.resolve("Launcher.exe", &rockstar).dxgi(),
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

#[test]
fn a_setting_is_unset_a_value_or_a_reset() {
    assert_eq!(Setting::<bool>::default(), Setting::Unset);
    assert_eq!(Setting::from(Dxgi::Dxmt), Setting::Set(Dxgi::Dxmt));
    assert_eq!(Setting::Set(true).value(), Some(true));
    assert_eq!(Setting::<bool>::Reset.value(), None);
    assert!(Setting::<bool>::Unset.is_unset());
    assert!(!Setting::<bool>::Reset.is_unset());
    assert_eq!(Setting::Set(2).map(|n| n * 2), Setting::Set(4));
    assert_eq!(Setting::<u8>::Reset.map(|n| n * 2), Setting::Reset);
}

#[test]
fn a_rule_reads_and_writes_each_scalar_by_its_key() {
    let mut rule = Rule::default();
    let values = [
        (Scalar::Dxgi, ScalarValue::Dxgi(Dxgi::Wined3d)),
        (Scalar::D3d9, ScalarValue::D3d9(D3d9::Mtld3d)),
        (Scalar::Wined3d, ScalarValue::Wined3d(Wined3d::Vulkan)),
        (Scalar::OpenGl, ScalarValue::OpenGl(OpenGl::Cgl)),
        (Scalar::DpiAware, ScalarValue::Bool(false)),
        (Scalar::LargeAddressAware, ScalarValue::Bool(true)),
        (Scalar::Gcmouse, ScalarValue::Bool(false)),
        (Scalar::ForceNx, ScalarValue::Bool(true)),
    ];
    for (scalar, value) in values {
        assert_eq!(rule.scalar(scalar), Setting::Unset, "{scalar}");
        rule.put_scalar(scalar, Setting::Set(value));
        assert_eq!(rule.scalar(scalar), Setting::Set(value), "{scalar}");
        // A value of another setting changes nothing.
        let other = if matches!(value, ScalarValue::Bool(_)) {
            ScalarValue::Dxgi(Dxgi::Dxmt)
        } else {
            ScalarValue::Bool(true)
        };
        rule.put_scalar(scalar, Setting::Set(other));
        assert_eq!(rule.scalar(scalar), Setting::Set(value), "{scalar}");
    }
    assert_eq!(rule.dpi_aware, Setting::Set(false));
    rule.put_scalar(Scalar::DpiAware, Setting::Reset);
    assert_eq!(rule.dpi_aware, Setting::Reset);
    rule.put_scalar(Scalar::DpiAware, Setting::Unset);
    assert_eq!(rule.dpi_aware, Setting::Unset);
}
