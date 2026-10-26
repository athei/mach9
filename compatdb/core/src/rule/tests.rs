use super::*;
use crate::{EnvName, Table};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn a_fingerprint_constraint_is_an_optional_extra_filter() {
    let table = Table {
        rules: vec![Rule {
            exe: Exe::literal("Launcher.exe"),
            company: Some(Fingerprint::literal("Rockstar Games")),
            product: Some(Fingerprint::literal("Rockstar Games Launcher")),
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
            exe: Exe::literal("steamwebhelper.exe"),
            arguments: vec![Argument::literal("--in-process-gpu")],
            ..Rule::default()
        }],
    };
    let r = table.resolve("steamwebhelper.exe", &VersionInfo::default());
    assert_eq!(r.arguments, ["--in-process-gpu"]);
}

#[test]
fn specificity_orders_wildcard_then_exe_then_fingerprint() {
    let wildcard = Rule {
        exe: Exe::Any,
        ..Rule::default()
    };
    let wildcard_pinned = Rule {
        exe: Exe::Any,
        company: Some(Fingerprint::literal("Rockstar Games")),
        ..Rule::default()
    };
    let named = Rule {
        exe: Exe::literal("a.exe"),
        ..Rule::default()
    };
    let named_pinned = Rule {
        exe: Exe::literal("a.exe"),
        product: Some(Fingerprint::literal("A")),
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
fn the_builder_sets_what_the_getters_read() -> TestResult {
    let rule = Rule::new(Exe::new("Game.exe")?)
        .with_company(Fingerprint::new("Acme")?)
        .with_product(Fingerprint::new("Game")?)
        .with_original_filename(Fingerprint::new("game.exe")?)
        .with_dxgi(Dxgi::Wined3d)
        .with_d3d9(Setting::Reset)
        .with_wined3d(Wined3d::Vulkan)
        .with_opengl(OpenGl::Cgl)
        .with_dpi_aware(false)
        .with_large_address_aware(true)
        .with_gcmouse(Setting::Reset)
        .with_force_nx(false)
        .with_dll_override(DllOverride::new("d3d12=")?)
        .with_argument(Argument::new("-a")?)
        .with_argument(Argument::new("-b")?)
        .with_arguments_override([Argument::new("-c")?])
        .with_env(EnvEntry::new(EnvName::new("A")?, "1")?);
    assert_eq!(rule.exe().as_str(), "Game.exe");
    assert_eq!(rule.company().map(Fingerprint::as_str), Some("Acme"));
    assert_eq!(rule.product().map(Fingerprint::as_str), Some("Game"));
    assert_eq!(
        rule.original_filename().map(Fingerprint::as_str),
        Some("game.exe")
    );
    assert_eq!(rule.dxgi(), Setting::Set(Dxgi::Wined3d));
    assert_eq!(rule.d3d9(), Setting::Reset);
    assert_eq!(rule.wined3d(), Setting::Set(Wined3d::Vulkan));
    assert_eq!(rule.opengl(), Setting::Set(OpenGl::Cgl));
    assert_eq!(rule.dpi_aware(), Setting::Set(false));
    assert_eq!(rule.large_address_aware(), Setting::Set(true));
    assert_eq!(rule.gcmouse(), Setting::Reset);
    assert_eq!(rule.force_nx(), Setting::Set(false));
    assert_eq!(rule.dll_overrides(), ["d3d12="]);
    assert_eq!(rule.arguments(), ["-a", "-b"]);
    assert_eq!(rule.arguments_override(), Some(&[Argument::new("-c")?][..]));
    assert_eq!(rule.env().len(), 1);
    assert_eq!(rule.source(), Source::BuiltIn);
    assert_eq!(rule.specificity(), Specificity::ExeWithFingerprint);
    assert_eq!(rule.matchers(), "Game.exe (Acme, Game, game.exe)");
    // An empty override clears, and is not the same as none.
    let cleared = Rule::new(Exe::Any).with_arguments_override([]);
    assert_eq!(cleared.arguments_override(), Some(&[][..]));
    assert_eq!(Rule::new(Exe::Any).arguments_override(), None);
    assert_eq!(Rule::default(), Rule::new(Exe::Any));
    Ok(())
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
