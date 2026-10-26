use super::*;
use crate::{
    Exe, Fingerprint,
    runtime::{append_cmdline_each, merge_env},
};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// [`Table::parse_lenient`], with the diagnostics as `compatdb.so` logs
/// them.
fn parse(text: &str) -> (Table, Vec<String>) {
    let (table, diagnostics) = Table::parse_lenient(text);
    (table, diagnostics.iter().map(ToString::to_string).collect())
}

/// The built-in-looking `table` with `text` parsed and appended, as
/// `compatdb.so` builds its table; `text` must parse cleanly.
fn with_lines(table: &Table, text: &str) -> Table {
    let (parsed, diagnostics) = parse(text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut table = table.clone();
    table.append(parsed);
    table
}

/// The env entry that sets `name` to `value`, or removes it when `value`
/// is empty.
fn pair(name: &str, value: &str) -> EnvEntry {
    EnvEntry::literal(name, value)
}

/// The built-in-looking rules of the `arguments_override` tests: a `*`
/// rule and a rule for `game.exe`, each with one argument.
fn argument_rules() -> Table {
    Table::of_builtin(vec![
        Rule {
            exe: Exe::Any,
            arguments: vec![Argument::literal("-all")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            company: Some(Fingerprint::literal("Acme")),
            arguments: vec![Argument::literal("-game")],
            ..Rule::default()
        },
    ])
}

/// The version resource the `game.exe` rule of [`argument_rules`] pins.
fn acme() -> VersionInfo {
    VersionInfo {
        company: "Acme".into(),
        ..VersionInfo::default()
    }
}

#[test]
fn dpi_aware_resolves_to_the_most_specific_rule_and_unset_inherits() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            company: Some(Fingerprint::literal("Acme")),
            dpi_aware: Setting::Set(true),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            dpi_aware: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            dpi_aware: Setting::Set(true),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("quiet.exe"),
            arguments: vec![Argument::literal("-x")],
            ..Rule::default()
        },
    ]);
    let acme = VersionInfo {
        company: "Acme".into(),
        ..VersionInfo::default()
    };
    // Only the wildcard matches.
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.dpi_aware(), Some(true));
    assert!(!other.is_empty());
    // The basename rule beats the wildcard.
    assert_eq!(
        table
            .resolve("game.exe", &VersionInfo::default())
            .dpi_aware(),
        Some(false)
    );
    // The fingerprinted rule beats both.
    let pinned = table.resolve("game.exe", &acme);
    assert_eq!(pinned.dpi_aware(), Some(true));
    assert_eq!(
        pinned.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&Origin::BuiltInFor("game.exe (Acme)".into()))
    );
    // A more specific rule that leaves it unset inherits the wildcard's.
    let quiet = table.resolve("quiet.exe", &VersionInfo::default());
    assert_eq!(quiet.dpi_aware(), Some(true));
    assert_eq!(
        quiet.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&Origin::BuiltIn)
    );
    assert_eq!(quiet.arguments, ["-x"]);
    // With no rule setting it, the resolution leaves it to wine.
    assert_eq!(Table::default().resolve("x.exe", &acme).dpi_aware(), None);
}

#[test]
fn a_later_star_rule_changes_a_builtin_value_only_when_it_sets_the_key() {
    let base = Table::of_builtin(vec![Rule {
        exe: Exe::Any,
        dpi_aware: Setting::Set(true),
        large_address_aware: Setting::Set(true),
        gcmouse: Setting::Set(true),
        force_nx: Setting::Set(true),
        ..Rule::default()
    }]);
    // A rule that does not mention them leaves the values alone.
    let table = with_lines(&base, "exe=*;dll_overrides=a=");
    let r = table.resolve("x.exe", &VersionInfo::default());
    assert_eq!(r.dpi_aware(), Some(true));
    assert_eq!(r.large_address_aware(), Some(true));
    assert_eq!(r.gcmouse(), Some(true));
    assert_eq!(r.force_nx(), Some(true));
    assert_eq!(
        r.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&Origin::BuiltIn)
    );
    // One that sets a key replaces that value and no other.
    let switches = [
        Scalar::DpiAware,
        Scalar::LargeAddressAware,
        Scalar::Gcmouse,
        Scalar::ForceNx,
    ];
    for key in switches {
        let table = with_lines(&base, &format!("exe=*;{key}=false"));
        let r = table.resolve("x.exe", &VersionInfo::default());
        for k in switches {
            let state = r.scalar(k);
            if k == key {
                assert_eq!(state.value, Some(ScalarValue::Bool(false)), "{key}");
                assert_eq!(state.set_by, Some(Origin::Line(1)), "{key}");
            } else {
                assert_eq!(state.value, Some(ScalarValue::Bool(true)), "{key}: {k}");
                assert_eq!(state.set_by, Some(Origin::BuiltIn), "{key}: {k}");
            }
        }
    }
}

#[test]
fn large_address_aware_resolves_to_the_most_specific_rule() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            company: Some(Fingerprint::literal("Acme")),
            dpi_aware: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            large_address_aware: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            large_address_aware: Setting::Set(true),
            ..Rule::default()
        },
    ]);
    let acme = VersionInfo {
        company: "Acme".into(),
        ..VersionInfo::default()
    };
    // Only the wildcard matches.
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.large_address_aware(), Some(true));
    // The basename rule beats the wildcard.
    let named = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(named.large_address_aware(), Some(false));
    // A fingerprinted rule that leaves it unset inherits it from the next
    // most specific rule.
    let pinned = table.resolve("game.exe", &acme);
    assert_eq!(pinned.dpi_aware(), Some(false));
    assert_eq!(pinned.large_address_aware(), Some(false));
    // With no rule setting it, there is no opinion.
    let none = Table::default().resolve("x.exe", &acme);
    assert_eq!(none.large_address_aware(), None);
    assert!(none.is_empty());
    // It alone makes a resolution non-empty.
    let table = Table::of_builtin(vec![Rule {
        exe: Exe::Any,
        large_address_aware: Setting::Set(false),
        ..Rule::default()
    }]);
    assert!(!table.resolve("x.exe", &acme).is_empty());
}

#[test]
fn gcmouse_resolves_to_the_most_specific_rule() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            company: Some(Fingerprint::literal("Acme")),
            dpi_aware: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            gcmouse: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            gcmouse: Setting::Set(true),
            ..Rule::default()
        },
    ]);
    let acme = VersionInfo {
        company: "Acme".into(),
        ..VersionInfo::default()
    };
    // Only the wildcard matches.
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.gcmouse(), Some(true));
    // The basename rule beats the wildcard.
    let named = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(named.gcmouse(), Some(false));
    // A fingerprinted rule that leaves it unset inherits it from the next
    // most specific rule.
    let pinned = table.resolve("game.exe", &acme);
    assert_eq!(pinned.dpi_aware(), Some(false));
    assert_eq!(pinned.gcmouse(), Some(false));
    // With no rule setting it, there is no opinion.
    let none = Table::default().resolve("x.exe", &acme);
    assert_eq!(none.gcmouse(), None);
    assert!(none.is_empty());
    // It alone makes a resolution non-empty.
    let table = Table::of_builtin(vec![Rule {
        exe: Exe::Any,
        gcmouse: Setting::Set(false),
        ..Rule::default()
    }]);
    assert!(!table.resolve("x.exe", &acme).is_empty());
}

#[test]
fn force_nx_resolves_to_the_most_specific_rule() {
    let base = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            force_nx: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            force_nx: Setting::Set(true),
            ..Rule::default()
        },
    ]);
    let other = base.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.force_nx(), Some(true));
    assert_eq!(
        other.scalar(Scalar::ForceNx).set_by.as_ref(),
        Some(&Origin::BuiltIn)
    );
    let named = base.resolve("Game.exe", &VersionInfo::default());
    assert_eq!(named.force_nx(), Some(false));
    assert_eq!(
        named.scalar(Scalar::ForceNx).set_by.as_ref(),
        Some(&Origin::BuiltInFor("game.exe".into()))
    );
    let none = Table::default().resolve("x.exe", &VersionInfo::default());
    assert_eq!(none.force_nx(), None);
    assert!(none.is_empty());
    // A `*` rule from WINE_COMPATDB folds after every built-in rule, so
    // it beats the built-in rule for one program too.
    let table = with_lines(&base, "exe=*;force_nx=false\nexe=*;dll_overrides=a=");
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.force_nx(), Some(false));
    assert_eq!(
        other.scalar(Scalar::ForceNx).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    let table = with_lines(&base, "exe=*;force_nx=true");
    let named = table.resolve("Game.exe", &VersionInfo::default());
    assert_eq!(named.force_nx(), Some(true));
    assert_eq!(
        named.scalar(Scalar::ForceNx).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
}

#[test]
fn the_rules_of_wine_compatdb_fold_after_every_builtin_rule() {
    let base = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            company: Some(Fingerprint::literal("Acme")),
            dpi_aware: Setting::Set(false),
            gcmouse: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            dpi_aware: Setting::Set(true),
            gcmouse: Setting::Set(true),
            ..Rule::default()
        },
    ]);
    let acme = VersionInfo {
        company: "Acme".into(),
        ..VersionInfo::default()
    };
    // The least specific line beats the most specific built-in rule.
    let table = with_lines(&base, "exe=*;dpi_aware=true");
    let game = table.resolve("game.exe", &acme);
    assert_eq!(game.dpi_aware(), Some(true));
    assert_eq!(
        game.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    // The key it leaves alone keeps the built-in value.
    assert_eq!(game.gcmouse(), Some(false));
    // Among the lines, specificity still decides before line order.
    let table = with_lines(
        &base,
        "exe=game.exe;company=Acme;dpi_aware=false\nexe=game.exe;dpi_aware=true\nexe=*;dpi_aware=true",
    );
    let game = table.resolve("game.exe", &acme);
    assert_eq!(game.dpi_aware(), Some(false));
    assert_eq!(
        game.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
}

#[test]
fn default_resets_a_scalar_from_that_rule_on() -> TestResult {
    let base = Table::of_builtin(vec![Rule {
        exe: Exe::Any,
        dxgi: Setting::Set(Dxgi::Dxmt),
        wined3d: Setting::Set(Wined3d::Vulkan),
        dpi_aware: Setting::Set(true),
        ..Rule::default()
    }]);
    let table = with_lines(
        &base,
        "exe=*;dxgi=default;wined3d=default;dpi_aware=default",
    );
    let r = table.resolve("x.exe", &VersionInfo::default());
    assert_eq!((r.dxgi(), r.wined3d(), r.dpi_aware()), (None, None, None));
    for key in [Scalar::Dxgi, Scalar::Wined3d, Scalar::DpiAware] {
        assert_eq!(r.scalar(key).set_by, None, "{key}");
        assert_eq!(r.scalar(key).reset_by, Some(Origin::Line(1)), "{key}");
    }
    assert!(r.is_empty());
    // A rule folded later sets it again.
    let table = with_lines(&base, "exe=*;dpi_aware=default\nexe=x.exe;dpi_aware=false");
    let r = table.resolve("x.exe", &VersionInfo::default());
    assert_eq!(r.dpi_aware(), Some(false));
    assert_eq!(r.scalar(Scalar::DpiAware).reset_by.as_ref(), None);
    // Within one line the last field for a key wins, with a warning.
    let (parsed, diagnostics) = parse(
        "exe=a.exe;dpi_aware=default;dpi_aware=true\nexe=b.exe;gcmouse=false;gcmouse=default",
    );
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: dpi_aware given more than once, the last one applies",
            "WINE_COMPATDB line 2: gcmouse given more than once, the last one applies",
        ]
    );
    let a = parsed.rules.first().ok_or("no a")?;
    assert_eq!(a.dpi_aware, Setting::Set(true));
    let b = parsed.rules.get(1).ok_or("no b")?;
    assert_eq!(b.gcmouse, Setting::Reset);
    // Every scalar key takes it, and nothing else does: `default` is a word
    // of the format, read in any case like the booleans, while `env` and
    // `arguments` take it as text.
    for &scalar in Scalar::ALL {
        let (parsed, diagnostics) = parse(&format!("exe=a.exe;{scalar}=default"));
        assert!(diagnostics.is_empty(), "{scalar}: {diagnostics:?}");
        let rule = parsed.rules.first().ok_or("no rule")?;
        for &other in Scalar::ALL {
            let want = if other == scalar {
                Setting::Reset
            } else {
                Setting::Unset
            };
            assert_eq!(rule.scalar(other), want, "{scalar}: {other}");
        }
    }
    let (parsed, diagnostics) =
        parse("exe=a.exe;dpi_aware=Default;env=A=default;arguments=DEFAULT");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let a = parsed.rules.first().ok_or("no a")?;
    assert_eq!(a.dpi_aware, Setting::Reset);
    assert_eq!(a.env, [pair("A", "default")]);
    assert_eq!(a.arguments, ["DEFAULT"]);
    Ok(())
}

#[test]
fn a_default_dll_override_takes_its_modules_out_of_the_entries_before_it() {
    let base = Table::of_builtin(vec![
        Rule {
            exe: Exe::Any,
            dll_overrides: vec![
                DllOverride::literal("mscoree,mshtml="),
                DllOverride::literal("d3d12="),
            ],
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            dll_overrides: vec![DllOverride::literal("D3D12.DLL=n")],
            ..Rule::default()
        },
    ]);
    let table = with_lines(
        &base,
        "exe=*;dll_overrides=d3d12=default;dll_overrides=mshtml=b",
    );
    let game = table.resolve("game.exe", &VersionInfo::default());
    // Both entries for d3d12 go, whatever their case or suffix; the
    // other modules and later entries stay.
    assert_eq!(game.dll_overrides, ["mscoree,mshtml=", "mshtml=b"]);
    assert_eq!(game.dll_overrides_from, [Origin::BuiltIn, Origin::Line(1)]);
    assert_eq!(
        game.dll_overrides_reset,
        [DllOverrideReset {
            module: "d3d12".into(),
            by: Origin::Line(1),
            removed: vec![Origin::BuiltIn, Origin::BuiltInFor("game.exe".into())],
        }]
    );
    let entry = DllOverride::literal;
    assert_eq!(
        without_module(&entry("a,b="), "B.dll"),
        Without::Rest(entry("a="))
    );
    assert_eq!(
        without_module(&entry(" a , b =n,b"), "a"),
        Without::Rest(entry("b=n,b"))
    );
    assert_eq!(without_module(&entry("a="), "a"), Without::Emptied);
    assert_eq!(without_module(&entry("a="), "b"), Without::Absent);
}

#[test]
fn resolve_matches_case_insensitively_and_folds_in_order() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            dxgi: Setting::Set(Dxgi::Dxmt),
            dll_overrides: vec![DllOverride::literal("a=")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("GAME.EXE"),
            dxgi: Setting::Set(Dxgi::Wined3d),
            dll_overrides: vec![DllOverride::literal("b=")],
            ..Rule::default()
        },
    ]);
    let r = table.resolve("Game.exe", &VersionInfo::default());
    assert_eq!(r.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(r.dll_overrides, ["a=", "b="]);
}

#[test]
fn the_resolution_says_where_every_value_came_from() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            company: Some(Fingerprint::literal("Acme")),
            product: Some(Fingerprint::literal("Game")),
            dpi_aware: Setting::Set(false),
            arguments: vec![Argument::literal("-b")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            dxgi: Setting::Set(Dxgi::Dxmt),
            dpi_aware: Setting::Set(true),
            dll_overrides: vec![DllOverride::literal("a=")],
            arguments: vec![Argument::literal("-a")],
            env: vec![EnvEntry::literal("A", "1"), EnvEntry::literal("B", "2")],
            ..Rule::default()
        },
    ]);
    let acme = VersionInfo {
        company: "Acme".into(),
        product: "Game".into(),
        ..VersionInfo::default()
    };
    let r = table.resolve("game.exe", &acme);
    // A scalar says which rule set it last, the most specific one.
    let pinned = Origin::BuiltInFor("game.exe (Acme, Game)".into());
    assert_eq!(
        r.scalar(Scalar::Dxgi).set_by.as_ref(),
        Some(&Origin::BuiltIn)
    );
    assert_eq!(r.scalar(Scalar::DpiAware).set_by.as_ref(), Some(&pinned));
    assert_eq!(r.scalar(Scalar::D3d9).set_by.as_ref(), None);
    assert_eq!(pinned.label(), "built-in for game.exe (Acme, Game)");
    assert_eq!(Origin::BuiltIn.label(), "built-in");
    assert_eq!(Origin::Line(3).label(), "WINE_COMPATDB line 3");
    // Lists say it for each entry.
    assert_eq!(r.dll_overrides_from, [Origin::BuiltIn]);
    assert_eq!(r.arguments, ["-a", "-b"]);
    assert_eq!(r.arguments_from, [Origin::BuiltIn, pinned]);
    // Both entries of a built-in `*` rule are defaults, which a variable
    // the process has keeps out.
    let steps = r.env_steps(|name| name == "A");
    assert_eq!(
        steps
            .iter()
            .map(|s| (s.entry.name().as_str(), &s.origin, s.applied))
            .collect::<Vec<_>>(),
        [
            ("A", &Origin::BuiltIn, false),
            ("B", &Origin::BuiltIn, true)
        ]
    );
}

#[test]
fn only_the_env_entries_of_a_builtin_star_rule_are_defaults() {
    let base = Table::of_builtin(vec![
        Rule {
            exe: Exe::Any,
            env: vec![EnvEntry::literal("A", "1")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            company: Some(Fingerprint::literal("Acme")),
            env: vec![EnvEntry::literal("B", "1")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            env: vec![EnvEntry::literal("C", "1")],
            ..Rule::default()
        },
    ]);
    let table = with_lines(&base, "exe=*;env=D=1");
    let defaults: Vec<bool> = table
        .entries()
        .map(|(source, rule)| Origin::of(source, rule).env_is_default())
        .collect();
    assert_eq!(defaults, [true, false, false, false]);
    let acme = VersionInfo {
        company: "Acme".into(),
        ..VersionInfo::default()
    };
    let r = table.resolve("game.exe", &acme);
    let applied: Vec<(&str, bool)> = r
        .env_steps(|_| true)
        .iter()
        .map(|s| {
            let name = if *s.entry.name() == "A" { "A" } else { "other" };
            (name, s.applied)
        })
        .collect();
    assert_eq!(
        applied,
        [
            ("A", false),
            ("other", true),
            ("other", true),
            ("other", true)
        ]
    );
}

#[test]
fn a_rule_from_wine_compatdb_is_named_by_its_line() -> TestResult {
    let base = Table::of_builtin(vec![Rule {
        exe: Exe::Any,
        dpi_aware: Setting::Set(true),
        gcmouse: Setting::Set(true),
        dll_overrides: vec![DllOverride::literal("a=")],
        env: vec![EnvEntry::literal("A", "1")],
        ..Rule::default()
    }]);
    let table = with_lines(&base, "v=7\nexe=*;dpi_aware=false;dll_overrides=b=;env=A=2");
    let r = table.resolve("x.exe", &VersionInfo::default());
    // The scalar the line set says so; the one it left does not.
    let dpi = r
        .scalar(Scalar::DpiAware)
        .set_by
        .as_ref()
        .ok_or("no dpi_aware")?;
    assert_eq!(dpi.label(), "WINE_COMPATDB line 2");
    assert_eq!(
        r.scalar(Scalar::Gcmouse).set_by.as_ref(),
        Some(&Origin::BuiltIn)
    );
    assert_eq!(r.dll_overrides_from, [Origin::BuiltIn, Origin::Line(2)]);
    // The line's env entry replaces, and the built-in's own entry, a
    // default, says which line replaced it.
    let steps = r.env_steps(|_| false);
    assert_eq!(
        steps
            .iter()
            .map(|s| (s.entry.value(), &s.origin, s.replaced_by.clone()))
            .collect::<Vec<_>>(),
        [
            (Some("1"), &Origin::BuiltIn, Some(Origin::Line(2))),
            (Some("2"), &Origin::Line(2), None)
        ]
    );
    // Left alone, the default is replaced all the same.
    let steps = r.env_steps(|_| true);
    assert_eq!(
        steps.iter().map(|s| s.applied).collect::<Vec<_>>(),
        [false, true]
    );
    Ok(())
}

#[test]
fn a_removal_names_the_value_it_undoes() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            env: vec![EnvEntry::literal("A", ""), EnvEntry::literal("B", "")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            env: vec![EnvEntry::literal("A", "1")],
            ..Rule::default()
        },
    ]);
    let r = table.resolve("game.exe", &VersionInfo::default());
    let steps = r.env_steps(|_| false);
    let removes = |name: &str| {
        steps
            .iter()
            .find(|s| *s.entry.name() == name && s.entry.value().is_none())
            .and_then(|s| s.removes.clone())
    };
    // A removes what the default set; B had no earlier entry.
    assert_eq!(removes("A"), Some(Origin::BuiltIn));
    assert_eq!(removes("B"), None);
    // Left alone, the default set nothing for the removal to undo.
    let steps = r.env_steps(|name| name == "A");
    assert_eq!(
        steps
            .iter()
            .find(|s| *s.entry.name() == "A" && s.entry.value().is_none())
            .and_then(|s| s.removes.clone()),
        None
    );
}

#[test]
fn a_rule_for_the_same_exe_does_not_take_another_rules_fingerprint() {
    // Each rule stands on its own matchers. A rule from WINE_COMPATDB
    // for a common basename does not inherit the fingerprint of a
    // built-in rule for it, and the built-in rule is unchanged.
    let base = Table::of_builtin(vec![Rule {
        exe: Exe::literal("Launcher.exe"),
        company: Some(Fingerprint::literal("Rockstar Games")),
        dxgi: Setting::Set(Dxgi::Wined3d),
        ..Rule::default()
    }]);
    let table = with_lines(&base, "exe=Launcher.exe;dxgi=dxmt;dll_overrides=b=");
    assert_eq!(table.rules.first(), base.rules.first());
    let rockstar = VersionInfo {
        company: "Rockstar Games".into(),
        ..VersionInfo::default()
    };
    // Another program of that name gets the new rule alone.
    let other = table.resolve("Launcher.exe", &VersionInfo::default());
    assert_eq!(other.dxgi(), Some(Dxgi::Dxmt));
    assert_eq!(other.dll_overrides, ["b="]);
    // The Rockstar launcher gets both. The line folds after every
    // built-in rule, so it takes the scalar, and the lists accumulate.
    let launcher = table.resolve("Launcher.exe", &rockstar);
    assert_eq!(launcher.dxgi(), Some(Dxgi::Dxmt));
    assert_eq!(
        launcher.scalar(Scalar::Dxgi).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    assert_eq!(launcher.dll_overrides, ["b="]);
    // A fingerprint keeps a line to the Rockstar launcher alone.
    let table = with_lines(&base, "exe=Launcher.exe;company=Rockstar;dxgi=dxmt");
    let launcher = table.resolve("Launcher.exe", &rockstar);
    assert_eq!(launcher.dxgi(), Some(Dxgi::Dxmt));
    assert!(
        table
            .resolve("Launcher.exe", &VersionInfo::default())
            .is_empty()
    );
}

#[test]
fn arguments_override_replaces_what_the_rules_before_it_accumulated() {
    // A rule for the game, folded after both built-in rules, replaces
    // both of their arguments with its own.
    let table = with_lines(
        &argument_rules(),
        "exe=game.exe;company=Acme;arguments_override=-mine",
    );
    let r = table.resolve("game.exe", &acme());
    assert_eq!(r.arguments, ["-mine"]);
    assert_eq!(r.arguments_from, [Origin::Line(1)]);
    let builtin_game = Origin::BuiltInFor("game.exe (Acme)".into());
    assert_eq!(
        r.arguments_dropped,
        [ArgumentsDropped {
            by: Origin::Line(1),
            entries: vec![
                (Argument::literal("-all"), Origin::BuiltIn),
                (Argument::literal("-game"), builtin_game),
            ],
        }]
    );
    // Another process keeps the `*` rule's argument.
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.arguments, ["-all"]);
    assert_eq!(other.arguments_dropped, []);
}

#[test]
fn an_empty_arguments_override_clears_the_list() {
    let table = with_lines(&argument_rules(), "exe=*;arguments_override=");
    // It folds after every built-in rule, so it drops the arguments of
    // both, the rule for the game included.
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.arguments, [] as [&str; 0]);
    assert_eq!(
        other.arguments_dropped,
        [ArgumentsDropped {
            by: Origin::Line(1),
            entries: vec![(Argument::literal("-all"), Origin::BuiltIn)],
        }]
    );
    let game = table.resolve("game.exe", &acme());
    assert_eq!(game.arguments, [] as [&str; 0]);
    // An override that finds nothing to drop is not reported.
    let table = with_lines(&Table::default(), "exe=*;arguments_override=");
    let r = table.resolve("x.exe", &VersionInfo::default());
    assert_eq!(r.arguments_dropped, []);
    assert!(r.is_empty());
}

#[test]
fn arguments_after_an_override_still_append() {
    // The rule's own `arguments` come after its override, and so do
    // those of the rules folded after it: a later line of the same tier
    // and a more specific line.
    let table = with_lines(
        &argument_rules(),
        "exe=game.exe;arguments=-specific\nexe=*;arguments_override=-new;arguments=-own\n\
         exe=*;arguments=-later",
    );
    let game = table.resolve("game.exe", &acme());
    assert_eq!(game.arguments, ["-new", "-own", "-later", "-specific"]);
    assert_eq!(
        game.arguments_from,
        [
            Origin::Line(2),
            Origin::Line(2),
            Origin::Line(3),
            Origin::Line(1)
        ]
    );
    // A later override in the same tier drops the earlier one's
    // arguments too, and each drop is reported on its own.
    let table = with_lines(
        &argument_rules(),
        "exe=*;arguments_override=-first\nexe=*;arguments_override=-second",
    );
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.arguments, ["-second"]);
    assert_eq!(
        other.arguments_dropped,
        [
            ArgumentsDropped {
                by: Origin::Line(1),
                entries: vec![(Argument::literal("-all"), Origin::BuiltIn)],
            },
            ArgumentsDropped {
                by: Origin::Line(2),
                entries: vec![(Argument::literal("-first"), Origin::Line(1))],
            },
        ]
    );
}

#[test]
fn a_less_specific_override_cannot_drop_a_more_specific_lines_arguments() {
    // Within WINE_COMPATDB tiers still fold least specific first, so a
    // `*` override runs before a line for the game and leaves its
    // argument, whatever the line order.
    for text in [
        "exe=game.exe;arguments=-mine\nexe=*;arguments_override=-x",
        "exe=*;arguments_override=-x\nexe=game.exe;arguments=-mine",
    ] {
        let table = with_lines(&argument_rules(), text);
        let game = table.resolve("game.exe", &acme());
        assert_eq!(game.arguments, ["-x", "-mine"], "{text}");
    }
    // A basename line without a fingerprint still drops the built-in
    // rule's argument, since every line folds after every built-in rule.
    let table = with_lines(&argument_rules(), "exe=game.exe;arguments_override=");
    let game = table.resolve("game.exe", &acme());
    assert_eq!(game.arguments, [] as [&str; 0]);
}

#[test]
fn an_overridden_list_is_appended_unless_already_present() -> TestResult {
    let table = with_lines(
        &argument_rules(),
        "exe=game.exe;company=Acme;arguments_override=-a;arguments_override=-b",
    );
    let r = table.resolve("game.exe", &acme());
    let (out, appended) = append_cmdline_each(&u16s("game.exe -b"), &r.arguments);
    assert_eq!(
        String::from_utf16_lossy(&out.ok_or("unchanged")?),
        "game.exe -b -a"
    );
    assert_eq!(appended, [true, false]);
    Ok(())
}

#[test]
fn a_wildcard_rule_is_the_least_specific_and_a_process_rule_beats_it() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::Any,
            dxgi: Setting::Set(Dxgi::Dxmt),
            dll_overrides: vec![DllOverride::literal("global=")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("Launcher.exe"),
            dxgi: Setting::Set(Dxgi::Wined3d),
            dll_overrides: vec![DllOverride::literal("specific=")],
            ..Rule::default()
        },
    ]);
    // The wildcard alone reaches an unrelated process.
    let other = table.resolve("Other.exe", &VersionInfo::default());
    assert_eq!(other.dxgi(), Some(Dxgi::Dxmt));
    assert_eq!(other.dll_overrides, ["global="]);
    // Both match Launcher.exe; the more specific rule takes the scalar and
    // the lists accumulate least-specific-first.
    let launcher = table.resolve("launcher.exe", &VersionInfo::default());
    assert_eq!(launcher.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(
        launcher.scalar(Scalar::Dxgi).set_by.as_ref(),
        Some(&Origin::BuiltInFor("Launcher.exe".into()))
    );
    assert_eq!(launcher.dll_overrides, ["global=", "specific="]);
}

#[test]
fn a_process_rule_wins_however_the_rules_are_ordered() {
    // Table order must not decide a scalar; specificity does. Same two
    // rules as above with the specific one declared first.
    let specific = Rule {
        exe: Exe::literal("Launcher.exe"),
        dxgi: Setting::Set(Dxgi::Wined3d),
        ..Rule::default()
    };
    let wildcard = Rule {
        exe: Exe::Any,
        dxgi: Setting::Set(Dxgi::Dxmt),
        ..Rule::default()
    };
    let table = Table::of_builtin(vec![specific, wildcard]);
    assert_eq!(
        table
            .resolve("Launcher.exe", &VersionInfo::default())
            .dxgi(),
        Some(Dxgi::Wined3d)
    );
}

#[test]
fn two_rules_for_one_basename_resolve_independently() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("Launcher.exe"),
            company: Some(Fingerprint::literal("Rockstar Games")),
            dxgi: Setting::Set(Dxgi::Wined3d),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("Launcher.exe"),
            company: Some(Fingerprint::literal("Acme")),
            dxgi: Setting::Set(Dxgi::Dxmt),
            ..Rule::default()
        },
    ]);
    let rockstar = VersionInfo {
        company: "Rockstar Games".into(),
        ..VersionInfo::default()
    };
    let acme = VersionInfo {
        company: "Acme Inc".into(),
        ..VersionInfo::default()
    };
    let launcher = table.resolve("Launcher.exe", &rockstar);
    assert_eq!(launcher.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(
        launcher.scalar(Scalar::Dxgi).set_by.as_ref(),
        Some(&Origin::BuiltInFor("Launcher.exe (Rockstar Games)".into()))
    );
    assert_eq!(
        table.resolve("Launcher.exe", &acme).dxgi(),
        Some(Dxgi::Dxmt)
    );
    // A third program of that name matches neither.
    assert!(
        table
            .resolve("Launcher.exe", &VersionInfo::default())
            .is_empty()
    );
}

#[test]
fn the_two_implementations_resolve_independently() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::Any,
            dxgi: Setting::Set(Dxgi::Wined3d),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            d3d9: Setting::Set(D3d9::Wined3d),
            ..Rule::default()
        },
    ]);
    let r = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(r.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(r.d3d9(), Some(D3d9::Wined3d));
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(other.d3d9(), None);
}

#[test]
fn wined3d_resolves_to_the_most_specific_rule() {
    let base = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            wined3d: Setting::Set(Wined3d::Gl),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            wined3d: Setting::Set(Wined3d::Vulkan),
            ..Rule::default()
        },
    ]);
    let none = VersionInfo::default();
    assert_eq!(base.resolve("game.exe", &none).wined3d(), Some(Wined3d::Gl));
    assert_eq!(
        base.resolve("other.exe", &none).wined3d(),
        Some(Wined3d::Vulkan)
    );
    // A later rule of the same tier changes the setting for the game
    // and leaves other processes alone.
    let table = with_lines(&base, "exe=game.exe;wined3d=vulkan");
    let game = table.resolve("game.exe", &none);
    assert_eq!(game.wined3d(), Some(Wined3d::Vulkan));
    assert_eq!(
        game.scalar(Scalar::Wined3d).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    assert_eq!(
        table
            .resolve("other.exe", &none)
            .scalar(Scalar::Wined3d)
            .set_by
            .as_ref(),
        Some(&Origin::BuiltIn)
    );
}

#[test]
fn opengl_resolves_to_the_most_specific_rule() {
    let base = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("apple.exe"),
            opengl: Setting::Set(OpenGl::Cgl),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            opengl: Setting::Set(OpenGl::Egl),
            ..Rule::default()
        },
    ]);
    let none = VersionInfo::default();
    let apple = base.resolve("Apple.exe", &none);
    assert_eq!(apple.opengl(), Some(OpenGl::Cgl));
    assert_eq!(
        apple.scalar(Scalar::OpenGl).set_by.as_ref(),
        Some(&Origin::BuiltInFor("apple.exe".into()))
    );
    // It is a setting of the process, not an env entry.
    assert_eq!(apple.env, []);
    let other = base.resolve("other.exe", &none);
    assert_eq!(other.opengl(), Some(OpenGl::Egl));
    assert!(Table::default().resolve("x.exe", &none).opengl().is_none());
    // A `*` rule from WINE_COMPATDB changes it for every process, the
    // one a built-in rule names included.
    let table = with_lines(&base, "exe=*;opengl=cgl");
    let other = table.resolve("other.exe", &none);
    assert_eq!(other.opengl(), Some(OpenGl::Cgl));
    assert_eq!(
        other.scalar(Scalar::OpenGl).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    let table = with_lines(&base, "exe=*;opengl=egl");
    assert_eq!(
        table.resolve("apple.exe", &none).opengl(),
        Some(OpenGl::Egl)
    );
}

/// A resolution with the env entries `env` in which, when `wined3d` is
/// set, line 1 set that renderer.
fn with_renderer(wined3d: Option<Wined3d>, env: Vec<EnvEntry>) -> Resolution {
    let mut r = Resolution {
        env,
        ..Resolution::default()
    };
    if let Some(wined3d) = wined3d {
        r.fold_scalar(
            Scalar::Wined3d,
            Setting::Set(ScalarValue::Wined3d(wined3d)),
            &Origin::Line(1),
        );
    }
    r
}

#[test]
fn a_set_renderer_is_always_named_and_unset_sets_nothing() {
    let resolved = |wined3d| with_renderer(wined3d, vec![pair("A", "1")]);
    // Each renderer a rule sets goes after the rules' env, OpenGL as
    // explicitly as Vulkan, so a child never keeps its parent's Vulkan.
    for (wined3d, value) in [
        (Wined3d::Vulkan, "renderer=vulkan"),
        (Wined3d::Gl, "renderer=gl"),
    ] {
        let mut r = resolved(Some(wined3d));
        r.fold_derived_env();
        assert_eq!(r.env, [pair("A", "1"), pair("WINE_D3D_CONFIG", value)]);
        // A second call adds nothing.
        r.fold_derived_env();
        assert_eq!(r.env.len(), 2);
    }
    // No rule sets it: nothing changes.
    let mut r = resolved(None);
    r.fold_derived_env();
    assert_eq!(r.env, [pair("A", "1")]);
}

#[test]
fn the_setting_decides_the_renderer_and_keeps_the_other_options_of_an_env_rule() -> TestResult {
    // A rule's own WINE_D3D_CONFIG keeps its other options, loses its
    // renderer, and the setting's entry is the value the block ends with.
    for (wined3d, renderer) in [(Wined3d::Vulkan, "vulkan"), (Wined3d::Gl, "gl")] {
        let mut r = with_renderer(
            Some(wined3d),
            vec![
                pair("wine_d3d_config", "csmt=0"),
                pair(
                    "WINE_D3D_CONFIG",
                    "renderer=no3d,csmt=0x1;MaxVersionGL=0x40001",
                ),
            ],
        );
        r.fold_derived_env();
        let want = format!("renderer={renderer},csmt=0x1,MaxVersionGL=0x40001");
        assert_eq!(r.env.last(), Some(&pair("WINE_D3D_CONFIG", &want)));
        let block: Vec<u16> = "WINE_D3D_CONFIG=shader_backend=glsl\0OTHER=x\0\0"
            .encode_utf16()
            .collect();
        let merged = merge_env(&block, &r.env).ok_or("unchanged")?;
        assert_eq!(
            String::from_utf16_lossy(&merged),
            format!("WINE_D3D_CONFIG={want}\0OTHER=x\0\0")
        );
    }
    // A rule that removes the variable leaves just the renderer.
    let mut r = with_renderer(Some(Wined3d::Vulkan), vec![pair("WINE_D3D_CONFIG", "")]);
    r.fold_derived_env();
    assert_eq!(
        r.env.last(),
        Some(&pair("WINE_D3D_CONFIG", "renderer=vulkan"))
    );
    // Without the setting a rule's WINE_D3D_CONFIG applies as it is,
    // renderer and all.
    let mut r = with_renderer(
        None,
        vec![pair("WINE_D3D_CONFIG", "renderer=vulkan,csmt=0")],
    );
    r.fold_derived_env();
    assert_eq!(r.env, [pair("WINE_D3D_CONFIG", "renderer=vulkan,csmt=0")]);
    assert_eq!(r.d3d_config(), Some("renderer=vulkan,csmt=0"));
    Ok(())
}

#[test]
fn metal_hud_resolves_to_the_most_specific_rule_and_becomes_env_entries() {
    let table = with_lines(
        &Table::default(),
        "exe=game.exe;metal_hud=true;env=A=1;wined3d=vulkan\nexe=*;metal_hud=false\n\
         exe=tool.exe;metal_hud=default",
    );
    let none = VersionInfo::default();
    let mut game = table.resolve("Game.exe", &none);
    assert_eq!(game.metal_hud(), Some(true));
    game.fold_derived_env();
    // Where the rule that set it folded, before its own env; wined3d's
    // after every rule's. Each from the rule that set the setting, none a
    // default.
    let mut want = metal_hud_env(true);
    want.extend([pair("A", "1"), pair("WINE_D3D_CONFIG", "renderer=vulkan")]);
    assert_eq!(game.env, want);
    assert!(
        game.env_from
            .iter()
            .all(|from| from.origin == Origin::Line(1) && !from.default)
    );
    // The `*` rule removes the variables everywhere else.
    let mut other = table.resolve("other.exe", &none);
    other.fold_derived_env();
    assert_eq!(other.metal_hud(), Some(false));
    assert_eq!(other.env, metal_hud_env(false));
    assert!(
        other
            .env_from
            .iter()
            .all(|from| from.origin == Origin::Line(2))
    );
    let steps = other.env_steps(|name| name == "MTL_HUD_ENABLED");
    assert!(steps.iter().all(|step| step.applied));
    // `default` hands it back to the environment: no entry at all.
    let mut tool = table.resolve("tool.exe", &none);
    tool.fold_derived_env();
    assert_eq!(tool.metal_hud(), None);
    assert_eq!(
        tool.scalar(Scalar::MetalHud).reset_by,
        Some(Origin::Line(3))
    );
    assert_eq!(tool.env, []);
}

#[test]
fn an_env_entry_of_the_rule_that_sets_metal_hud_or_a_later_rule_wins() {
    let table = with_lines(
        &Table::default(),
        "exe=*;metal_hud=true;env=A=1\n\
         exe=game.exe;env=MTL_HUD_ALIGNMENT=4\n\
         exe=tool.exe;metal_hud=true;env=MTL_HUD_ENABLED=0",
    );
    let none = VersionInfo::default();
    let mut game = table.resolve("game.exe", &none);
    game.fold_derived_env();
    let mut want = metal_hud_env(true);
    want.extend([pair("A", "1"), pair("MTL_HUD_ALIGNMENT", "4")]);
    assert_eq!(game.env, want);
    let steps = game.env_steps(|_| false);
    let alignment = steps
        .iter()
        .find(|step| step.entry == pair("MTL_HUD_ALIGNMENT", "28"));
    assert_eq!(
        alignment.and_then(|step| step.replaced_by.clone()),
        Some(Origin::Line(2))
    );
    // The rule's own entry comes after the setting's.
    let mut tool = table.resolve("tool.exe", &none);
    tool.fold_derived_env();
    let mut want = vec![pair("A", "1")];
    want.extend(metal_hud_env(true));
    want.push(pair("MTL_HUD_ENABLED", "0"));
    assert_eq!(tool.env, want);
    let origins: Vec<Origin> = tool.env_from.iter().map(|f| f.origin.clone()).collect();
    assert_eq!(
        origins,
        [
            Origin::Line(1),
            Origin::Line(3),
            Origin::Line(3),
            Origin::Line(3),
            Origin::Line(3)
        ]
    );
}

#[test]
fn debug_channels_accumulate_in_fold_order_with_their_origin() -> TestResult {
    let only_channels = Table::of_builtin(vec![
        Rule::new(Exe::Any).with_debug_channel(DebugChannel::new("+tid")?),
    ]);
    let table = with_lines(
        &only_channels,
        "exe=game.exe;debug_channels=+relay;debug_channels=-heap\nexe=*;debug_channels=+seh\n\
         exe=other.exe;debug_channels=+x",
    );
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.debug_channels, ["+tid", "+seh", "+relay", "-heap"]);
    assert_eq!(
        game.debug_channels_from,
        [
            Origin::BuiltIn,
            Origin::Line(2),
            Origin::Line(1),
            Origin::Line(1)
        ]
    );
    // A resolution with channels alone is not empty: the log has them.
    assert!(
        !only_channels
            .resolve("x.exe", &VersionInfo::default())
            .is_empty()
    );
    Ok(())
}
