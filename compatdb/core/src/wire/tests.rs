use super::*;
use crate::{D3d9, Dxgi, KeyKind, OpenGl, Origin, VersionInfo, Wined3d};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// `table` as [`Table::parse`] reads it back from its env value: each
/// rule with its line as its source.
fn as_sent(table: &Table) -> Table {
    Table {
        rules: table
            .rules
            .iter()
            .enumerate()
            .map(|(i, rule)| Rule {
                source: Source::Line(i.saturating_add(1)),
                ..rule.clone()
            })
            .collect(),
    }
}

#[test]
fn encode_decode_round_trips_specials() {
    for raw in [
        "plain",
        "a;b",
        "a%b",
        "a=b,c d",
        "line\nfeed\r",
        "tab\there",
        "café ünïcode",
        "",
    ] {
        assert_eq!(decode(&encode(raw)), raw, "round trip of {raw:?}");
    }
}

#[test]
fn encode_escapes_only_the_dangerous_characters() {
    assert_eq!(encode("d3d12,vulkan-1="), "d3d12,vulkan-1=");
    assert_eq!(encode("a;b"), "a%3Bb");
    assert_eq!(encode("100%"), "100%25");
}

#[test]
fn decode_is_lenient_about_bad_escapes() {
    assert_eq!(decode("50%"), "50%");
    assert_eq!(decode("%zz"), "%zz");
    assert_eq!(decode("%2"), "%2");
}

#[test]
fn table_round_trips_through_the_env_value() {
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::literal("Launcher.exe"),
                company: Some(Fingerprint::literal("Rockstar Games")),
                product: Some(Fingerprint::literal("Rockstar Games Launcher")),
                dxgi: Setting::Set(Dxgi::Wined3d),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("SocialClubHelper.exe"),
                arguments: vec![Argument::literal("--in-process-gpu --disable-gpu")],
                original_filename: Some(Fingerprint::literal("SocialClubHelper.exe")),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("WoW.exe"),
                d3d9: Setting::Set(D3d9::Wined3d),
                dll_overrides: vec![DllOverride::literal("d3d12,vulkan-1=")],
                env: vec![EnvEntry::literal("DXMT_CONFIG", "d3d11.maxFrameRate=60")],
                ..Rule::default()
            },
        ],
    };
    let (parsed, diagnostics) = Table::parse(&table.to_env_value());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
}

#[test]
fn env_field_survives_an_equals_in_the_value() {
    let table = Table {
        rules: vec![Rule {
            exe: Exe::literal("x.exe"),
            env: vec![EnvEntry::literal("A", "b=c;d")],
            ..Rule::default()
        }],
    };
    let (parsed, _) = Table::parse(&table.to_env_value());
    assert_eq!(parsed, as_sent(&table));
}

#[test]
fn a_legacy_header_on_the_first_line_is_skipped() {
    // What a launcher written for the versioned format sends: the same
    // records mean the same with any version header in front, or none,
    // and the header counts as a line.
    let records = "exe=Launcher.exe;dxgi=dxmt\n\
                   exe=WoW.exe;company=Blizzard;d3d9=wined3d;wined3d=vulkan;env=A=1";
    let (bare, diagnostics) = Table::parse(records);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(bare.rules.len(), 2);
    for header in ["v=5", "v=6", "v=7", "v=10"] {
        let (table, diagnostics) = Table::parse(&format!("{header}\n{records}"));
        assert!(diagnostics.is_empty(), "{header}: {diagnostics:?}");
        let sources: Vec<Source> = table.rules.iter().map(|r| r.source).collect();
        assert_eq!(sources, [Source::Line(2), Source::Line(3)], "{header}");
        let mut shifted = bare.clone();
        for rule in &mut shifted.rules {
            if let Source::Line(n) = rule.source {
                rule.source = Source::Line(n.saturating_add(1));
            }
        }
        assert_eq!(table, shifted, "{header}");
    }
}

#[test]
fn only_a_first_line_of_v_and_digits_is_a_legacy_header() {
    // The `v` is a key, so its case does not matter.
    for header in ["v=6", "v=0", "v=123", "V=6"] {
        assert!(is_legacy_header(header), "{header}");
    }
    for other in ["v=", "v=6a", "v=-1", " v=6", "v=6;exe=x", "exe=x;v=6"] {
        assert!(!is_legacy_header(other), "{other}");
    }
    // Anything else on the first line is a record: one with an exe is a
    // rule, and one without, a legacy header that looks almost right
    // included, is ignored with a diagnostic.
    let (table, diagnostics) = Table::parse("exe=x.exe\nexe=y.exe");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(table.rules.len(), 2);
    let (table, diagnostics) = Table::parse("v=6a\nexe=x.exe");
    assert_eq!(table.rules.len(), 1);
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: ignoring unknown key \"v\"",
            "WINE_COMPATDB line 1: ignoring the line, it has no exe"
        ]
    );
    // Only the first line can be one; a later one is a line without an
    // exe.
    let (table, diagnostics) = Table::parse("exe=x.exe\nv=6");
    assert_eq!(table.rules.len(), 1);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
}

#[test]
fn the_env_value_is_the_records_alone() {
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::literal("game.exe"),
                dxgi: Setting::Set(Dxgi::Wined3d),
                wined3d: Setting::Set(Wined3d::Vulkan),
                env: vec![EnvEntry::literal("A", "1")],
                ..Rule::default()
            },
            // Neither the source nor the debug channels travel.
            Rule {
                exe: Exe::Any,
                gcmouse: Setting::Set(false),
                source: Source::Line(7),
                ..Rule::default()
            },
        ],
    };
    let value = table.to_env_value();
    assert_eq!(
        value,
        "exe=game.exe;dxgi=wined3d;wined3d=vulkan;env=A=1\n\
         exe=*;gcmouse=false"
    );
    let (parsed, diagnostics) = Table::parse(&value);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed.rules.first(), as_sent(&table).rules.first());
    assert_eq!(parsed.to_env_value(), value);
    assert_eq!(Table::default().to_env_value(), "");
    assert_eq!(Table::parse(""), (Table::default(), Vec::new()));
}

#[test]
fn parse_numbers_the_lines_and_counts_blank_ones() {
    let (table, diagnostics) = Table::parse("exe=a.exe\n\nexe=b.exe\n  \nexe=c.exe");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let sources: Vec<Source> = table.rules.iter().map(|r| r.source).collect();
    assert_eq!(sources, [Source::Line(1), Source::Line(3), Source::Line(5)]);
}

#[test]
fn parse_ignores_a_line_without_an_exe_and_keeps_the_rest() {
    // What a launcher wrote to change a built-in rule by name in the old
    // format: it has no exe, so it matches no process.
    for (line, said) in [
        ("dxgi=dxmt", vec![]),
        ("exe=", vec![]),
        (
            "name=rockstar-launcher;dxgi=dxmt",
            vec!["WINE_COMPATDB line 1: ignoring unknown key \"name\""],
        ),
        (
            "name=dpi-aware;enabled=false",
            vec![
                "WINE_COMPATDB line 1: ignoring unknown key \"name\"",
                "WINE_COMPATDB line 1: ignoring unknown key \"enabled\"",
            ],
        ),
    ] {
        let (table, diagnostics) = Table::parse(&format!("{line}\nexe=ok.exe"));
        assert_eq!(table.rules.len(), 1, "{line}");
        assert_eq!(table.rules.first().map(|r| r.exe.as_str()), Some("ok.exe"));
        let mut want: Vec<String> = said.into_iter().map(String::from).collect();
        want.push("WINE_COMPATDB line 1: ignoring the line, it has no exe".into());
        assert_eq!(diagnostics, want, "{line}");
    }
}

#[test]
fn the_name_and_enabled_of_the_old_format_are_unknown_keys() {
    // A launcher written for the old format names its rules and may
    // send `enabled`. Each such field is ignored with a warning, and the
    // rest of the line applies: `enabled=false` no longer drops a rule,
    // and a rule named like a built-in one is a rule of its own.
    let (table, diagnostics) = Table::parse(
        "name=my-game;exe=Game.exe;dpi_aware=false\nname=x;enabled=false;exe=*;gcmouse=false",
    );
    assert_eq!(
        table.rules,
        [
            Rule {
                exe: Exe::literal("Game.exe"),
                dpi_aware: Setting::Set(false),
                source: Source::Line(1),
                ..Rule::default()
            },
            Rule {
                exe: Exe::Any,
                gcmouse: Setting::Set(false),
                source: Source::Line(2),
                ..Rule::default()
            },
        ]
    );
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: ignoring unknown key \"name\"",
            "WINE_COMPATDB line 2: ignoring unknown key \"name\"",
            "WINE_COMPATDB line 2: ignoring unknown key \"enabled\"",
        ]
    );
}

#[test]
fn a_field_that_cannot_be_read_is_ignored_and_the_rest_applies() -> TestResult {
    // An unknown key, a value its key does not take and a field without
    // '=' each cost only that field, with one diagnostic naming the line
    // and the field. The record before and after it still applies.
    let want = Rule {
        exe: Exe::literal("x.exe"),
        d3d9: Setting::Set(D3d9::Wined3d),
        env: vec![EnvEntry::literal("A", "1")],
        source: Source::Line(1),
        ..Rule::default()
    };
    for (field, said) in [
        ("bogus=1", "unknown key \"bogus\""),
        ("dxgi=vulkan", "unknown dxgi value \"vulkan\""),
        // The x86_64 Wine's D3DMetal is a value like any other unknown one.
        ("dxgi=gptk", "unknown dxgi value \"gptk\""),
        ("d3d9=gptk", "unknown d3d9 value \"gptk\""),
        ("wined3d=opengl", "unknown wined3d value \"opengl\""),
        ("dpi_aware=yes", "unknown dpi_aware value \"yes\""),
        ("bogus", "field \"bogus\" without '='"),
        // A list entry or a fingerprint value its type does not take.
        ("arguments=", "an empty argument"),
        ("arguments_override=%00", "a NUL in its value"),
        ("company=a%00", "a NUL in its value"),
        (
            "dll_overrides=mscoree",
            "DLL override \"mscoree\" without '=' before its load order",
        ),
        ("env==1", "invalid variable name \"\""),
        ("env=A%3DB=1", "invalid variable name \"A=B\""),
    ] {
        let text = format!("{field};exe=x.exe;d3d9=wined3d;env=A=1\nexe=ok.exe");
        let (table, diagnostics) = Table::parse(&text);
        assert_eq!(table.rules.first(), Some(&want), "{field}");
        assert_eq!(
            table.rules.get(1).map(|r| r.exe.as_str()),
            Some("ok.exe"),
            "{field}"
        );
        let diagnostic = diagnostics.first().ok_or("no diagnostic")?;
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostic,
            &format!("WINE_COMPATDB line 1: ignoring {said}")
        );
    }
    Ok(())
}

#[test]
fn dpi_aware_round_trips_in_both_values_and_is_absent_when_unset() {
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::Any,
                dpi_aware: Setting::Set(true),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("old.exe"),
                d3d9: Setting::Set(D3d9::Wined3d),
                dpi_aware: Setting::Set(false),
                dll_overrides: vec![DllOverride::literal("a=")],
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("other.exe"),
                ..Rule::default()
            },
        ],
    };
    let text = table.to_env_value();
    assert_eq!(
        text,
        "exe=*;dpi_aware=true\n\
         exe=old.exe;d3d9=wined3d;dpi_aware=false;dll_overrides=a=\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = Table::parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
}

#[test]
fn a_bad_dpi_aware_value_is_ignored() {
    for bad in ["yes", "1", "truee", ""] {
        let (table, diagnostics) = Table::parse(&format!("exe=x.exe;dpi_aware={bad}\nexe=ok.exe"));
        assert_eq!(table.rules.len(), 2, "dpi_aware={bad:?}");
        assert_eq!(
            table.rules.first().map(|r| r.dpi_aware),
            Some(Setting::Unset)
        );
        assert_eq!(diagnostics.len(), 1, "dpi_aware={bad:?}");
    }
}

#[test]
fn large_address_aware_round_trips_and_is_absent_when_unset() {
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::Any,
                large_address_aware: Setting::Set(true),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("old.exe"),
                dpi_aware: Setting::Set(false),
                large_address_aware: Setting::Set(false),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("other.exe"),
                ..Rule::default()
            },
        ],
    };
    let text = table.to_env_value();
    assert_eq!(
        text,
        "exe=*;large_address_aware=true\n\
         exe=old.exe;dpi_aware=false;large_address_aware=false\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = Table::parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
}

#[test]
fn a_bad_large_address_aware_value_is_ignored() {
    let key = "large_address_aware";
    for bad in ["yes", "1", "fals", ""] {
        let (table, diagnostics) = Table::parse(&format!(
            "exe=x.exe;dpi_aware=false;{key}={bad}\nexe=ok.exe"
        ));
        assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
        let x = table.rules.first();
        assert_eq!(x.map(|r| r.large_address_aware), Some(Setting::Unset));
        assert_eq!(x.map(|r| r.dpi_aware), Some(Setting::Set(false)));
        assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
        assert!(
            diagnostics.first().is_some_and(|d| d.contains(key)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn gcmouse_round_trips_and_is_absent_when_unset() {
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::Any,
                gcmouse: Setting::Set(true),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("old.exe"),
                large_address_aware: Setting::Set(false),
                gcmouse: Setting::Set(false),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("other.exe"),
                ..Rule::default()
            },
        ],
    };
    let text = table.to_env_value();
    assert_eq!(
        text,
        "exe=*;gcmouse=true\n\
         exe=old.exe;large_address_aware=false;gcmouse=false\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = Table::parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
}

#[test]
fn a_bad_gcmouse_value_is_ignored() {
    let key = "gcmouse";
    for bad in ["yes", "1", "fals", ""] {
        let (table, diagnostics) = Table::parse(&format!(
            "exe=x.exe;dpi_aware=false;{key}={bad}\nexe=ok.exe"
        ));
        assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
        let x = table.rules.first();
        assert_eq!(x.map(|r| r.gcmouse), Some(Setting::Unset));
        assert_eq!(x.map(|r| r.dpi_aware), Some(Setting::Set(false)));
        assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
        assert!(
            diagnostics.first().is_some_and(|d| d.contains(key)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn force_nx_round_trips_and_is_absent_when_unset() {
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::Any,
                force_nx: Setting::Set(true),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("old.exe"),
                gcmouse: Setting::Set(false),
                force_nx: Setting::Set(false),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("other.exe"),
                ..Rule::default()
            },
        ],
    };
    let text = table.to_env_value();
    assert_eq!(
        text,
        "exe=*;force_nx=true\n\
         exe=old.exe;gcmouse=false;force_nx=false\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = Table::parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
}

#[test]
fn a_bad_force_nx_value_is_ignored() {
    let key = "force_nx";
    for bad in ["yes", "1", "fals", ""] {
        let (table, diagnostics) = Table::parse(&format!(
            "exe=x.exe;dpi_aware=false;{key}={bad}\nexe=ok.exe"
        ));
        assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
        let x = table.rules.first();
        assert_eq!(x.map(|r| r.force_nx), Some(Setting::Unset));
        assert_eq!(x.map(|r| r.dpi_aware), Some(Setting::Set(false)));
        assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
        assert!(
            diagnostics.first().is_some_and(|d| d.contains(key)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn a_reset_round_trips_through_the_env_value() {
    let table = Table {
        rules: vec![Rule {
            exe: Exe::literal("a.exe"),
            dxgi: Setting::Set(Dxgi::Wined3d),
            opengl: Setting::Reset,
            force_nx: Setting::Reset,
            dll_overrides: vec![DllOverride::literal("mscoree=default")],
            ..Rule::default()
        }],
    };
    let text = table.to_env_value();
    assert_eq!(
        text,
        "exe=a.exe;dxgi=wined3d;opengl=default;force_nx=default;dll_overrides=mscoree=default"
    );
    let (parsed, diagnostics) = Table::parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
    let r = table.resolve("a.exe", &VersionInfo::default());
    assert_eq!(r.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(r.scalar(Scalar::Dxgi).reset_by, None);
    assert_eq!(
        r.scalar(Scalar::OpenGl).reset_by,
        Some(Origin::BuiltInFor("a.exe".into()))
    );
}

#[test]
fn the_x86_64_wines_x87_sidecar_key_is_an_unknown_key() {
    // A launcher written for that Wine may still send it. Like any key
    // the library does not know, it costs only its own field, with a
    // warning, and the rest of the record still applies.
    let (table, diagnostics) =
        Table::parse("exe=old.exe;x87_sidecar=false;large_address_aware=false");
    assert_eq!(
        table.rules.first().map(|r| r.large_address_aware),
        Some(Setting::Set(false))
    );
    assert_eq!(
        diagnostics,
        ["WINE_COMPATDB line 1: ignoring unknown key \"x87_sidecar\"".to_string()]
    );
}

#[test]
fn every_field_a_record_can_carry_is_a_key_in_the_order_of_key_all() -> TestResult {
    // The literal names every field, so a new one does not compile until
    // it is added here, and with it its key.
    let rule = Rule {
        exe: Exe::literal("all.exe"),
        company: Some(Fingerprint::literal("c")),
        product: Some(Fingerprint::literal("p")),
        original_filename: Some(Fingerprint::literal("o")),
        dxgi: Setting::Set(Dxgi::Dxmt),
        d3d9: Setting::Set(D3d9::Mtld3d),
        wined3d: Setting::Set(Wined3d::Gl),
        opengl: Setting::Set(OpenGl::Egl),
        dpi_aware: Setting::Set(true),
        large_address_aware: Setting::Set(true),
        gcmouse: Setting::Set(true),
        force_nx: Setting::Set(true),
        dll_overrides: vec![DllOverride::literal("a=")],
        arguments: vec![Argument::literal("-a")],
        arguments_override: Some(vec![Argument::literal("-b")]),
        env: vec![EnvEntry::literal("A", "1")],
        source: Source::BuiltIn,
    };
    let record = rule_to_record(&rule);
    let mut keys = Vec::new();
    for field in record.split(';') {
        let (key, _) = field.split_once('=').ok_or("a field without '='")?;
        keys.push(key.parse::<Key>()?);
    }
    assert_eq!(keys, Key::ALL);
    // Every setting says what upstream Wine does without it; a matcher
    // is no setting.
    for key in &keys {
        assert_eq!(
            key.upstream().is_none(),
            key.kind() == KeyKind::Matcher,
            "{key}"
        );
        assert_eq!(key.to_string(), key.as_str());
    }
    // The scalar keys are the settings that are not lists, in order.
    let scalars: Vec<Scalar> = keys.iter().filter_map(|key| key.scalar()).collect();
    assert_eq!(scalars, Scalar::ALL);
    Ok(())
}

#[test]
fn every_word_of_the_format_is_read_in_any_case_and_written_in_lowercase() -> TestResult {
    // Keys, values, `true`/`false` and `default` alike.
    let (table, diagnostics) = Table::parse(
        "EXE=Game.EXE;Company=ACME;DXGI=WineD3D;D3D9=MtlD3D;Wined3d=VULKAN;OpenGL=CGL;\
         DPI_Aware=TRUE;Large_Address_Aware=False;GCMOUSE=Default;Force_NX=DEFAULT;\
         DLL_Overrides=D3D12=;Arguments=-X;Env=Name=Value\n\
         exe=x.exe;opengl=EGL",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let rule = table.rules.first().ok_or("no rule")?;
    assert_eq!(rule.dxgi, Setting::Set(Dxgi::Wined3d));
    assert_eq!(rule.d3d9, Setting::Set(D3d9::Mtld3d));
    assert_eq!(rule.wined3d, Setting::Set(Wined3d::Vulkan));
    assert_eq!(rule.opengl, Setting::Set(OpenGl::Cgl));
    assert_eq!(rule.dpi_aware, Setting::Set(true));
    assert_eq!(rule.large_address_aware, Setting::Set(false));
    assert_eq!(rule.gcmouse, Setting::Reset);
    assert_eq!(rule.force_nx, Setting::Reset);
    // The user data keeps its case.
    assert_eq!(
        table.to_env_value(),
        "exe=Game.EXE;company=ACME;dxgi=wined3d;d3d9=mtld3d;wined3d=vulkan;opengl=cgl;\
         dpi_aware=true;large_address_aware=false;gcmouse=default;force_nx=default;\
         dll_overrides=D3D12=;arguments=-X;env=Name=Value\n\
         exe=x.exe;opengl=egl"
    );
    // An unknown key or value is reported as it was written.
    let (_, diagnostics) = Table::parse("exe=x.exe;Bogus=1;DXGI=Vulkan");
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: ignoring unknown key \"Bogus\"",
            "WINE_COMPATDB line 1: ignoring unknown dxgi value \"Vulkan\"",
        ]
    );
    Ok(())
}

#[test]
fn arguments_override_round_trips_and_an_empty_one_clears() {
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::literal("a.exe"),
                arguments_override: Some(vec![Argument::literal("-x"), Argument::literal("-y z")]),
                arguments: vec![Argument::literal("-w")],
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("b.exe"),
                arguments_override: Some(Vec::new()),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("c.exe"),
                ..Rule::default()
            },
        ],
    };
    let text = table.to_env_value();
    assert_eq!(
        text,
        "exe=a.exe;arguments_override=-x;arguments_override=-y z;arguments=-w\n\
         exe=b.exe;arguments_override=\n\
         exe=c.exe"
    );
    let (parsed, diagnostics) = Table::parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
    // An empty value among others adds no entry.
    let (parsed, _) = Table::parse("exe=a.exe;arguments_override=;arguments_override=-x");
    assert_eq!(
        parsed.rules.first().and_then(Rule::arguments_override),
        Some(&[Argument::literal("-x")][..])
    );
}

#[test]
fn wined3d_parses_round_trips_and_rejects_other_values() {
    assert_eq!("GL".parse(), Ok(Wined3d::Gl));
    assert_eq!("Vulkan".parse(), Ok(Wined3d::Vulkan));
    assert!("opengl".parse::<Wined3d>().is_err());
    assert!("dxmt".parse::<Wined3d>().is_err());
    assert_eq!(Wined3d::Vulkan.as_str(), "vulkan");
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::literal("vk.exe"),
                wined3d: Setting::Set(Wined3d::Vulkan),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("gl.exe"),
                wined3d: Setting::Set(Wined3d::Gl),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("unset.exe"),
                ..Rule::default()
            },
        ],
    };
    let value = table.to_env_value();
    assert!(value.contains("exe=vk.exe;wined3d=vulkan"), "{value}");
    assert!(value.contains("exe=gl.exe;wined3d=gl"), "{value}");
    assert!(!value.contains("exe=unset.exe;wined3d"), "{value}");
    let (parsed, diagnostics) = Table::parse(&value);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
    let (bad, diagnostics) = Table::parse("exe=x.exe;wined3d=opengl\nexe=ok.exe");
    assert_eq!(bad.rules.len(), 2);
    assert_eq!(bad.rules.first().map(|r| r.wined3d), Some(Setting::Unset));
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn opengl_parses_round_trips_and_rejects_other_values() {
    assert_eq!("EGL".parse(), Ok(OpenGl::Egl));
    assert_eq!("Cgl".parse(), Ok(OpenGl::Cgl));
    assert!("gl".parse::<OpenGl>().is_err());
    assert!("".parse::<OpenGl>().is_err());
    assert_eq!(OpenGl::Egl.as_str(), "egl");
    assert_eq!(OpenGl::Egl.hook_value(), 1);
    assert_eq!(OpenGl::Cgl.hook_value(), 0);
    let table = Table {
        rules: vec![
            Rule {
                exe: Exe::Any,
                opengl: Setting::Set(OpenGl::Egl),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("apple.exe"),
                opengl: Setting::Set(OpenGl::Cgl),
                ..Rule::default()
            },
            Rule {
                exe: Exe::literal("unset.exe"),
                ..Rule::default()
            },
        ],
    };
    let value = table.to_env_value();
    assert_eq!(
        value,
        "exe=*;opengl=egl\n\
         exe=apple.exe;opengl=cgl\n\
         exe=unset.exe"
    );
    let (parsed, diagnostics) = Table::parse(&value);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, as_sent(&table));
    let (bad, diagnostics) = Table::parse("exe=x.exe;opengl=gl\nexe=ok.exe");
    assert_eq!(bad.rules.len(), 2);
    assert_eq!(bad.rules.first().map(|r| r.opengl), Some(Setting::Unset));
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics.first().is_some_and(|d| d.contains("opengl")),
        "{diagnostics:?}"
    );
}
